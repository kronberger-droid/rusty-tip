//! Multi-pass, drift compensation and the scan buffer, exercised through the
//! routine harness the way a routine would reach them. No hardware involved.

use std::path::PathBuf;
use std::time::Duration;

use rusty_tip::event::EventBus;
use rusty_tip::mock_controller::{FaultKind, MockController};
use rusty_tip::multi_pass::MultiPassConfig;
use rusty_tip::routine::Rt;
use rusty_tip::shutdown::ShutdownFlag;
use rusty_tip::spm_controller::{Capability, SpmController};
use rusty_tip::{ScanLineEnd, ScanLineMovement, SignalIndex};

fn temp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "rusty-tip-mpass-{name}-{}.mpas",
        std::process::id()
    ));
    p
}

#[test]
fn apply_writes_loads_and_activates_in_that_order() {
    let mut controller = MockController::builder().build();
    let obs = controller.observations();
    let path = temp("apply");
    let config = MultiPassConfig::constant_lift(SignalIndex(30), 210e-12);

    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    rt.multi_pass()
        .expect("the mock supports multi-pass")
        .apply(&config, &path, path.to_string_lossy().into_owned())
        .expect("apply succeeds");

    // The file the controller was pointed at is the one we wrote, and it still
    // says what we meant.
    let written = MultiPassConfig::read(&path).expect("written file reads back");
    assert_eq!(written, config);

    let obs = obs.lock();
    assert_eq!(obs.multi_pass_loaded, vec![path.to_string_lossy()]);
    assert_eq!(obs.multi_pass_active, Some(true));
    assert!(
        obs.first_index("multi_pass_load") < obs.first_index("multi_pass_activate"),
        "the configuration has to be loaded before multi-pass is switched on"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn activating_stops_a_running_scan() {
    // Nanonis stops an active scan when multi-pass is activated. Anything that
    // starts a scan first and activates afterwards is a bug, so the mock
    // reproduces the behaviour rather than hiding it.
    let mut controller = MockController::builder().build();
    let obs = controller.observations();

    controller
        .scan_action(rusty_tip::ScanAction::Start, rusty_tip::ScanDirection::Down)
        .unwrap();
    assert!(obs.lock().scan_running);

    controller.multi_pass_activate(true).unwrap();
    assert!(!obs.lock().scan_running);
}

#[test]
fn a_load_failure_leaves_multi_pass_off() {
    let mut controller = MockController::builder()
        .fail_on_call("multi_pass_load", 1, FaultKind::Protocol)
        .build();
    let obs = controller.observations();
    let path = temp("load-fault");

    let config = MultiPassConfig::constant_lift(SignalIndex(30), 210e-12);
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    assert!(
        rt.multi_pass()
            .expect("the mock supports multi-pass")
            .apply(&config, &path, path.to_string_lossy().into_owned())
            .is_err()
    );
    assert_eq!(
        obs.lock().multi_pass_active,
        None,
        "a failed load must not be followed by an activate"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn ensuring_a_channel_keeps_the_ones_already_recorded() {
    // A multi-pass run is worthless if the signal it plays back was never
    // acquired, but clobbering the operator's other channels to fix that would
    // be worse.
    let mut controller = MockController::builder().build();
    let obs = controller.observations();
    let before = obs.lock().scan_buffer.clone();
    assert!(before.channels.contains(&SignalIndex(30)));

    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    let mut scan = rt.scan().unwrap();
    scan.ensure_channels(&[SignalIndex(30)]).unwrap();
    assert!(
        !obs.lock().called("scan_buffer_set"),
        "a channel already recorded should not provoke a write"
    );

    let after_ensure = scan.ensure_channels(&[SignalIndex(14)]).unwrap();
    let after = obs.lock().scan_buffer.clone();
    // The returned buffer is the one now in effect, so a caller that wants to
    // report it does not pay a second round trip to read it back.
    assert_eq!(after, after_ensure);
    assert_eq!(
        after.channels,
        [before.channels, vec![SignalIndex(14)]].concat()
    );
    assert_eq!((after.pixels, after.lines), (before.pixels, before.lines));
}

#[test]
fn the_harness_refuses_a_controller_without_the_capability() {
    // The gate is `Rt::multi_pass`, so a routine never reaches an action it
    // cannot run and nothing is written before the check.
    let mut caps = MockController::builder().build().capabilities();
    caps.remove(&Capability::MultiPass);
    let mut controller = MockController::builder().capabilities(caps).build();
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);

    match rt.multi_pass() {
        Ok(_) => panic!("a controller without the capability must not hand out the handle"),
        Err(e) => assert!(matches!(e, rusty_tip::spm_error::SpmError::Unsupported(_))),
    }
}

#[test]
fn compensating_drift_re_arms_a_latched_axis_before_measuring() {
    // Saturation stops the axis for good, so measuring first would fit a drift
    // that nothing is correcting. The re-arm is an off/on cycle, which is the
    // only documented way back.
    let mut controller = MockController::builder().build();
    let obs = controller.observations();
    obs.lock().drift_comp.z_saturated = true;

    // The mock's Z is constant, so the drift never responds and the solve
    // refuses rather than inventing a velocity. That is the assertion: it
    // fails loudly instead of writing a confident wrong number.
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    let err = rt
        .drift()
        .expect("the mock supports drift compensation")
        .compensate(SignalIndex(30), Duration::from_millis(9), 3)
        .unwrap_err();
    assert!(
        format!("{err}").contains("not responding"),
        "expected a non-responding-channel error, got: {err}"
    );

    let obs = obs.lock();
    let rearm: Vec<bool> = obs
        .drift_comp_writes
        .iter()
        .take(2)
        .map(|c| c.enabled)
        .collect();
    assert_eq!(rearm, vec![false, true], "the re-arm has to be off then on");
    assert!(
        obs.first_index("drift_comp_set") < obs.first_index("read_signal"),
        "the axis has to be re-armed before the first measurement"
    );
    assert!(
        !obs.drift_comp.z_saturated,
        "the off/on cycle clears the latch"
    );
}

#[test]
fn measuring_drift_refuses_an_open_feedback_loop() {
    // With the loop open, Z sits where it was parked and the drift it would
    // have followed is invisible. Fitting that returns a confident zero.
    let mut controller = MockController::builder().build();
    controller.observations().lock().z_controller_on = false;
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    let err = rt
        .drift()
        .expect("the mock supports drift compensation")
        .measure_z(SignalIndex(30), Duration::from_millis(9), 3)
        .unwrap_err();
    assert!(
        format!("{err}").contains("Z controller is off"),
        "got: {err}"
    );
}

#[test]
fn a_line_wait_reports_direction_and_pass_separately() {
    // The two numbers are not the same scheme: a `[PassN]` section in a .mpas
    // file is one direction of a pass, while `pass` here counts passes and
    // `movement` carries the direction. A four-section constant-lift run makes
    // that concrete.
    let mut controller = MockController::builder().build();
    controller.observations().lock().scan_line_ends = [
        (ScanLineMovement::Forward, 0),
        (ScanLineMovement::Backward, 0),
        (ScanLineMovement::Forward, 1),
        (ScanLineMovement::Backward, 1),
    ]
    .into_iter()
    .map(|(movement, pass)| ScanLineEnd {
        timed_out: false,
        line: 0,
        movement,
        pass,
    })
    .collect();

    let mut seen = Vec::new();
    loop {
        let end = controller
            .scan_wait_end_of_line(Duration::from_millis(10))
            .unwrap();
        if end.timed_out {
            break;
        }
        seen.push((end.movement, end.pass));
    }

    assert_eq!(
        seen,
        vec![
            (ScanLineMovement::Forward, 0),
            (ScanLineMovement::Backward, 0),
            (ScanLineMovement::Forward, 1),
            (ScanLineMovement::Backward, 1),
        ]
    );
}
