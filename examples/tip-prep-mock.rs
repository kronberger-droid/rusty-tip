//! Dry-run the full tip-prep routine against the in-memory mock controller —
//! no Nanonis system required. Real logging, fake hardware.
//!
//! Run a scenario (default `sharpen`):
//!
//! ```text
//! cargo run --example tip-prep-mock                 # sharpens, then Completed
//! cargo run --example tip-prep-mock -- cyclelimit   # never sharpens
//! cargo run --example tip-prep-mock -- unstable     # sharp but drifts -> max pulse
//! cargo run --example tip-prep-mock -- fault        # I/O error mid-run + cleanup
//! cargo run --example tip-prep-mock -- shutdown     # Ctrl+C-style graceful stop
//! ```
//!
//! Crank up the detail with `RUST_LOG`:
//!
//! ```text
//! RUST_LOG=debug cargo run --example tip-prep-mock -- unstable
//! ```

use std::thread;
use std::time::Duration;

use env_logger::Env;
use log::info;

use rusty_tip::config::AppConfig;
use rusty_tip::controller_types::{BiasSweepPolarity, PolaritySign, PulseMethod};
use rusty_tip::event::{ConsoleLogger, EventBus};
use rusty_tip::mock_controller::{FaultKind, FreqShiftModel, MockController, models};
use rusty_tip::tip_prep::{Outcome, run_tip_prep};
use rusty_tip::workflow::ShutdownFlag;

/// Signal index the mock's tip model answers for (arbitrary, but the mock and
/// `run_tip_prep` must agree on it).
const FREQ_SHIFT_INDEX: u32 = 2;

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let scenario = std::env::args().nth(1).unwrap_or_else(|| "sharpen".into());

    info!("=== tip-prep mock dry run: scenario '{scenario}' ===");

    let plan = match build_scenario(&scenario) {
        Some(plan) => plan,
        None => {
            eprintln!(
                "unknown scenario '{scenario}'.\n\
                 try: sharpen | cyclelimit | unstable | fault | shutdown"
            );
            std::process::exit(2);
        }
    };

    info!("{}", plan.description);

    let mut builder = MockController::builder()
        .freq_shift_index(FREQ_SHIFT_INDEX)
        .freq_shift(plan.model);
    for (method, kind) in plan.faults {
        builder = builder.fail_every(method, kind);
    }
    let mock = builder.build();
    let obs = mock.observations();

    // Event stream -> console, alongside the routine's own log output.
    let mut events = EventBus::new();
    events.add_observer(Box::new(ConsoleLogger));

    let shutdown = ShutdownFlag::new();
    if plan.request_shutdown_after.is_some() {
        let flag = shutdown.clone();
        let delay = plan.request_shutdown_after.unwrap();
        thread::spawn(move || {
            thread::sleep(delay);
            info!(">>> simulating Ctrl+C: requesting graceful shutdown");
            flag.request();
        });
    }

    let result = run_tip_prep(
        Box::new(mock),
        &events,
        &shutdown,
        &plan.config,
        FREQ_SHIFT_INDEX,
    );

    println!();
    println!("───────────────────────── result ─────────────────────────");
    match &result {
        Ok(Outcome::Completed) => println!("outcome:  Completed (tip is sharp + stable)"),
        Ok(Outcome::StoppedByUser) => println!("outcome:  StoppedByUser"),
        Ok(Outcome::CycleLimit(n)) => println!("outcome:  CycleLimit({n})"),
        Ok(Outcome::TimedOut(d)) => {
            println!("outcome:  TimedOut after {:.0}s", d.as_secs_f64())
        }
        Err(e) => println!("outcome:  Error — {e}"),
    }

    let obs = obs.lock();
    println!("pulses fired:    {}", obs.pulses.len());
    println!("approaches:      {}", obs.approach_count);
    println!("withdraws:       {}", obs.withdraw_count);
    println!("motor moves:     {}", obs.motor_moves);
    println!("freq-shift reads:{}", obs.freq_reads);
    println!("teardown ran:    {}", obs.torn_down);
    println!("───────────────────────────────────────────────────────────");
}

/// A self-contained scenario: the tip model, config tweaks, any injected
/// faults, and an optional auto-shutdown.
struct Scenario {
    description: &'static str,
    model: FreqShiftModel,
    config: AppConfig,
    faults: Vec<(&'static str, FaultKind)>,
    request_shutdown_after: Option<Duration>,
}

fn build_scenario(name: &str) -> Option<Scenario> {
    let s = match name {
        "sharpen" => Scenario {
            description:
                "Tip is blunt (+40 Hz) until 5 pulses land, then sharp (-1 Hz). \
                 Expect a few pulse/reposition cycles, a stability sweep, then Completed.",
            model: models::sharpens_after(5, 40.0, -1.0),
            config: with_stability(base_config()),
            faults: vec![],
            request_shutdown_after: None,
        },

        "cyclelimit" => {
            let mut cfg = base_config();
            cfg.tip_prep.max_cycles = Some(6);
            Scenario {
                description:
                    "Tip never sharpens (+40 Hz forever). Expect 6 pulse cycles, then CycleLimit.",
                model: models::always(40.0),
                config: cfg,
                faults: vec![],
                request_shutdown_after: None,
            }
        }

        "unstable" => {
            let mut cfg = with_stability(base_config());
            cfg.tip_prep.max_cycles = Some(2);
            cfg.tip_prep.stability.polarity_mode = BiasSweepPolarity::Positive;
            cfg.tip_prep.stability.stable_tip_allowed_change = 0.05;
            // Stepping so the recovery max-pulse (6 V) stands out from a normal one.
            cfg.pulse_method = PulseMethod::Stepping {
                voltage_bounds: (2.0, 6.0),
                voltage_steps: 4,
                cycles_before_step: 10,
                threshold_value: 0.1,
                polarity: PolaritySign::Positive,
                random_polarity_switch: None,
            };
            Scenario {
                description:
                    "Tip reads sharp, but the post-sweep measurement has drifted past the \
                     stability threshold. Expect a stability sweep, a 6 V recovery pulse, then CycleLimit.",
                // [0] blunt, [1..4] sharp (baseline), [5] drifted, [6+] blunt.
                model: models::scripted(vec![40.0, -1.0, -1.0, -1.0, -1.0, -1.8, 40.0]),
                config: cfg,
                faults: vec![],
                request_shutdown_after: None,
            }
        }

        "fault" => Scenario {
            description:
                "Every auto-approach fails with an I/O error, so the initial approach aborts \
                 the run. Watch the routine still withdraw and tear down on the way out.",
            model: models::always(-1.0),
            config: base_config(),
            faults: vec![("auto_approach", FaultKind::Io)],
            request_shutdown_after: None,
        },

        "shutdown" => Scenario {
            description:
                "A blunt tip pulses away while a background thread requests shutdown after 3s \
                 (a stand-in for Ctrl+C). Expect StoppedByUser with cleanup.",
            model: models::always(40.0),
            config: base_config(),
            faults: vec![],
            request_shutdown_after: Some(Duration::from_secs(3)),
        },

        _ => return None,
    };
    Some(s)
}

/// A config with visible-but-snappy timings, stability OFF by default.
fn base_config() -> AppConfig {
    let mut cfg = AppConfig::default();

    cfg.tip_prep.sharp_tip_bounds = [-2.0, 0.0];
    cfg.tip_prep.max_cycles = Some(30);
    cfg.tip_prep.max_duration_secs = None;

    let t = &mut cfg.tip_prep.timing;
    t.pulse_width_ms = 50;
    t.post_approach_settle_ms = 100;
    t.post_reposition_settle_ms = 100;
    t.buffer_clear_wait_ms = 50;
    t.post_pulse_settle_ms = 100;
    t.reposition_steps = [2, 2];
    t.status_interval = 5;

    let da = &mut cfg.data_acquisition;
    da.stable_signal_samples = 32;
    da.stable_read_retries = 1;

    cfg.tip_prep.stability.check_stability = false;
    cfg.tip_prep.stability.scan_speed_m_s = None;

    cfg
}

/// Turn stability checking on with a short, watchable bias sweep.
fn with_stability(mut cfg: AppConfig) -> AppConfig {
    cfg.tip_prep.stability.check_stability = true;
    cfg.tip_prep.stability.polarity_mode = BiasSweepPolarity::Both;
    cfg.tip_prep.stability.bias_steps = 20;
    cfg.tip_prep.stability.step_period_ms = 20;
    cfg.tip_prep.stability.scan_speed_m_s = Some(5e-9);
    cfg.tip_prep.stability.stable_tip_allowed_change = 0.2;
    cfg
}
