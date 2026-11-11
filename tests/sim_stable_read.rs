//! The stable read judged against the truth the mock knows.
//!
//! The noise here is real where it can be: `fixtures/df_quiet_noise.txt`
//! holds two steady seconds of frequency shift from a hardware session,
//! which the mock replays. The PLL's noise is blue (neighbours
//! anticorrelate), and that matters: white noise of the same size fails the
//! drift gate far more often, which is why the plain mock needed a slope
//! gate of 20 Hz/s to get through a run at all.
//!
//! As in `sim_drift.rs`, outcome tests carry the bound the read should meet,
//! `#[ignore = "known gap: ..."]` where today's code misses it; run them with
//! `cargo test --test sim_stable_read -- --ignored`.

use std::sync::Arc;

use rusty_tip::config::AppConfig;
use rusty_tip::event::{Event, EventAccumulator, EventBus};
use rusty_tip::mock_controller::{FreqShiftNoise, MockController, models};
use rusty_tip::routine::{Rt, StableReadSpec};
use rusty_tip::shutdown::ShutdownFlag;
use rusty_tip::spm_controller::SpmController;
use rusty_tip::tip_prep::{TipPrepParams, run_tip_prep};
use rusty_tip::{SignalIndex, mock_controller::MockControllerBuilder};

const FREQ_SHIFT: SignalIndex = SignalIndex(2);
const CURRENT: SignalIndex = SignalIndex(0);
const RATE_HZ: f64 = 1000.0;
/// Where the PLL in the 2026-10-06 runs railed: 5 MHz / 2^16.
const RAIL_HZ: f64 = 76.293_945_312_5;

/// Two seconds of real frequency-shift noise, Hz, at 1 kHz.
fn quiet_noise() -> Vec<f64> {
    include_str!("fixtures/df_quiet_noise.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.parse().expect("a number per line"))
        .collect()
}

/// The real noise, scaled to `sigma_hz`: same spectrum, other amplitude.
fn real_noise(sigma_hz: f64) -> FreqShiftNoise {
    let noise = quiet_noise();
    let sd = (noise.iter().map(|v| v * v).sum::<f64>() / noise.len() as f64).sqrt();
    FreqShiftNoise::Replay(
        noise
            .iter()
            .map(|v| v * sigma_hz / sd)
            .collect::<Arc<[f64]>>(),
    )
}

/// The read as tip prep is configured to take it, from the shipped defaults.
fn shipped_spec() -> StableReadSpec {
    let cfg = AppConfig::default();
    StableReadSpec {
        num_samples: cfg.data_acquisition.stable_signal_samples,
        max_std_dev: cfg.tip_prep.signal_stability.max_std_dev_hz,
        max_slope: cfg.tip_prep.signal_stability.max_slope_hz_per_s,
        max_retries: cfg.tip_prep.signal_stability.read_retry_count as usize,
        sample_rate_hz: RATE_HZ,
    }
}

fn mock(value_hz: f64, noise: FreqShiftNoise) -> MockControllerBuilder {
    MockController::builder()
        .freq_shift_index(FREQ_SHIFT)
        .freq_shift(models::always(value_hz))
        .freq_shift_noise(noise)
        .stream_rate(RATE_HZ)
}

/// One stable read: the value, and the `stable` flag of the measurement it
/// reported.
fn read(mock: &mut MockController, spec: &StableReadSpec) -> (f64, bool) {
    let recorder = Arc::new(EventAccumulator::new(usize::MAX));
    let mut events = EventBus::new();
    events.add_observer(Box::new(recorder.clone()));
    let shutdown = ShutdownFlag::new();
    let value = Rt::new(mock, &events, &shutdown)
        .signals()
        .unwrap()
        .read_stable(FREQ_SHIFT, spec)
        .unwrap();
    let stable = recorder
        .all()
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::DataCollected { label, value, .. } if label == "stable_read" => {
                value["stable"].as_bool()
            }
            _ => None,
        })
        .expect("a stable read reports its measurement");
    (value, stable)
}

/// Share of single-attempt reads the gates accept.
fn acceptance(noise: FreqShiftNoise, reads: usize) -> f64 {
    let spec = StableReadSpec {
        max_retries: 0,
        ..shipped_spec()
    };
    let mut mock = mock(-1.0, noise).build();
    let accepted = (0..reads).filter(|_| read(&mut mock, &spec).1).count();
    accepted as f64 / reads as f64
}

fn lag1(xs: &[f64]) -> f64 {
    let m = xs.iter().sum::<f64>() / xs.len() as f64;
    let num: f64 = xs.windows(2).map(|w| (w[0] - m) * (w[1] - m)).sum();
    let den: f64 = xs.iter().map(|x| (x - m).powi(2)).sum();
    num / den
}

// -- The simulator --------------------------------------------------------

#[test]
fn blue_noise_anticorrelates_at_its_size() {
    let mut mock = mock(
        0.0,
        FreqShiftNoise::Blue {
            sigma_hz: 0.3,
            c: 0.22,
        },
    )
    .build();
    let xs = mock.read_signal_samples(FREQ_SHIFT, 100_000).unwrap();
    let sd = (xs.iter().map(|x| x * x).sum::<f64>() / xs.len() as f64).sqrt();
    assert!((sd - 0.3).abs() < 0.01, "{sd}");
    // -c / (1 + c²) for c = 0.22, the lag-1 of the recorded noise.
    assert!((lag1(&xs) + 0.21).abs() < 0.02, "{}", lag1(&xs));
}

#[test]
fn the_recorded_noise_replays_in_order() {
    let noise = quiet_noise();
    assert_eq!(noise.len(), 2000);
    assert!((lag1(&noise) + 0.21).abs() < 0.01, "{}", lag1(&noise));
    let mut mock = mock(-1.0, FreqShiftNoise::Replay(noise.clone().into())).build();
    let first = mock.read_signal_samples(FREQ_SHIFT, 3).unwrap();
    assert_eq!(
        first,
        vec![-1.0 + noise[0], -1.0 + noise[1], -1.0 + noise[2]]
    );
}

#[test]
fn a_tip_beyond_the_rail_reads_as_the_rail_with_no_scatter() {
    let mut mock = mock(-100.0, real_noise(0.3)).pll_rail(RAIL_HZ).build();
    let xs = mock.read_signal_samples(FREQ_SHIFT, 100).unwrap();
    assert!(xs.iter().all(|&x| x == -RAIL_HZ), "{:?}", &xs[..5]);
}

// -- Outcomes -------------------------------------------------------------

#[test]
fn real_quiet_noise_reads_stable_first_time_and_close_to_the_truth() {
    let spec = shipped_spec();
    let mut mock = mock(-1.0, real_noise(0.28)).build();
    let obs = mock.observations();
    let reads = 2000 / spec.num_samples;
    for _ in 0..reads {
        let (value, stable) = read(&mut mock, &spec);
        assert!(stable);
        // A quarter of the tightest tolerance a read is judged against, the
        // 0.2 Hz a stable tip may move by; the sharp window is 1.5 Hz wide.
        assert!((value + 1.0).abs() < 0.05, "{value}");
    }
    assert_eq!(
        obs.lock().count("read_signal_samples"),
        reads,
        "no read needed a retry"
    );
}

#[test]
#[ignore = "known gap: a railed PLL streams a constant, and a batch with no \
            scatter passes the noise gate as perfectly stable"]
fn a_railed_batch_is_never_taken_as_stable() {
    let mut mock = mock(-100.0, real_noise(0.3)).pll_rail(RAIL_HZ).build();
    let (value, stable) = read(&mut mock, &shipped_spec());
    assert!(
        !stable,
        "a railed read of {value} Hz was accepted as stable"
    );
}

#[test]
#[ignore = "known gap: the drift gate's 0.5 Hz/s was set for 500 ms batches, so \
            on 100 samples it rejects the noise of a steady signal in the noisy \
            regime"]
fn a_steady_signal_in_the_noisy_regime_passes_the_gates() {
    // About the noise of the noisier runs on 2026-10-06, under the shipped
    // 1.5 Hz noise gate.
    let accepted = acceptance(real_noise(1.25), 200);
    assert!(accepted >= 0.9, "{:.0} % accepted", accepted * 100.0);
}

#[test]
#[ignore = "known gap: a read that fails its gates on every attempt still hands \
            its mean to tip prep, which picks a pulse from it"]
fn tip_prep_never_pulses_on_a_read_that_failed_its_gates() {
    // A blunt tip whose reading is far too noisy for the 1.5 Hz gate.
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
    let mock = mock(-10.0, FreqShiftNoise::White { sigma_hz: 4.0 }).build();
    let obs = mock.observations();
    let _ = run_tip_prep(
        Box::new(mock),
        TipPrepParams {
            events: &EventBus::new(),
            shutdown: &ShutdownFlag::new(),
            config: &cfg,
            freq_shift: FREQ_SHIFT,
            current: CURRENT,
        },
    );
    assert!(obs.lock().pulses.is_empty(), "{:?}", obs.lock().pulses);
}

// -- Characterisation -----------------------------------------------------

/// The drift gate against noise of the same size but another colour:
/// white noise fails it where the PLL's blue noise passes. Pinned so a
/// change to the gate moves these on purpose.
#[test]
fn characterisation_white_noise_fails_the_slope_gate_where_real_noise_passes() {
    let white = acceptance(FreqShiftNoise::White { sigma_hz: 0.28 }, 200);
    let real = acceptance(real_noise(0.28), 200);
    assert!(
        (0.35..=0.46).contains(&white),
        "white: {:.1} %",
        white * 100.0
    );
    assert!(real >= 0.95, "real: {:.0} %", real * 100.0);
}
