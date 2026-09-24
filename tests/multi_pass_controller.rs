//! Multi-pass, drift compensation and the scan buffer, exercised through the
//! routine harness the way a routine would reach them. No hardware involved.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusty_tip::action::drift::CompensateDrift;
use rusty_tip::event::{Event, EventBus, Observer};
use rusty_tip::mock_controller::{FaultKind, MockController};
use rusty_tip::multi_pass::MultiPassConfig;
use rusty_tip::routine::Rt;
use rusty_tip::shutdown::ShutdownFlag;
use rusty_tip::spm_controller::{Capability, SpmController};
use rusty_tip::{ScanAction, ScanDirection, ScanLineEnd, ScanLineMovement, SignalIndex};

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

/// Every action start, as `(name, depth)`.
#[derive(Clone, Default)]
struct Starts(Arc<Mutex<Vec<(String, usize)>>>);

impl Observer for Starts {
    fn on_event(&self, event: &Event) {
        if let Event::ActionStarted { action, depth, .. } = event {
            self.0.lock().unwrap().push((action.clone(), *depth));
        }
    }
}

#[test]
fn apply_logs_the_load_and_the_switch_on_as_its_own_steps() {
    let mut controller = MockController::builder().build();
    let path = temp("apply-log");
    let config = MultiPassConfig::constant_lift(SignalIndex(30), 210e-12);
    let starts = Starts::default();
    let mut events = EventBus::new();
    events.add_observer(Box::new(starts.clone()));
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    rt.multi_pass()
        .unwrap()
        .apply(&config, &path, path.to_string_lossy().into_owned())
        .unwrap();

    let starts = starts.0.lock().unwrap();
    for step in ["load_multi_pass", "activate_multi_pass"] {
        assert!(
            starts.iter().any(|(a, d)| a == step && *d == 1),
            "{step} should be logged one level under apply: {starts:?}"
        );
    }
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
    let mut controller = MockController::builder().z_drift(Z, 2e-12, -1.0).build();
    let obs = controller.observations();
    obs.lock().drift_comp.z_saturated = true;

    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    rt.drift()
        .expect("the mock supports drift compensation")
        .compensate(&CompensateDrift::new(Z))
        .expect("a drifting, responsive Z compensates");

    let obs = obs.lock();
    let rearm: Vec<bool> = obs
        .drift_comp_writes
        .iter()
        .take(2)
        .map(|c| c.enabled)
        .collect();
    assert_eq!(rearm, vec![false, true], "the re-arm has to be off then on");
    assert!(
        obs.first_index("drift_comp_set") < obs.first_index("read_signal_samples"),
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
        .measure_z(Z, Duration::from_millis(9))
        .unwrap_err();
    assert!(format!("{err}").contains("not On"), "got: {err}");
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

#[test]
fn measuring_drift_refuses_while_a_scan_is_running() {
    // A scan moves Z over topography, which fits as a drift of nanometres per
    // second. Silently compensating for that would drive Z somewhere on the
    // strength of a step edge.
    let mut controller = MockController::builder().build();
    controller
        .scan_action(ScanAction::Start, ScanDirection::Up)
        .unwrap();

    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    let err = rt
        .drift()
        .expect("the mock supports drift compensation")
        .measure_z(Z, Duration::from_millis(9))
        .unwrap_err();

    assert!(format!("{err}").contains("a scan is running"), "got: {err}");
}

/// The Z position's RT slot, as the drift tests use it.
const Z: SignalIndex = SignalIndex(30);

/// Collects the role of every `drift/burst` event, in order.
#[derive(Default, Clone)]
struct BurstRoles(Arc<Mutex<Vec<String>>>);

impl Observer for BurstRoles {
    fn on_event(&self, event: &Event) {
        if let Event::Custom { kind, data, .. } = event
            && kind == "drift/burst"
        {
            let role = data["role"].as_str().unwrap_or("?").to_string();
            self.0.lock().unwrap().push(role);
        }
    }
}

#[test]
fn a_burst_measures_the_drift_and_says_what_it_is_worth() {
    // A constant Z used to be an error, "no drift rate can be resolved". It is
    // a measurement like any other: zero, with an error bar, and the caller
    // decides what to make of it.
    for (drift, negligible) in [(3e-12, false), (0.0, true)] {
        let mut controller = MockController::builder().z_drift(Z, drift, -1.0).build();
        let events = EventBus::new();
        let shutdown = ShutdownFlag::new();
        let mut rt = Rt::new(&mut controller, &events, &shutdown);
        let estimate = rt
            .drift()
            .expect("the mock supports drift compensation")
            .measure_z(Z, Duration::from_secs(5))
            .expect("a streamed Z measures");

        assert!((estimate.rate_m_s - drift).abs() < 1e-15, "{estimate:?}");
        assert_eq!(estimate.samples, 5000, "five seconds of a 1 kHz stream");
        assert_eq!(estimate.is_negligible(0.05e-12), negligible);
    }
}

#[test]
fn compensation_converges_whichever_way_the_velocity_sign_runs() {
    for response in [1.0, -1.0] {
        let drift = 4e-12;
        let mut controller = MockController::builder()
            .z_drift(Z, drift, response)
            .build();
        let obs = controller.observations();
        let roles = BurstRoles::default();
        let mut events = EventBus::new();
        events.add_observer(Box::new(roles.clone()));
        let shutdown = ShutdownFlag::new();
        let mut rt = Rt::new(&mut controller, &events, &shutdown);
        let result = rt
            .drift()
            .expect("the mock supports drift compensation")
            .compensate(&CompensateDrift::new(Z))
            .expect("compensates");

        assert!(result.converged, "response {response}: {result:?}");
        let learned = result.response.expect("the trial learned a response");
        assert!((learned - response).abs() < 1e-6, "learned {learned}");
        // What is left on the machine cancels the drift, in its convention.
        let left = drift + response * obs.lock().drift_comp.vz;
        assert!(
            left.abs() < 0.05e-12,
            "response {response}: {left} m/s left"
        );
        assert!(obs.lock().drift_comp.enabled);
        // Baseline, a trial to learn the sign, and the rest of the budget in
        // corrections, spent in full even though the first one lands.
        assert_eq!(
            *roles.0.lock().unwrap(),
            [
                "baseline",
                "trial",
                "correction",
                "correction",
                "correction"
            ]
        );
        assert_eq!(result.bursts, 5);
    }
}

/// Asks for a stop as soon as the baseline burst is reported, so the request
/// lands inside the trial burst.
struct StopAfterBaseline(ShutdownFlag);

impl Observer for StopAfterBaseline {
    fn on_event(&self, event: &Event) {
        if let Event::Custom { kind, data, .. } = event
            && kind == "drift/burst"
            && data["role"] == "baseline"
        {
            self.0.request();
        }
    }
}

#[test]
fn a_stop_during_the_trial_puts_the_previous_velocity_back() {
    let mut controller = MockController::builder().z_drift(Z, 4e-12, 1.0).build();
    let obs = controller.observations();
    let vz_before = obs.lock().drift_comp.vz;
    let shutdown = ShutdownFlag::new();
    let mut events = EventBus::new();
    events.add_observer(Box::new(StopAfterBaseline(shutdown.clone())));
    let mut rt = Rt::new(&mut controller, &events, &shutdown);

    let err = rt
        .drift()
        .expect("the mock supports drift compensation")
        .compensate(&CompensateDrift::new(Z))
        .expect_err("the stop must end the compensation");

    assert!(
        matches!(err, rusty_tip::spm_error::SpmError::ShutdownRequested),
        "{err:?}"
    );
    assert_eq!(
        obs.lock().drift_comp.vz,
        vz_before,
        "the 20 pm/s trial velocity must not outlive the stop"
    );
}

#[test]
fn compensation_converges_through_measurement_noise() {
    // 20 pm of scatter per sample, five times the drift over a whole burst.
    let (drift, response) = (1e-12, -1.0);
    let mut controller = MockController::builder()
        .z_drift(Z, drift, response)
        .z_drift_noise(20e-12)
        .build();
    let obs = controller.observations();
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    let result = rt
        .drift()
        .expect("the mock supports drift compensation")
        .compensate(&CompensateDrift::new(Z))
        .expect("compensates");

    assert!(result.converged, "{result:?}");
    assert!(result.bursts <= 5);
    let left = drift + response * obs.lock().drift_comp.vz;
    assert!(left.abs() < 0.2e-12, "{left} m/s left, {result:?}");
}

#[test]
fn a_known_response_skips_the_trial() {
    let mut controller = MockController::builder().z_drift(Z, 4e-12, -1.0).build();
    let obs = controller.observations();
    let roles = BurstRoles::default();
    let mut events = EventBus::new();
    events.add_observer(Box::new(roles.clone()));
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    let result = rt
        .drift()
        .expect("the mock supports drift compensation")
        .compensate(&CompensateDrift {
            response: Some(-1.0),
            ..CompensateDrift::new(Z)
        })
        .expect("compensates");

    assert!(result.converged, "{result:?}");
    assert!(!roles.0.lock().unwrap().contains(&"trial".to_string()));
    // Corrections only, so the trial velocity never reached the piezo.
    let trial = CompensateDrift::new(Z).trial_vz;
    assert!(
        obs.lock()
            .drift_comp_writes
            .iter()
            .all(|c| c.vz.abs() < trial / 2.0)
    );
}

#[test]
fn a_negligible_drift_changes_nothing() {
    let mut controller = MockController::builder().z_drift(Z, 0.0, -1.0).build();
    let obs = controller.observations();
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    let result = rt
        .drift()
        .expect("the mock supports drift compensation")
        .compensate(&CompensateDrift::new(Z))
        .expect("nothing to do is not an error");

    assert!(result.converged);
    assert_eq!(result.response, None, "no trial, so nothing was learned");
    assert_eq!(result.bursts, 1);
    assert!(obs.lock().drift_comp_writes.is_empty());
}

#[test]
fn a_channel_that_does_not_respond_is_refused_and_the_velocity_put_back() {
    // Saturated, disabled, or not wired to the piezo: the drift ignores the
    // trial. Solving against that response would produce a huge, confident
    // velocity.
    let mut controller = MockController::builder().z_drift(Z, 4e-12, 0.0).build();
    let obs = controller.observations();
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    let err = rt
        .drift()
        .expect("the mock supports drift compensation")
        .compensate(&CompensateDrift::new(Z))
        .unwrap_err();

    assert!(format!("{err}").contains("a response of"), "got: {err}");
    let last = *obs
        .lock()
        .drift_comp_writes
        .last()
        .expect("the trial wrote");
    assert_eq!(last.vz, 0.0, "the velocity from before the trial is back");
}

#[test]
fn a_residual_outside_its_error_bar_is_reported_not_hidden() {
    // A response claimed twice as strong as it is halves every correction,
    // which cannot close a 4 pm/s drift in the bursts allowed.
    let mut controller = MockController::builder().z_drift(Z, 4e-12, -1.0).build();
    let obs = controller.observations();
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    let result = rt
        .drift()
        .expect("the mock supports drift compensation")
        .compensate(&CompensateDrift {
            response: Some(-2.0),
            bursts: 3,
            ..CompensateDrift::new(Z)
        })
        .expect("a residual left over is a result, not an error");

    assert!(!result.converged);
    assert_eq!(result.bursts, 3);
    // The k-th correction takes a k-th of its reading: 4 / 2, then 2 / (2 * 2).
    let written: Vec<f64> = obs.lock().drift_comp_writes.iter().map(|c| c.vz).collect();
    assert_eq!(written.len(), 2);
    assert!((written[0] - 2e-12).abs() < 1e-15, "{written:?}");
    assert!((written[1] - 2.5e-12).abs() < 1e-15, "{written:?}");
    // The residual reported belongs to the velocity left on the machine.
    assert!((result.residual.rate_m_s - (4e-12 - result.vz_m_s)).abs() < 1e-15);
}

#[test]
fn too_few_bursts_to_get_past_the_trial_are_refused() {
    // Two bursts with the response unknown would be baseline and trial, and
    // the run would end with the trial velocity on the controller.
    let mut controller = MockController::builder().z_drift(Z, 4e-12, -1.0).build();
    let obs = controller.observations();
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut controller, &events, &shutdown);
    let err = rt
        .drift()
        .expect("the mock supports drift compensation")
        .compensate(&CompensateDrift {
            bursts: 2,
            ..CompensateDrift::new(Z)
        })
        .expect_err("two bursts cannot learn a response and check the result");

    assert!(format!("{err}").contains("too few"), "got: {err}");
    assert!(obs.lock().drift_comp_writes.is_empty());
}
