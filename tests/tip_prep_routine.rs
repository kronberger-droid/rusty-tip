//! End-to-end tests for the tip-prep routine driven by the mock controller.
//!
//! Each test wires a [`MockController`] with a chosen tip model (and sometimes
//! injected faults) into [`run_tip_prep`] and asserts on the resulting
//! [`Outcome`] plus the recorded [`MockObservations`]. No hardware involved —
//! these are the dress rehearsals before touching the real machine.

use std::sync::{Arc, Mutex as StdMutex};

use rusty_tip::SignalIndex;
use rusty_tip::config::AppConfig;
use rusty_tip::controller_types::{BiasSweepPolarity, PolaritySign, PulseMethod};
use rusty_tip::event::{Event, EventBus, Observer};
use rusty_tip::mock_controller::{FaultKind, MockController, models};
use rusty_tip::tip_prep::{Outcome, TipPrepParams, run_tip_prep};
use rusty_tip::workflow::ShutdownFlag;

const FREQ_SHIFT_INDEX: SignalIndex = SignalIndex(2);

/// A config tuned for fast tests: zero settle times, a handful of samples,
/// sharp-tip window of `[-2, 0]` Hz. Mutate the returned value per scenario.
fn fast_config() -> AppConfig {
    let mut cfg = AppConfig::default();

    cfg.tip_prep.sharp_tip_bounds = [-2.0, 0.0];
    cfg.tip_prep.max_cycles = Some(5);
    cfg.tip_prep.max_duration_secs = None;

    let t = &mut cfg.tip_prep.timing;
    t.pulse_width_ms = 0;
    t.post_approach_settle_ms = 0;
    t.post_reposition_settle_ms = 0;
    t.buffer_clear_wait_ms = 0;
    t.post_pulse_settle_ms = 0;
    t.reposition_steps = [1, 1];
    t.status_interval = 100;

    cfg.data_acquisition.stable_signal_samples = 16;

    // 20 Hz/s is the old 0.01 Hz/sample gate restated at the 2 kHz default
    // sample rate, so these tests keep judging drift exactly as before.
    let ss = &mut cfg.tip_prep.signal_stability;
    ss.max_std_dev_hz = 1.0;
    ss.max_slope_hz_per_s = 20.0;
    ss.read_retry_count = 0;

    // Default: stability checking OFF so most tests stop at "sharp confirmed".
    // Tests that exercise the sweep turn it back on explicitly.
    cfg.tip_prep.stability.check_stability = false;
    cfg.tip_prep.stability.scan_speed_m_s = None;
    cfg.tip_prep.stability.bias_steps = 2;
    cfg.tip_prep.stability.step_period_ms = 0;
    cfg.tip_prep.stability.stable_tip_allowed_change = 0.05;

    cfg
}

/// Observer that records every event into a shared vec, so a test can inspect
/// the emitted action / state-snapshot stream (what the GUI consumes).
#[derive(Clone, Default)]
struct RecordingObserver {
    events: Arc<StdMutex<Vec<Event>>>,
}

impl Observer for RecordingObserver {
    fn on_event(&self, event: &Event) {
        self.events.lock().unwrap().push(event.clone());
    }
}

/// `true` if any recorded event is a `Custom { kind }` carrying the given phase.
fn saw_phase(events: &[Event], phase: &str) -> bool {
    events.iter().any(|e| match e {
        Event::Custom { kind, data } if kind == "tip_prep_state" => {
            data.get("phase").and_then(|p| p.as_str()) == Some(phase)
        }
        _ => false,
    })
}

// ============================================================================
// Happy paths
// ============================================================================

#[test]
fn already_sharp_completes_when_stability_disabled() {
    // Tip reads -1 Hz (inside [-2, 0]) from the very first measurement.
    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(models::always(-1.0))
        .build();
    let obs = mock.observations();

    let outcome = run_tip_prep(
        Box::new(mock),
        TipPrepParams {
            events: &EventBus::new(),
            shutdown: &ShutdownFlag::new(),
            config: &fast_config(),
            freq_shift: FREQ_SHIFT_INDEX,
        },
    )
    .expect("routine should not error");

    assert!(matches!(outcome, Outcome::Completed));

    let obs = obs.lock();
    assert!(obs.prepared, "prepare() must run before the loop");
    assert!(obs.torn_down, "teardown() must run during cleanup");
    assert!(
        obs.pulses.is_empty(),
        "an already-sharp tip should never be pulsed, got {:?}",
        obs.pulses
    );
    assert!(obs.withdraw_count >= 1, "cleanup must withdraw");
}

#[test]
fn already_sharp_and_stable_completes_through_full_sweep() {
    // Constant -1 Hz => baseline == final => change 0 => stable.
    let mut cfg = fast_config();
    cfg.tip_prep.stability.check_stability = true;
    cfg.tip_prep.stability.polarity_mode = BiasSweepPolarity::Both; // 2 sweeps
    cfg.tip_prep.stability.scan_speed_m_s = Some(5e-9); // exercise scan_speed_*

    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(models::always(-1.0))
        .build();
    let obs = mock.observations();

    let recorder = RecordingObserver::default();
    let events_handle = recorder.events.clone();
    let mut bus = EventBus::new();
    bus.add_observer(Box::new(recorder));

    let outcome = run_tip_prep(
        Box::new(mock),
        TipPrepParams {
            events: &bus,
            shutdown: &ShutdownFlag::new(),
            config: &cfg,
            freq_shift: FREQ_SHIFT_INDEX,
        },
    )
    .expect("routine should not error");

    assert!(matches!(outcome, Outcome::Completed));

    let obs = obs.lock();
    assert!(obs.called("scan_speed_get"), "scan speed should be read");
    assert!(obs.called("scan_speed_set"), "scan speed should be set");
    assert!(obs.called("scan_action"), "stability sweep should scan");

    let events = events_handle.lock().unwrap();
    assert!(
        saw_phase(&events, "stability_check"),
        "should emit stability_check"
    );
    assert!(
        saw_phase(&events, "stable"),
        "should emit a stable snapshot"
    );
}

#[test]
fn tip_sharpens_after_pulses_then_completes() {
    // Blunt (-40 Hz) until 3 pulses land, then sharp (-1 Hz).
    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(models::sharpens_after(3, -40.0, -1.0))
        .build();
    let obs = mock.observations();

    let outcome = run_tip_prep(
        Box::new(mock),
        TipPrepParams {
            events: &EventBus::new(),
            shutdown: &ShutdownFlag::new(),
            config: &fast_config(),
            freq_shift: FREQ_SHIFT_INDEX,
        },
    )
    .expect("routine should not error");

    assert!(matches!(outcome, Outcome::Completed));

    let obs = obs.lock();
    assert!(
        obs.pulses.len() >= 3,
        "needed at least 3 pulses to sharpen, fired {}",
        obs.pulses.len()
    );
    assert!(obs.torn_down);
}

// ============================================================================
// Non-success outcomes
// ============================================================================

#[test]
fn blunt_tip_hits_cycle_limit() {
    let mut cfg = fast_config();
    cfg.tip_prep.max_cycles = Some(3);

    // Never sharp: -40 Hz forever.
    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(models::always(-40.0))
        .build();
    let obs = mock.observations();

    let outcome = run_tip_prep(
        Box::new(mock),
        TipPrepParams {
            events: &EventBus::new(),
            shutdown: &ShutdownFlag::new(),
            config: &cfg,
            freq_shift: FREQ_SHIFT_INDEX,
        },
    )
    .expect("routine should not error");

    match outcome {
        Outcome::CycleLimit(n) => assert_eq!(n, 3),
        other => {
            panic!("expected CycleLimit(3), got {:?}", outcome_name(&other))
        }
    }

    let obs = obs.lock();
    assert_eq!(obs.pulses.len(), 3, "one pulse per cycle for 3 cycles");
    assert!(obs.torn_down);
}

#[test]
fn shutdown_before_loop_stops_by_user() {
    let shutdown = ShutdownFlag::new();
    shutdown.request(); // pre-requested: loop bails on its first iteration

    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(models::always(-40.0)) // blunt => routine reaches the loop
        .build();
    let obs = mock.observations();

    let outcome = run_tip_prep(
        Box::new(mock),
        TipPrepParams {
            events: &EventBus::new(),
            shutdown: &shutdown,
            config: &fast_config(),
            freq_shift: FREQ_SHIFT_INDEX,
        },
    )
    .expect("routine should not error");

    assert!(matches!(outcome, Outcome::StoppedByUser));
    assert!(obs.lock().torn_down, "cleanup must still run on shutdown");
}

// ============================================================================
// Edge cases: faults
// ============================================================================

#[test]
fn io_fault_mid_run_propagates_but_still_cleans_up() {
    // Every auto_approach fails => the initial CalibratedApproach errors out.
    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(models::always(-1.0))
        .fail_every("auto_approach", FaultKind::Io)
        .build();
    let obs = mock.observations();

    let result = run_tip_prep(
        Box::new(mock),
        TipPrepParams {
            events: &EventBus::new(),
            shutdown: &ShutdownFlag::new(),
            config: &fast_config(),
            freq_shift: FREQ_SHIFT_INDEX,
        },
    );

    let spm = match result {
        Err(e) => e,
        Ok(o) => panic!(
            "auto_approach fault should surface as an error, got {}",
            outcome_name(&o)
        ),
    };
    assert!(
        spm.is_connection_error(),
        "expected a connection-class error"
    );

    // The crucial safety property: even though the run aborted, cleanup ran.
    let obs = obs.lock();
    assert!(obs.torn_down, "teardown must run even after a fault");
    assert!(
        obs.withdraw_count >= 1,
        "cleanup must withdraw the tip even after a fault"
    );
    // And the withdraw happened after the failed approach.
    assert!(
        obs.last_index("withdraw") > obs.first_index("auto_approach"),
        "cleanup withdraw should come after the failing approach"
    );
}

#[test]
fn withdraw_fault_during_cleanup_is_swallowed() {
    // With a blunt tip and max_cycles=1 the withdraws are, in order:
    //   #1 the Reposition inside cycle 1  (must succeed, else the loop aborts)
    //   #2 the cleanup withdraw           (faulted here)
    // cleanup() logs withdraw errors instead of propagating, so the run should
    // still report CycleLimit(1) rather than surfacing the I/O error.
    let mut cfg = fast_config();
    cfg.tip_prep.max_cycles = Some(1);

    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(models::always(-40.0))
        .fail_on_call("withdraw", 2, FaultKind::Io)
        .build();
    let obs = mock.observations();

    let outcome = run_tip_prep(
        Box::new(mock),
        TipPrepParams {
            events: &EventBus::new(),
            shutdown: &ShutdownFlag::new(),
            config: &cfg,
            freq_shift: FREQ_SHIFT_INDEX,
        },
    )
    .expect("a failing cleanup withdraw must not turn into a routine error");

    assert!(matches!(outcome, Outcome::CycleLimit(1)));
    let obs = obs.lock();
    assert!(obs.torn_down, "teardown still runs after a withdraw fault");
    assert!(
        obs.count("withdraw") >= 2,
        "the cleanup withdraw should have been attempted"
    );
}

// ============================================================================
// Edge case: sharp-but-unstable triggers the max-pulse recovery branch
// ============================================================================

#[test]
fn sharp_but_unstable_fires_max_pulse_then_cycle_limit() {
    let mut cfg = fast_config();
    cfg.tip_prep.max_cycles = Some(2);
    cfg.tip_prep.stability.check_stability = true;
    cfg.tip_prep.stability.polarity_mode = BiasSweepPolarity::Positive; // 1 sweep
    cfg.tip_prep.stability.stable_tip_allowed_change = 0.05;
    // Stepping so the max-voltage pulse (6 V) is distinct from a normal one (2 V).
    cfg.pulse_method = PulseMethod::Stepping {
        voltage_bounds: (2.0, 6.0),
        voltage_steps: 4,
        cycles_before_step: 10,
        threshold_value: 0.1,
        polarity: PolaritySign::Positive,
        random_polarity_switch: None,
    };

    // Freq-shift read sequence (see runner trace):
    //   [0] pre-loop initial  -> blunt, skip initial stability
    //   [1] cycle-1 measure   -> sharp, enters stability check
    //   [2..4] confirm reads  -> sharp; baseline = read[4]
    //   [5] post-sweep final  -> still in-bounds but drifted 0.8 Hz => UNSTABLE
    //   [6+] cycle-2 measure  -> blunt again => no second stability check
    let scripted = models::scripted(vec![-40.0, -1.0, -1.0, -1.0, -1.0, -1.8, -40.0]);

    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(scripted)
        .build();
    let obs = mock.observations();

    let outcome = run_tip_prep(
        Box::new(mock),
        TipPrepParams {
            events: &EventBus::new(),
            shutdown: &ShutdownFlag::new(),
            config: &cfg,
            freq_shift: FREQ_SHIFT_INDEX,
        },
    )
    .expect("routine should not error");

    assert!(matches!(outcome, Outcome::CycleLimit(2)));

    let obs = obs.lock();
    assert!(
        obs.called("scan_action"),
        "the stability sweep must have run"
    );
    assert!(
        obs.pulses.iter().any(|&v| (v - 6.0).abs() < 1e-9),
        "instability should trigger a max-voltage (6 V) pulse, got {:?}",
        obs.pulses
    );
}

/// The `realistic` tip model must drive the routine end to end, and each stable
/// read must reach observers as one measurement — this is what the GUI's
/// "Simulate" mode and the poster figure plot.
///
/// The sharp band is widened well beyond the real `[-2, 0]` here on purpose.
/// Conditioning is a random search, so with a realistic band a run needs O(100)
/// pulses and its length swings by an order of magnitude — fine for a demo,
/// far too slow and seed-sensitive for a test. A wide band makes a hit likely
/// within a few cycles, which is all this test needs to exercise the plumbing.
/// The statistics of the process itself are asserted in the mock's unit tests.
#[test]
fn realistic_model_completes_and_publishes_measurements() {
    let mut cfg = fast_config();
    cfg.tip_prep.max_cycles = Some(200);
    cfg.tip_prep.sharp_tip_bounds = [-14.0, -9.0];
    cfg.tip_prep.stability.check_stability = true;
    cfg.tip_prep.stability.bias_steps = 2;
    cfg.tip_prep.stability.step_period_ms = 1;
    cfg.data_acquisition.stable_signal_samples = 64;
    // Mirror `configs/mock_demo.toml` — the config the GUI's Simulate mode runs.
    cfg.pulse_method = PulseMethod::Linear {
        voltage_bounds: (2.0, 7.0),
        linear_clamp: (-20.0, 0.0),
        polarity: PolaritySign::Positive,
        random_polarity_switch: None,
    };

    let mock = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(models::realistic(models::RealisticParams::default()))
        .sample_noise_hz(0.25)
        .build();
    let obs = mock.observations();

    /// One captured `stable_read`: the measured value and its batch statistics.
    #[derive(Default)]
    struct Reads(StdMutex<Vec<(f64, f64, usize)>>);
    impl Observer for Reads {
        fn on_event(&self, event: &Event) {
            if let Event::DataCollected { label, value, .. } = event
                && label == "stable_read"
                && let (Some(v), Some(sd), Some(n)) = (
                    value.get("value").and_then(|v| v.as_f64()),
                    value.get("std_dev").and_then(|v| v.as_f64()),
                    value.get("n").and_then(|v| v.as_u64()),
                )
            {
                self.0.lock().unwrap().push((v, sd, n as usize));
            }
        }
    }

    let reads = Arc::new(Reads::default());
    let mut events = EventBus::new();
    events.add_observer(Box::new(ArcObserver(Arc::clone(&reads))));

    let outcome = run_tip_prep(
        Box::new(mock),
        TipPrepParams {
            events: &events,
            shutdown: &ShutdownFlag::new(),
            config: &cfg,
            freq_shift: FREQ_SHIFT_INDEX,
        },
    )
    .expect("routine should not error");

    assert!(
        matches!(outcome, Outcome::Completed),
        "realistic model should reach a sharp, stable tip, got {}",
        outcome_name(&outcome)
    );

    let captured = reads.0.lock().unwrap();

    // One event per stable read — not one per sample.
    assert_eq!(
        captured.len(),
        obs.lock().freq_reads,
        "every stable read should publish exactly one measurement"
    );
    assert!(captured.len() >= 5, "expected several measurements");
    assert!(
        captured.iter().all(|&(_, _, n)| n == 64),
        "each measurement should report the configured sample count"
    );

    // The uncertainty must be real — a constant batch would mean the noise never
    // reached the statistics, and the trace would be a step function again.
    let widest_sd = captured.iter().map(|&(_, sd, _)| sd).fold(0.0, f64::max);
    assert!(
        widest_sd > 0.1,
        "batches look constant, widest std_dev was {widest_sd}"
    );

    // Values must land in the physical range: negative, and in the band the
    // process wanders through rather than pinned at one value.
    let obs = obs.lock();
    assert!(
        obs.freq_values.iter().all(|&v| v < 5.0),
        "shift should stay negative apart from rare insulating events"
    );
}

/// Lets an `Arc`-shared observer be registered without giving up the handle.
struct ArcObserver<T>(Arc<T>);
impl<T: Observer> Observer for ArcObserver<T> {
    fn on_event(&self, event: &Event) {
        self.0.on_event(event);
    }
}

// Tiny helper so panic messages name the outcome (Outcome has no Debug).
fn outcome_name(o: &Outcome) -> &'static str {
    match o {
        Outcome::Completed => "Completed",
        Outcome::StoppedByUser => "StoppedByUser",
        Outcome::CycleLimit(_) => "CycleLimit",
        Outcome::TimedOut(_) => "TimedOut",
    }
}
