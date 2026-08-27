//! The controller side of multi-pass: writing a configuration, loading it, and
//! activating it, driven by the mock so no hardware is involved.

use std::path::PathBuf;
use std::time::Duration;

use rusty_tip::SignalIndex;
use rusty_tip::mock_controller::{FaultKind, MockController};
use rusty_tip::multi_pass::{self, MultiPassConfig};
use rusty_tip::spm_controller::{Capability, SpmController};

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

    multi_pass::apply(&mut controller, &config, &path, &path.to_string_lossy())
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
    assert!(multi_pass::apply(&mut controller, &config, &path, &path.to_string_lossy()).is_err());
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

    controller.scan_buffer_ensure(&[SignalIndex(30)]).unwrap();
    assert!(
        !obs.lock().called("scan_buffer_set"),
        "a channel already recorded should not provoke a write"
    );

    controller.scan_buffer_ensure(&[SignalIndex(14)]).unwrap();
    let after = obs.lock().scan_buffer.clone();
    assert_eq!(
        after.channels,
        [before.channels, vec![SignalIndex(14)]].concat()
    );
    assert_eq!((after.pixels, after.lines), (before.pixels, before.lines));
}

#[test]
fn apply_refuses_a_controller_without_the_capability() {
    // The capability now gates something, rather than only being declared.
    let mut caps = MockController::builder().build().capabilities();
    caps.remove(&Capability::MultiPass);
    let mut controller = MockController::builder().capabilities(caps).build();
    let path = temp("no-capability");

    let err = multi_pass::apply(
        &mut controller,
        &MultiPassConfig::constant_lift(SignalIndex(30), 210e-12),
        &path,
        &path.to_string_lossy(),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        rusty_tip::spm_error::SpmError::Unsupported(_)
    ));
    assert!(!path.exists(), "nothing should be written before the check");
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
    let err = controller
        .compensate_drift(SignalIndex(30), Duration::from_millis(9), 3)
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
    let err = controller
        .measure_z_drift(SignalIndex(30), Duration::from_millis(9), 3)
        .unwrap_err();
    assert!(
        format!("{err}").contains("Z controller is off"),
        "got: {err}"
    );
}
