//! `rt-log` end to end: a log written by the real routine against the mock,
//! read back by the built binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusty_tip::SignalIndex;
use rusty_tip::config::AppConfig;
use rusty_tip::event::{Event, EventBus, EventEmitter, FileLogger};
use rusty_tip::experiment_log::{ControllerFacts, RunHeader};
use rusty_tip::mock_controller::{MockController, models};
use rusty_tip::shutdown::ShutdownFlag;
use rusty_tip::tip_prep::{Outcome, TipPrepParams, log_schema, run_tip_prep};

const FREQ_SHIFT_INDEX: SignalIndex = SignalIndex(2);

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rt-log-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A short blunt-tip run that hits its cycle limit, logged to `path`.
fn write_cycle_limit_log(path: &Path) {
    let mut cfg = AppConfig::default();
    cfg.tip_prep.max_cycles = Some(3);
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

    let mut mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(models::always(-40.0))
        .build();
    let facts = ControllerFacts::gather(&mut mock, None);

    let mut bus = EventBus::new();
    bus.add_observer(Box::new(FileLogger::new(fs::File::create(path).unwrap())));
    bus.emit(Event::run_started(RunHeader::new(
        log_schema(),
        &cfg,
        facts,
    )));
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
    assert!(matches!(outcome, Outcome::CycleLimit(3)));
}

fn rt_log(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_rt-log"))
        .args(args)
        .output()
        .expect("rt-log runs");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn summary_timeline_plot_and_export_read_a_real_log() {
    let dir = scratch_dir("cli");
    let log = dir.join("tip_prep_test.jsonl");
    write_cycle_limit_log(&log);
    let log_s = log.to_str().unwrap();

    let (ok, out, err) = rt_log(&["summary", log_s]);
    assert!(ok, "summary failed: {err}");
    assert!(out.contains("tip_prep 0."), "tool and version: {out}");
    assert!(out.contains("outcome cycle_limit (3)"), "{out}");
    assert!(
        out.contains("bias_pulse"),
        "top-level actions listed: {out}"
    );
    assert!(out.contains("stable_read"), "measurements listed: {out}");
    assert!(out.contains("tip_prep/cycle"), "custom kinds listed: {out}");

    let (ok, out, _) = rt_log(&["timeline", log_s, "--action", "calibrated_approach"]);
    assert!(ok);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].contains("calibrated_approach"), "{out}");
    assert!(
        lines[1].contains("  auto_approach"),
        "children are indented under their parent: {out}"
    );
    assert!(
        lines[0].contains("timeout_ms=600000"),
        "params are shown as key=value: {out}"
    );

    let (ok, out, _) = rt_log(&["plot", log_s]);
    assert!(ok);
    assert!(
        out.contains("tip_prep/cycle.freq_shift") && out.contains("stable_read.value"),
        "series come from the declared schema and the measurements: {out}"
    );
    let (ok, out, err) = rt_log(&["plot", log_s, "tip_prep/cycle.pulse_voltage"]);
    assert!(ok, "{err}");
    assert!(out.contains("3 points"), "{out}");
    let (ok, _, err) = rt_log(&["plot", log_s, "tip_prep/cycle.nope"]);
    assert!(!ok);
    assert!(
        err.contains("available:"),
        "an unknown series lists what exists: {err}"
    );

    let export_dir = dir.join("export");
    let (ok, _, err) = rt_log(&["export", log_s, "--out", export_dir.to_str().unwrap()]);
    assert!(ok, "{err}");
    let cycles = fs::read_to_string(export_dir.join("tip_prep_cycle.csv")).unwrap();
    assert_eq!(
        cycles.lines().next().unwrap(),
        "seq,time_s,cycle,elapsed_secs,freq_shift,is_sharp,pulse_voltage",
        "columns are the declared schema's scalar fields"
    );
    assert_eq!(cycles.lines().count(), 4, "header plus three cycles");
    let actions = fs::read_to_string(export_dir.join("actions.csv")).unwrap();
    assert!(
        actions
            .lines()
            .next()
            .unwrap()
            .starts_with("seq,time_s,depth,action,")
    );
    assert!(
        actions.contains(",1,auto_approach,"),
        "nested actions carry depth 1"
    );
    let reads = fs::read_to_string(export_dir.join("stable_read.csv")).unwrap();
    assert!(reads.lines().next().unwrap().contains("std_dev"));
    let run: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(export_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(run["outcome"], "cycle_limit");
    assert_eq!(run["config"]["tip_prep"]["max_cycles"], 3);

    let (ok, out, _) = rt_log(&["ls", dir.to_str().unwrap()]);
    assert!(ok);
    assert!(
        out.contains("tip_prep_test.jsonl") && out.contains("cycle_limit (3)"),
        "{out}"
    );

    fs::remove_dir_all(&dir).unwrap();
}
