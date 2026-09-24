//! The experiment log format: what a file contains, in what order, and that
//! each tool's declared schema is pinned.
//!
//! The schema snapshot under `tests/snapshots/` is the review point for a
//! log-format change. When a tool's events change on purpose, regenerate it:
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test --test experiment_log
//! ```
//!
//! and commit the diff alongside the change.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusty_tip::SignalIndex;
use rusty_tip::config::AppConfig;
use rusty_tip::event::{Event, EventBus, EventEmitter, FileLogger, Observer};
use rusty_tip::experiment_log::{ControllerFacts, RunHeader, ToolSchema};
use rusty_tip::mock_controller::{MockController, models};
use rusty_tip::routine::{Routine, Rt, run_routine};
use rusty_tip::shutdown::ShutdownFlag;
use rusty_tip::spm_error::SpmError;
use rusty_tip::tip_prep::{Outcome, TipPrepParams, log_schema, run_tip_prep};

const FREQ_SHIFT_INDEX: SignalIndex = SignalIndex(2);

#[derive(Clone, Default)]
struct Recorder {
    events: Arc<Mutex<Vec<Event>>>,
}

impl Observer for Recorder {
    fn on_event(&self, event: &Event) {
        self.events.lock().unwrap().push(event.clone());
    }
}

fn recording_bus() -> (EventBus, Arc<Mutex<Vec<Event>>>) {
    let recorder = Recorder::default();
    let events = Arc::clone(&recorder.events);
    let mut bus = EventBus::new();
    bus.add_observer(Box::new(recorder));
    (bus, events)
}

fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(name)
}

/// Compare a tool's declared schema against its committed snapshot.
fn assert_schema_snapshot(name: &str, schema: &ToolSchema) {
    let rendered = serde_json::to_string_pretty(schema).unwrap() + "\n";
    let path = snapshot_path(name);
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &rendered).unwrap();
        return;
    }
    let committed = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "no snapshot at {}: {e}. Run with UPDATE_SNAPSHOTS=1 to create it.",
            path.display()
        )
    });
    assert_eq!(
        committed, rendered,
        "the declared log schema for {} changed. If that is intended, rerun with \
         UPDATE_SNAPSHOTS=1 and commit the new snapshot with the change.",
        schema.tool
    );
}

#[test]
fn tip_prep_schema_matches_its_snapshot() {
    assert_schema_snapshot("tip_prep.schema.json", &log_schema());
}

#[test]
fn every_declared_kind_is_namespaced_and_unique() {
    let schema = log_schema();
    let mut seen = std::collections::HashSet::new();
    for kind in &schema.kinds {
        let (tool, name) = kind
            .kind
            .split_once('/')
            .unwrap_or_else(|| panic!("kind {:?} is not tool/name", kind.kind));
        assert!(!tool.is_empty() && !name.is_empty(), "{:?}", kind.kind);
        assert!(
            seen.insert(kind.kind.clone()),
            "duplicate kind {:?}",
            kind.kind
        );
        assert!(
            kind.schema.get("$schema").is_some() || kind.schema.get("type").is_some(),
            "{:?} has no JSON Schema body",
            kind.kind
        );
    }
}

/// Nested actions are logged as children with depth, and every action logs
/// its own fields as params, so a reader can rebuild what ran with what.
#[test]
fn composite_actions_log_their_steps_one_level_deeper_with_params() {
    struct OneApproach;
    impl Routine for OneApproach {
        fn name(&self) -> &str {
            "one_approach"
        }
        fn run(&mut self, rt: &mut Rt) -> Result<Outcome, SpmError> {
            rt.z()?
                .calibrated_approach_within(std::time::Duration::from_secs(7))?;
            Ok(Outcome::Completed)
        }
    }

    let mut mock = MockController::builder().build();
    let (bus, events) = recording_bus();
    run_routine(&mut mock, &bus, &ShutdownFlag::new(), &mut OneApproach).unwrap();

    let events = events.lock().unwrap();
    let started: Vec<(String, usize, serde_json::Value)> = events
        .iter()
        .filter_map(|e| match e {
            Event::ActionStarted {
                action,
                depth,
                params,
                ..
            } => Some((action.clone(), *depth, params.clone())),
            _ => None,
        })
        .collect();

    let (_, depth, params) = started
        .iter()
        .find(|(a, _, _)| a == "calibrated_approach")
        .expect("the composite itself is logged");
    assert_eq!(*depth, 0);
    assert_eq!(params["timeout_ms"], 7000, "params are the action's fields");

    let approaches: Vec<usize> = started
        .iter()
        .filter(|(a, _, _)| a == "auto_approach")
        .map(|(_, d, _)| *d)
        .collect();
    assert_eq!(
        approaches,
        vec![1, 1],
        "both approaches inside the composite are logged one level deeper"
    );
    assert!(
        started.iter().any(|(a, d, _)| a == "wait" && *d == 1),
        "the settles inside the composite are logged too"
    );
    assert!(
        started
            .iter()
            .any(|(a, d, _)| a == "center_freq_shift" && *d == 1),
        "the PLL centre inside the composite is logged"
    );
}

/// A run's events are bracketed: the harness ends every run with
/// `run_finished` carrying the outcome, whatever it was.
#[test]
fn a_run_ends_with_run_finished_carrying_the_outcome() {
    let mut cfg = AppConfig::default();
    cfg.tip_prep.max_cycles = Some(2);
    cfg.tip_prep.max_duration_secs = None;
    cfg.tip_prep.stability.check_stability = false;
    let t = &mut cfg.tip_prep.timing;
    t.pulse_width_ms = 0;
    t.post_approach_settle_ms = 0;
    t.post_reposition_settle_ms = 0;
    t.buffer_clear_wait_ms = 0;
    t.post_pulse_settle_ms = 0;
    cfg.data_acquisition.stable_signal_samples = 8;
    cfg.tip_prep.signal_stability.read_retry_count = 0;

    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(models::always(-40.0))
        .build();
    let (bus, events) = recording_bus();
    let outcome = run_tip_prep(
        Box::new(mock),
        TipPrepParams {
            events: &bus,
            shutdown: &ShutdownFlag::new(),
            config: &cfg,
            freq_shift: FREQ_SHIFT_INDEX,
        },
    )
    .unwrap();
    assert!(matches!(outcome, Outcome::CycleLimit(2)));

    let events = events.lock().unwrap();
    match events.last().expect("events were emitted") {
        Event::RunFinished {
            outcome, detail, ..
        } => {
            assert_eq!(outcome, "cycle_limit");
            assert_eq!(detail.as_deref(), Some("2"));
        }
        other => panic!("the last event must be run_finished, got {other:?}"),
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Custom { kind, .. } if kind == "tip_prep/cycle")),
        "cycles are logged under their declared kind"
    );
}

/// What lands in the file: one JSON object per line, a `seq` that counts
/// from zero, a `run_started` header first with everything a reader needs.
#[test]
fn the_file_has_numbered_lines_and_a_self_describing_header() {
    let dir = std::env::temp_dir().join(format!(
        "rusty-tip-log-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("run.jsonl");

    let mut mock = MockController::builder().build();
    let facts = ControllerFacts::gather(&mut mock, None);
    let cfg = AppConfig::default();
    {
        let mut bus = EventBus::new();
        bus.add_observer(Box::new(FileLogger::new(fs::File::create(&path).unwrap())));
        bus.emit(Event::run_started(RunHeader::new(
            log_schema(),
            &cfg,
            facts,
        )));
        bus.emit(Event::action_started(
            "set_bias",
            serde_json::json!({"voltage": -0.5}),
            0,
        ));
        bus.emit(Event::run_finished(
            "completed",
            None,
            std::time::Duration::from_secs(1),
        ));
        // Dropping the bus drops the logger, which flushes.
    }

    let text = fs::read_to_string(&path).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).expect("each line is one JSON object"))
        .collect();
    assert_eq!(lines.len(), 3);
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(line["seq"], i as u64, "seq counts from zero in file order");
        assert!(line["timestamp"].is_f64(), "every line is timestamped");
    }

    let header = &lines[0];
    assert_eq!(header["type"], "run_started");
    assert_eq!(header["header"]["tool"], "tip_prep");
    assert_eq!(header["header"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(header["header"]["envelope_version"], 1);
    assert_eq!(
        header["header"]["config"]["tip_prep"]["sharp_tip_bounds"],
        serde_json::json!([-2.0, 0.0]),
        "the config as loaded is in the header"
    );
    assert!(
        header["header"]["schema"]["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k["kind"] == "tip_prep/cycle"),
        "the header declares the tool's event kinds"
    );
    assert_eq!(lines[1]["type"], "action_started");
    assert_eq!(lines[1]["params"]["voltage"], -0.5);
    assert_eq!(lines[1]["depth"], 0);
    assert_eq!(lines[2]["type"], "run_finished");
    assert_eq!(lines[2]["outcome"], "completed");

    fs::remove_dir_all(&dir).unwrap();
}
