//! The session against the mock: one connection, several runs, and what
//! survives between them.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use rusty_tip::config::AppConfig;
use rusty_tip::event::{Event, Observer};
use rusty_tip::experiment_log::{ToolSchema, reader::Log};
use rusty_tip::mock_controller::{FaultKind, MockController, MockObservations, models};
use rusty_tip::routine::{ExitPolicy, Outcome, Routine, Rt, run_routine};
use rusty_tip::session::{
    Backend, ConnState, Job, JobCx, PresetFiles, Session, SessionCmd, SessionUpdate, spawn_with,
};
use rusty_tip::signal_registry::{SignalIndex, SignalRegistry};
use rusty_tip::spm_error::SpmError;
use rusty_tip::tip_prep::TipPrep;
use rusty_tip::{ShutdownFlag, SignalRegistry as _Registry};

const FREQ_SHIFT: SignalIndex = SignalIndex(2);

fn fast_config() -> AppConfig {
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
    cfg
}

/// Tip prep as a job: what the workbench's tip-prep tool will build.
struct TipPrepJob {
    config: AppConfig,
}

impl Job for TipPrepJob {
    fn name(&self) -> &str {
        "tip_prep"
    }

    fn log_schema(&self) -> ToolSchema {
        rusty_tip::tip_prep::log_schema()
    }

    fn header_config(&self) -> serde_json::Value {
        serde_json::to_value(&self.config).unwrap()
    }

    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        let fs = cx
            .registry
            .get_by_name("freq shift")
            .ok_or_else(|| SpmError::Workflow("no freq shift signal".into()))?
            .signal_index();
        let mut routine = TipPrep::new(&self.config, fs);
        run_routine(cx.controller, cx.events, cx.shutdown, &mut routine)
    }
}

/// A routine-backed job that reconfigures the controller and leaves the
/// tip where it is: the shape of the drift and baseline tools.
struct LoadAndLeave {
    settings: PathBuf,
}

impl Routine for LoadAndLeave {
    fn name(&self) -> &str {
        "load_and_leave"
    }

    fn run(&mut self, rt: &mut Rt) -> Result<Outcome, SpmError> {
        rt.presets()?.load_settings(&self.settings)?;
        Ok(Outcome::Completed)
    }

    fn exit_policy(&self) -> ExitPolicy {
        ExitPolicy::LeaveInPlace
    }
}

impl Job for LoadAndLeave {
    fn name(&self) -> &str {
        "load_and_leave"
    }

    fn log_schema(&self) -> ToolSchema {
        ToolSchema::new("load_and_leave").including(rusty_tip::routine::log_schema())
    }

    fn header_config(&self) -> serde_json::Value {
        serde_json::json!({ "settings": self.settings.display().to_string() })
    }

    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        let mut routine = LoadAndLeave {
            settings: self.settings.clone(),
        };
        run_routine(cx.controller, cx.events, cx.shutdown, &mut routine)
    }
}

/// A job that is not a routine at all, so the session has to close its log.
struct Bare(Result<Outcome, SpmError>);

impl Job for Bare {
    fn name(&self) -> &str {
        "bare"
    }
    fn log_schema(&self) -> ToolSchema {
        ToolSchema::new("bare")
    }
    fn header_config(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        cx.controller.set_bias(-0.3)?;
        std::mem::replace(&mut self.0, Ok(Outcome::Completed))
    }
}

struct Panicking;

impl Job for Panicking {
    fn name(&self) -> &str {
        "panicking"
    }
    fn log_schema(&self) -> ToolSchema {
        ToolSchema::new("panicking")
    }
    fn header_config(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn run(&mut self, _cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        panic!("job blew up");
    }
}

#[derive(Clone, Default)]
struct Recorder(Arc<StdMutex<Vec<Event>>>);

impl Observer for Recorder {
    fn on_event(&self, event: &Event) {
        self.0.lock().unwrap().push(event.clone());
    }
}

fn mock() -> (MockController, Arc<parking_lot::Mutex<MockObservations>>) {
    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT)
        .freq_shift(models::always(-1.0))
        .build();
    let obs = mock.observations();
    (mock, obs)
}

fn registry(mock: &mut MockController) -> SignalRegistry {
    _Registry::from_controller(mock).unwrap()
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusty-tip-session-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn two_jobs_run_back_to_back_on_one_connection() {
    let (mut mock, obs) = mock();
    let reg = registry(&mut mock);
    let mut session = Session::new(None);
    session
        .connect_with(Box::new(mock), reg, PresetFiles::default())
        .unwrap();
    assert_eq!(session.state(), ConnState::Connected);

    let shutdown = ShutdownFlag::new();
    let mut job = TipPrepJob {
        config: fast_config(),
    };
    assert!(matches!(
        session.run(&mut job, &shutdown, Vec::new()).unwrap(),
        Outcome::Completed
    ));
    assert!(matches!(
        session.run(&mut job, &shutdown, Vec::new()).unwrap(),
        Outcome::Completed
    ));

    let obs = obs.lock();
    assert_eq!(obs.count("prepare"), 2, "prepare brackets each run");
    assert_eq!(obs.count("teardown"), 2, "and so does teardown");
    assert_eq!(obs.count("signal_names"), 1, "the registry is built once");
    assert!(!obs.disconnected, "the connection outlives both runs");
    assert_eq!(session.state(), ConnState::Connected);
}

#[test]
fn safe_tip_is_restored_after_every_run() {
    let (mut mock, obs) = mock();
    obs.lock().safe_tip_enabled = true;
    let reg = registry(&mut mock);
    let mut session = Session::new(None);
    session
        .connect_with(Box::new(mock), reg, PresetFiles::default())
        .unwrap();

    let shutdown = ShutdownFlag::new();
    let mut job = TipPrepJob {
        config: fast_config(),
    };
    for _ in 0..2 {
        session.run(&mut job, &shutdown, Vec::new()).unwrap();
        assert!(obs.lock().safe_tip_enabled, "restored to on after the run");
    }
    // The approach toggles safe-tip on its own, so no call count is stable;
    // what must hold is that each run's restore is its last safe-tip write.
    let obs = obs.lock();
    let teardowns: Vec<usize> = obs
        .calls
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == "teardown")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(teardowns.len(), 2);
    for end in teardowns {
        let last_toggle = obs.calls[..end]
            .iter()
            .rposition(|c| *c == "safe_tip_set_enabled")
            .unwrap();
        let last_configure = obs.calls[..end]
            .iter()
            .rposition(|c| *c == "safe_tip_configure")
            .unwrap();
        assert!(
            last_configure < last_toggle,
            "the restore configures, then re-enables, just before teardown: {:?}",
            &obs.calls[last_configure..=end]
        );
    }
}

#[test]
fn a_leave_in_place_job_never_withdraws_and_its_settings_load_is_recorded() {
    let (mut mock, obs) = mock();
    let reg = registry(&mut mock);
    let dir = temp_dir("leave");
    let mut session = Session::new(Some(dir.clone()));
    session
        .connect_with(Box::new(mock), reg, PresetFiles::default())
        .unwrap();
    assert!(session.settings_load().is_none());

    let recorder = Recorder::default();
    let mut job = LoadAndLeave {
        settings: PathBuf::from("settings/drift.ini"),
    };
    session
        .run(
            &mut job,
            &ShutdownFlag::new(),
            vec![Box::new(recorder.clone())],
        )
        .unwrap();

    let obs = obs.lock();
    assert_eq!(obs.withdraw_count, 0, "LeaveInPlace issues no withdraw");
    assert_eq!(obs.motor_moves, 0);
    assert_eq!(obs.settings_loaded, vec!["settings/drift.ini".to_string()]);

    let load = session.settings_load().expect("the session saw the load");
    assert_eq!(load.path, Path::new("settings/drift.ini"));
    assert_eq!(load.by, "load_and_leave");

    let events = recorder.0.lock().unwrap();
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::Custom { kind, data, .. }
                if kind == "routine/settings_loaded" && data["path"] == "settings/drift.ini"
        )),
        "the typed event reaches observers"
    );

    // The log on disk starts with the header and ends with the outcome.
    let logs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(logs.len(), 1);
    assert!(
        logs[0]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("load_and_leave_")
    );
    let log = Log::read(&logs[0]).unwrap();
    assert_eq!(log.header().unwrap().tool, "load_and_leave");
    assert_eq!(log.finished().unwrap().0, "completed");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_session_closes_the_log_of_a_job_that_is_not_a_routine() {
    let (mut mock, _) = mock();
    let reg = registry(&mut mock);
    let mut session = Session::new(None);
    session
        .connect_with(Box::new(mock), reg, PresetFiles::default())
        .unwrap();

    let recorder = Recorder::default();
    let mut job = Bare(Err(SpmError::Workflow("nope".into())));
    let err = session
        .run(
            &mut job,
            &ShutdownFlag::new(),
            vec![Box::new(recorder.clone())],
        )
        .unwrap_err();
    assert_eq!(err.to_string(), "nope");

    let events = recorder.0.lock().unwrap();
    let finished: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::RunFinished {
                outcome, detail, ..
            } => Some((outcome.clone(), detail.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        finished,
        vec![("error".to_string(), Some("nope".to_string()))],
        "exactly one run_finished, written by the session"
    );
    assert!(matches!(events[0], Event::RunStarted { .. }));
}

#[test]
fn a_routine_job_gets_exactly_one_run_finished() {
    let (mut mock, _) = mock();
    let reg = registry(&mut mock);
    let mut session = Session::new(None);
    session
        .connect_with(Box::new(mock), reg, PresetFiles::default())
        .unwrap();

    let recorder = Recorder::default();
    let mut job = TipPrepJob {
        config: fast_config(),
    };
    session
        .run(
            &mut job,
            &ShutdownFlag::new(),
            vec![Box::new(recorder.clone())],
        )
        .unwrap();

    let events = recorder.0.lock().unwrap();
    let finished = events
        .iter()
        .filter(|e| matches!(e, Event::RunFinished { .. }))
        .count();
    assert_eq!(finished, 1, "the harness wrote it; the session must not");
}

#[test]
fn a_job_error_or_panic_leaves_the_session_usable() {
    let (mut mock, obs) = mock();
    let reg = registry(&mut mock);
    let mut session = Session::new(None);
    session
        .connect_with(Box::new(mock), reg, PresetFiles::default())
        .unwrap();
    let shutdown = ShutdownFlag::new();

    let err = session
        .run(&mut Panicking, &shutdown, Vec::new())
        .unwrap_err();
    assert!(err.to_string().contains("job blew up"), "{err}");
    assert_eq!(session.state(), ConnState::Connected);

    let mut failing = Bare(Err(SpmError::Protocol("bad".into())));
    session
        .run(&mut failing, &shutdown, Vec::new())
        .unwrap_err();
    assert_eq!(session.state(), ConnState::Connected);

    let mut job = TipPrepJob {
        config: fast_config(),
    };
    session.run(&mut job, &shutdown, Vec::new()).unwrap();
    assert!(obs.lock().torn_down);
}

#[test]
fn a_connection_error_poisons_the_session_until_reconnect() {
    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT)
        .freq_shift(models::always(-1.0))
        .fail_on_call("set_bias", 1, FaultKind::Disconnect)
        .build();
    let mut mock = mock;
    let reg = registry(&mut mock);
    let mut session = Session::new(None);
    session
        .connect_with(Box::new(mock), reg, PresetFiles::default())
        .unwrap();
    let shutdown = ShutdownFlag::new();

    session
        .run(&mut Bare(Ok(Outcome::Completed)), &shutdown, Vec::new())
        .unwrap_err();
    assert_eq!(session.state(), ConnState::Poisoned);
    assert!(
        session
            .run(&mut Bare(Ok(Outcome::Completed)), &shutdown, Vec::new())
            .is_err(),
        "nothing runs on a poisoned connection"
    );

    session.reconnect().unwrap();
    assert_eq!(session.state(), ConnState::Connected);
    session
        .run(&mut Bare(Ok(Outcome::Completed)), &shutdown, Vec::new())
        .unwrap();
}

#[test]
fn connect_loads_the_preset_files_and_disconnect_stops_the_stream_once() {
    let (mut mock, obs) = mock();
    let reg = registry(&mut mock);
    let mut session = Session::new(None);
    session
        .connect_with(
            Box::new(mock),
            reg,
            PresetFiles {
                layout: Some(PathBuf::from("a.lyt")),
                settings: Some(PathBuf::from("b.ini")),
            },
        )
        .unwrap();
    {
        let obs = obs.lock();
        assert_eq!(obs.layouts_loaded, vec!["a.lyt".to_string()]);
        assert_eq!(obs.settings_loaded, vec!["b.ini".to_string()]);
        let layout = obs.first_index("load_layout").unwrap();
        let settings = obs.first_index("load_settings").unwrap();
        assert!(layout < settings, "layout first, then settings");
    }
    assert_eq!(session.settings_load().unwrap().by, "connect");
    assert_eq!(session.layout_load().unwrap().by, "connect");

    session.reload_presets().unwrap();
    assert_eq!(obs.lock().settings_loaded.len(), 2);
    assert_eq!(session.settings_load().unwrap().by, "reload");

    session.disconnect();
    session.disconnect();
    assert_eq!(session.state(), ConnState::Disconnected);
    let obs = obs.lock();
    assert!(obs.disconnected);
    assert_eq!(obs.count("disconnect"), 1, "the mock is dropped after one");
}

#[test]
fn readouts_come_from_the_registry_names() {
    let (mut mock, _) = mock();
    let reg = registry(&mut mock);
    let mut session = Session::new(None);
    session
        .connect_with(Box::new(mock), reg, PresetFiles::default())
        .unwrap();

    let readouts = session.read_readouts().unwrap();
    let names: Vec<&str> = readouts.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Bias (V)", "Z (m)", "Current (A)", "freq shift"]
    );
    let fs = readouts.iter().find(|r| r.name == "freq shift").unwrap();
    assert_eq!(fs.index, FREQ_SHIFT);
    assert_eq!(fs.value, -1.0);
}

#[test]
fn the_mock_backend_connects_on_its_own() {
    let mut session = Session::new(None);
    session.connect(&Backend::Mock).unwrap();
    assert_eq!(session.state(), ConnState::Connected);
    let facts = session.facts().unwrap();
    assert!(facts.signals.iter().any(|s| s.name == "freq shift"));
    assert!(
        session
            .capabilities()
            .contains(&rusty_tip::spm_controller::Capability::Presets)
    );
}

#[test]
fn the_session_thread_runs_a_job_and_reports_it() {
    let (mut mock, _) = mock();
    let reg = registry(&mut mock);
    let mut session = Session::new(None);
    session
        .connect_with(Box::new(mock), reg, PresetFiles::default())
        .unwrap();
    let handle = spawn_with(session);

    let (tx, rx) = crossbeam_channel::unbounded();
    let shutdown = ShutdownFlag::new();
    handle
        .send(SessionCmd::Run {
            job: Box::new(TipPrepJob {
                config: fast_config(),
            }),
            shutdown: shutdown.clone(),
            events: tx,
        })
        .unwrap();

    let mut states = Vec::new();
    let mut finished = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while finished.is_none() && std::time::Instant::now() < deadline {
        match handle.updates().recv_timeout(Duration::from_millis(100)) {
            Ok(SessionUpdate::State(s)) => states.push(s),
            Ok(SessionUpdate::JobFinished(r)) => finished = Some(r),
            Ok(_) => {}
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(e) => panic!("session thread went away: {e}"),
        }
    }
    assert!(
        matches!(finished, Some(Ok(Outcome::Completed))),
        "{finished:?}"
    );
    assert!(states.contains(&ConnState::Running));
    assert!(
        rx.try_iter().any(|e| matches!(e, Event::RunStarted { .. })),
        "events were forwarded to the caller's channel"
    );
    handle.join();
}

/// The handoff's step-2 acceptance, through the same commands the window
/// sends: connect once, run tip prep twice, stop one mid-run, disconnect.
/// The second run must not reconnect or rebuild the registry.
#[test]
fn connect_once_run_twice_stop_one_disconnect() {
    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT)
        .freq_shift(models::always(-40.0)) // blunt: the loop keeps pulsing
        .build();
    let obs = mock.observations();
    let mut mock = mock;
    let reg = registry(&mut mock);
    let mut session = Session::new(None);
    session
        .connect_with(Box::new(mock), reg, PresetFiles::default())
        .unwrap();
    let handle = spawn_with(session);

    let wait_for_finish = |handle: &rusty_tip::session::SessionHandle| {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match handle.updates().recv_timeout(Duration::from_millis(100)) {
                Ok(SessionUpdate::JobFinished(r)) => return r,
                Ok(_) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(e) => panic!("session thread went away: {e}"),
            }
        }
        panic!("the job did not finish in time");
    };

    // Run 1: a long run, stopped from the outside mid-cycle.
    let mut slow = fast_config();
    slow.tip_prep.max_cycles = Some(10_000);
    slow.tip_prep.timing.post_pulse_settle_ms = 100;
    let stop = ShutdownFlag::new();
    let (tx, _rx) = crossbeam_channel::unbounded();
    handle
        .send(SessionCmd::Run {
            job: Box::new(TipPrepJob { config: slow }),
            shutdown: stop.clone(),
            events: tx,
        })
        .unwrap();
    // Stop once the loop is pulsing, so the stop lands mid-run rather than
    // during the initial approach.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while obs.lock().pulses.is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the first run never fired a pulse"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    stop.request();
    let first = wait_for_finish(&handle);
    assert!(matches!(first, Ok(Outcome::StoppedByUser)), "{first:?}");
    let pulses_after_first = obs.lock().pulses.len();
    assert!(pulses_after_first >= 1, "the first run got going");

    // Run 2: to its cycle limit, on the same connection.
    let mut short = fast_config();
    short.tip_prep.max_cycles = Some(2);
    let (tx, _rx) = crossbeam_channel::unbounded();
    handle
        .send(SessionCmd::Run {
            job: Box::new(TipPrepJob { config: short }),
            shutdown: ShutdownFlag::new(),
            events: tx,
        })
        .unwrap();
    let second = wait_for_finish(&handle);
    assert!(matches!(second, Ok(Outcome::CycleLimit(2))), "{second:?}");

    handle.send(SessionCmd::Disconnect).unwrap();
    handle.join();

    let obs = obs.lock();
    assert_eq!(obs.pulses.len(), pulses_after_first + 2);
    assert_eq!(obs.count("prepare"), 2);
    assert_eq!(obs.count("teardown"), 2);
    assert_eq!(
        obs.count("signal_names"),
        1,
        "no reconnect between the runs"
    );
    assert_eq!(obs.count("reconnect"), 0);
    assert_eq!(obs.count("disconnect"), 1, "disconnected once, at the end");
}
