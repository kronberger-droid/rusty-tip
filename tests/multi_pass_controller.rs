//! The controller side of multi-pass: writing a configuration, loading it, and
//! activating it, driven by the mock so no hardware is involved.

use std::path::PathBuf;

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
