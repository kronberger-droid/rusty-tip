//! Drift compensation judged by what it leaves behind.
//!
//! Each test runs `compensate` against a [`DriftScenario`] and reads the
//! truth off the scenario: the drift still acting 30 minutes after the run,
//! with the velocity the run left on the controller. That is what a scan
//! sees, and it is not what the run reports about itself.
//!
//! Three kinds of test live here. The simulator's own checks pass. Outcome
//! tests carry the bound compensation should meet: plain where today's code
//! meets it, `#[ignore = "known gap: ..."]` where it does not, so
//! `cargo test --test sim_drift -- --ignored` lists what is still open. One
//! characterisation pins today's residual under creep, so a change to the
//! algorithm has to move it on purpose.

use std::time::Duration;

use rusty_tip::SignalIndex;
use rusty_tip::action::drift::{CompensateDrift, DriftCompensation};
use rusty_tip::event::EventBus;
use rusty_tip::mock_controller::{DriftScenario, MockController, ZNoise};
use rusty_tip::routine::Rt;
use rusty_tip::shutdown::ShutdownFlag;
use rusty_tip::spm_controller::{DriftComp, SpmController};
use rusty_tip::spm_error::SpmError;

const Z: SignalIndex = SignalIndex(30);
const PM: f64 = 1e-12;
/// How long after the run a scan is still relying on the compensation.
const HORIZON_S: f64 = 1800.0;

/// White noise of this size gives a 5 s burst at 1 kHz a standard error of
/// about 0.3 pm/s, the figure the drift docs work with.
const Z_NOISE_M: f64 = 30e-12;

/// A drift run as the instrument would see it: noise on Z, the controller
/// round trips between bursts, a compensation that takes a moment to act.
fn realistic(drift_m_s: f64, response: f64) -> DriftScenario {
    DriftScenario::constant(drift_m_s, response)
        .noise(ZNoise::White { sigma_m: Z_NOISE_M })
        .set_overhead(0.3)
        .latency(0.1)
}

/// What a compensation run left: its own report, and the truth.
struct Outcome {
    result: Result<DriftCompensation, SpmError>,
    /// Drift acting `HORIZON_S` after the run, m/s.
    residual_later: f64,
    /// Simulated time the run took, s.
    took_s: f64,
    writes: Vec<DriftComp>,
}

fn compensate_with(
    scenario: &DriftScenario,
    seed: u64,
    prepare: impl FnOnce(&mut DriftComp),
) -> Outcome {
    run(scenario, seed, &CompensateDrift::new(Z), prepare)
}

fn compensate(scenario: &DriftScenario, seed: u64) -> Outcome {
    compensate_with(scenario, seed, |_| {})
}

fn compensate_quick(scenario: &DriftScenario, seed: u64) -> Outcome {
    run(scenario, seed, &CompensateDrift::quick(Z), |_| {})
}

fn run(
    scenario: &DriftScenario,
    seed: u64,
    action: &CompensateDrift,
    prepare: impl FnOnce(&mut DriftComp),
) -> Outcome {
    let mut mock = MockController::builder()
        .noise_seed(seed)
        .z_drift_scenario(Z, scenario.clone())
        .build();
    let obs = mock.observations();
    prepare(&mut obs.lock().drift_comp);
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let result = Rt::new(&mut mock, &events, &shutdown)
        .drift()
        .expect("the mock supports drift compensation")
        .compensate(action);

    let obs = obs.lock();
    let comp = obs.drift_comp;
    let acting = if comp.enabled && !comp.z_saturated {
        comp.vz
    } else {
        0.0
    };
    Outcome {
        result,
        residual_later: scenario.residual_at(obs.z_sim_time_s + HORIZON_S, acting),
        took_s: obs.z_sim_time_s - scenario.start_s,
        writes: obs.drift_comp_writes.clone(),
    }
}

// -- The simulator --------------------------------------------------------

#[test]
fn the_clock_runs_on_samples_and_controller_writes() {
    let scenario = DriftScenario::constant(0.0, -1.0)
        .starting_after(30.0)
        .set_overhead(0.3);
    let mut mock = MockController::builder()
        .z_drift_scenario(Z, scenario)
        .build();
    let obs = mock.observations();

    mock.read_signal_samples(Z, 5000).unwrap();
    assert!((obs.lock().z_sim_time_s - 35.0).abs() < 1e-9);
    let comp = obs.lock().drift_comp;
    mock.drift_comp_set(&comp).unwrap();
    mock.read_signal_samples(Z, 1).unwrap();
    assert!((obs.lock().z_sim_time_s - 35.301).abs() < 1e-9);
}

#[test]
fn creep_dies_out_with_its_time_constant() {
    let scenario = DriftScenario::constant(2.0 * PM, -1.0).creep(20.0 * PM, 60.0);
    let mut mock = MockController::builder()
        .z_drift_scenario(Z, scenario.clone())
        .build();
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut mock, &events, &shutdown);

    // A 2 s burst at t = 0 and one at t = 300 s, each fitted over its own
    // window, read the creep where it is.
    let early = rt
        .drift()
        .unwrap()
        .measure_z(Z, Duration::from_secs(2))
        .unwrap();
    rt.controller().read_signal_samples(Z, 298_000).unwrap();
    let late = rt
        .drift()
        .unwrap()
        .measure_z(Z, Duration::from_secs(2))
        .unwrap();

    let at = |t: f64| scenario.drift_at(t);
    assert!((early.rate_m_s - at(1.0)).abs() < 0.05 * PM, "{early:?}");
    assert!((late.rate_m_s - at(301.0)).abs() < 0.05 * PM, "{late:?}");
    assert!(
        late.rate_m_s < 2.2 * PM,
        "five time constants later: {late:?}"
    );
}

#[test]
fn the_response_acts_with_its_size_and_after_its_latency() {
    let scenario = DriftScenario::constant(5.0 * PM, -0.8).latency(1.0);
    let mut mock = MockController::builder()
        .z_drift_scenario(Z, scenario)
        .build();
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut rt = Rt::new(&mut mock, &events, &shutdown);

    let comp = DriftComp {
        enabled: true,
        vz: 5.0 * PM,
        ..rt.controller().drift_comp_get().unwrap()
    };
    rt.controller().drift_comp_set(&comp).unwrap();
    // Inside the latency nothing has changed yet.
    let before = rt
        .drift()
        .unwrap()
        .measure_z(Z, Duration::from_millis(900))
        .unwrap();
    assert!((before.rate_m_s - 5.0 * PM).abs() < 0.01 * PM, "{before:?}");
    rt.controller().read_signal_samples(Z, 200).unwrap();
    let after = rt
        .drift()
        .unwrap()
        .measure_z(Z, Duration::from_secs(2))
        .unwrap();
    assert!(
        (after.rate_m_s - 1.0 * PM).abs() < 0.01 * PM,
        "5 - 0.8 * 5: {after:?}"
    );
}

#[test]
fn saturation_latches_stops_compensating_and_rearms_when_switched_off() {
    let scenario = DriftScenario::constant(5.0 * PM, -1.0).saturates_at(1e-9);
    let mut mock = MockController::builder()
        .z_drift_scenario(Z, scenario)
        .build();
    let obs = mock.observations();
    let on = DriftComp {
        enabled: true,
        vz: 100.0 * PM,
        ..obs.lock().drift_comp
    };
    mock.drift_comp_set(&on).unwrap();

    // 100 pm/s reaches 1 nm in 10 s.
    mock.read_signal_samples(Z, 9_000).unwrap();
    assert!(!obs.lock().drift_comp.z_saturated);
    mock.read_signal_samples(Z, 2_000).unwrap();
    assert!(obs.lock().drift_comp.z_saturated, "latched past 1 nm");

    // Latched, the axis no longer compensates: Z is back to its own drift.
    let z0 = mock.read_signal_samples(Z, 1).unwrap()[0];
    let z1 = mock.read_signal_samples(Z, 1000).unwrap()[999];
    assert!(((z1 - z0) - 5.0 * PM).abs() < 0.01 * PM);

    mock.drift_comp_set(&DriftComp {
        enabled: false,
        ..on
    })
    .unwrap();
    mock.drift_comp_set(&on).unwrap();
    assert!(!obs.lock().drift_comp.z_saturated, "off and on re-arms");
}

#[test]
fn correlated_noise_keeps_its_amplitude() {
    let scenario = DriftScenario::constant(0.0, -1.0).noise(ZNoise::Ar1 {
        sigma_m: 20e-12,
        tau_s: 0.05,
    });
    let mut mock = MockController::builder()
        .z_drift_scenario(Z, scenario)
        .build();
    let z = mock.read_signal_samples(Z, 200_000).unwrap();
    let mean = z.iter().sum::<f64>() / z.len() as f64;
    let sd = (z.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / z.len() as f64).sqrt();
    assert!((sd - 20e-12).abs() < 2e-12, "{sd:e}");
}

// -- Outcomes -------------------------------------------------------------

#[test]
fn a_constant_drift_stays_cancelled() {
    for seed in 1..=5 {
        let outcome = compensate(&realistic(5.0 * PM, -1.0), seed);
        let result = outcome.result.expect("compensates");
        assert!(
            outcome.residual_later.abs() < 0.6 * PM,
            "seed {seed}: {:.2} pm/s left, reported {:?}",
            outcome.residual_later / PM,
            result.residual
        );
    }
}

/// Careful settling never leaves more than 1 pm/s behind: creep it can wait
/// out within its budget is cancelled to that, and creep too slow for it is
/// refused with nothing switched on, so the operator knows to come back.
#[test]
fn creep_is_waited_out_or_refused_never_left_behind() {
    for tau_s in [60.0, 120.0, 300.0] {
        let scenario = realistic(5.0 * PM, -1.0)
            .creep(20.0 * PM, tau_s)
            .starting_after(30.0);
        let outcome = compensate(&scenario, 7);
        match &outcome.result {
            Ok(_) => assert!(
                outcome.residual_later.abs() < 1.0 * PM,
                "tau {tau_s} s: {:.2} pm/s left after {:.0} s",
                outcome.residual_later / PM,
                outcome.took_s,
            ),
            Err(e) => {
                assert!(
                    e.to_string().contains("still changing"),
                    "tau {tau_s} s: {e}"
                );
                assert!(
                    outcome.writes.iter().all(|w| !w.enabled),
                    "nothing switched on"
                );
            }
        }
    }
}

/// The cases careful settling has to finish rather than refuse, and how
/// long it takes.
#[test]
fn creep_up_to_two_minutes_is_cancelled_within_the_budget() {
    for tau_s in [60.0, 120.0] {
        let scenario = realistic(5.0 * PM, -1.0)
            .creep(20.0 * PM, tau_s)
            .starting_after(30.0);
        let outcome = compensate(&scenario, 7);
        assert!(
            outcome.result.is_ok(),
            "tau {tau_s} s: {:?}",
            outcome.result
        );
        assert!(
            outcome.took_s < 600.0,
            "tau {tau_s} s took {:.0} s",
            outcome.took_s
        );
    }
}

/// A constant drift is settled once two minutes of bursts show no trend.
#[test]
fn careful_takes_about_two_minutes_for_a_drift_that_holds_still() {
    let outcome = compensate(&realistic(5.0 * PM, -1.0), 1);
    outcome.result.expect("compensates");
    assert!(
        (120.0..=200.0).contains(&outcome.took_s),
        "{:.0} s",
        outcome.took_s
    );
}

#[test]
fn a_response_ten_percent_off_still_converges() {
    for response in [-0.9, -1.1] {
        let scenario = DriftScenario::constant(5.0 * PM, response).set_overhead(0.3);
        let outcome = compensate(&scenario, 1);
        assert!(
            outcome.residual_later.abs() < 0.1 * PM,
            "response {response}: {:.2} pm/s left",
            outcome.residual_later / PM
        );
    }
}

/// Through noise the size is used once the trial resolves it, and the run
/// does as well as one whose response is exactly one.
#[test]
fn a_response_ten_percent_off_is_cancelled_through_noise() {
    for seed in 1..=5 {
        for response in [-0.9, -1.1] {
            let outcome = compensate(&realistic(5.0 * PM, response), seed);
            outcome.result.expect("compensates");
            assert!(
                outcome.residual_later.abs() < 0.6 * PM,
                "seed {seed}, response {response}: {:.2} pm/s left",
                outcome.residual_later / PM
            );
        }
    }
}

#[test]
fn lateral_velocities_in_effect_are_the_operators_and_stay() {
    let outcome = compensate_with(&realistic(5.0 * PM, -1.0), 1, |comp| {
        comp.enabled = true;
        comp.vx = 50.0 * PM;
        comp.vy = -30.0 * PM;
    });
    outcome.result.expect("compensates");
    for write in &outcome.writes {
        assert_eq!((write.vx, write.vy), (50.0 * PM, -30.0 * PM), "{write:?}");
    }
}

#[test]
fn compensation_never_switches_on_stale_lateral_velocities() {
    let outcome = compensate_with(&realistic(5.0 * PM, -1.0), 1, |comp| {
        comp.enabled = false;
        comp.vx = 50.0 * PM;
        comp.vy = -30.0 * PM;
    });
    outcome.result.expect("compensates");
    for write in outcome.writes.iter().filter(|w| w.enabled) {
        assert_eq!((write.vx, write.vy), (0.0, 0.0), "{write:?}");
    }
}

#[test]
#[ignore = "known gap: the standard error assumes independent block means, and \
            correlated Z noise makes it optimistic by a factor of two or more"]
fn the_reported_residual_is_inside_its_own_error_bar() {
    let scenario = realistic(5.0 * PM, -1.0).noise(ZNoise::Ar1 {
        sigma_m: 60e-12,
        tau_s: 2.0,
    });
    let runs = 40;
    let finished: Vec<bool> = (1..=runs)
        .filter_map(|seed| {
            let outcome = compensate(&scenario, seed);
            // The drift is constant, so the truth after the run is the
            // truth during its last burst.
            let r = outcome.result.ok()?;
            Some(
                (r.residual.rate_m_s - outcome.residual_later).abs() > 2.0 * r.residual.std_err_m_s,
            )
        })
        .collect();
    let outside = finished.iter().filter(|&&o| o).count();
    // Two standard errors cover 95 %; allow some slack.
    assert!(
        finished.len() == runs as usize && outside * 10 <= finished.len(),
        "{} of {runs} runs finished; {outside} of them report a residual outside 2 SE of the truth",
        finished.len()
    );
}

#[test]
fn a_never_settling_drift_gives_up_after_the_budget_in_measured_time() {
    // 1/t creep so strong it never holds still within the budget: at 600 s
    // it still changes by about 2.7 pm/s over the two minutes the careful
    // test spans.
    let scenario = realistic(0.0, -1.0).log_creep(1e-8, 5.0);
    let outcome = compensate(&scenario, 1);
    let err = outcome.result.expect_err("refuses a drift still changing");
    assert!(err.to_string().contains("still changing"), "{err}");
    assert!(
        outcome.writes.iter().all(|w| !w.enabled),
        "nothing was switched on"
    );
    // The careful budget is 600 s, counted in measured time: the run stops
    // after the burst that crossed it.
    assert!(
        (600.0..=610.0).contains(&outcome.took_s),
        "{:.1} s",
        outcome.took_s
    );
}

// -- Characterisation -----------------------------------------------------

/// What quick settling leaves under a two-minute creep: the price of the
/// half minute it takes. Pinned so a change to it moves this on purpose;
/// careful settling is held to 1 pm/s in
/// `creep_is_waited_out_or_refused_never_left_behind`.
#[test]
fn characterisation_quick_settling_leaves_two_minute_creep_behind() {
    let scenario = realistic(5.0 * PM, -1.0)
        .creep(20.0 * PM, 120.0)
        .starting_after(30.0);
    let outcome = compensate_quick(&scenario, 7);
    let left = outcome.residual_later / PM;
    assert!((-11.5..=-9.3).contains(&left), "{left:.2} pm/s left");
}
