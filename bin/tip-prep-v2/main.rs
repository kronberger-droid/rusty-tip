mod config;

use chrono::Utc;
use clap::Parser;
use env_logger::Env;
use log::{error, info, LevelFilter};
use std::{
    fs,
    io,
    path::PathBuf,
    time::Duration,
};

use rusty_tip::action::{ActionContext, ActionOutput};
use rusty_tip::action::bias::{BiasPulse, SetBias};
use rusty_tip::action::motor::MoveMotor3D;
use rusty_tip::action::pll::CenterFreqShift;
use rusty_tip::action::scan::{ScanActionParam, ScanControl, ScanDirectionParam};
use rusty_tip::action::signals::ReadSignal;
use rusty_tip::action::util::Wait;
use rusty_tip::action::z_controller::{AutoApproach, SetZSetpoint, Withdraw};
use rusty_tip::{BiasSweepPolarity, PolaritySign};
use rusty_tip::action::Action;
use rusty_tip::action::DataStore;
use rusty_tip::event::{ConsoleLogger, EventAccumulator, EventBus, FileLogger};
use rusty_tip::nanonis_controller::NanonisController;
use rusty_tip::signal_registry::SignalRegistry;
use rusty_tip::spm_controller::SpmController;
use rusty_tip::workflow::ShutdownFlag;

use crate::config::{load_config, AppConfig};

/// Rusty Tip Preparation Tool (v2)
#[derive(Parser, Debug)]
#[command(name = "tip-prep-v2")]
#[command(about = "Automated tip preparation for STM/AFM (v2)", long_about = None)]
struct Args {
    /// Path to configuration file (required)
    #[arg(short, long, value_name = "FILE", required = true)]
    config: PathBuf,

    /// Override log level (trace, debug, info, warn, error)
    #[arg(short, long, value_name = "LEVEL")]
    log_level: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    ensure_console_allocated();

    let args = Args::parse();
    let config = load_config(&args.config)?;

    let log_level = args
        .log_level
        .unwrap_or(config.console.verbosity.clone());
    initialize_logging(&log_level)?;

    info!("=== Rusty Tip Preparation Tool (v2) ===");
    info!("Configuration: {}", args.config.display());
    info!(
        "Nanonis: {}:{}",
        config.nanonis.host_ip, config.nanonis.control_ports[0]
    );

    // Connect to hardware
    let client = rusty_tip::NanonisClient::builder()
        .address(&config.nanonis.host_ip)
        .port(config.nanonis.control_ports[0])
        .build()?;
    let mut controller = NanonisController::new(client);
    info!("Connected to Nanonis system");

    // Build signal registry
    let signal_names = controller.signal_names()?;
    let registry = build_signal_registry(&signal_names, &config);
    let freq_shift_signal = registry
        .get_by_name("freq shift")
        .ok_or("Frequency shift signal not found in registry")?;
    info!(
        "Frequency shift signal: index {}{}",
        freq_shift_signal.index,
        freq_shift_signal
            .tcp_channel
            .map(|ch| format!(", TCP channel {}", ch))
            .unwrap_or_default()
    );
    let freq_shift_index = freq_shift_signal.index as u32;

    // Setup event bus
    let events = setup_event_bus(&config)?;

    // Setup shutdown handler
    let shutdown = setup_shutdown_handler();

    // Wait for user confirmation
    wait_for_user_confirmation()?;

    // Run tip preparation
    let result = run_tip_prep(controller, &events, &shutdown, &config, freq_shift_index);

    match &result {
        Ok(Outcome::Completed) => info!("Tip preparation completed successfully!"),
        Ok(Outcome::StoppedByUser) => info!("Tip preparation stopped by user"),
        Ok(Outcome::CycleLimit(n)) => error!("Max cycles ({}) exceeded", n),
        Ok(Outcome::TimedOut(d)) => {
            error!("Max duration ({:.0}s) exceeded", d.as_secs_f64())
        }
        Err(e) => error!("Tip preparation failed: {}", e),
    }

    info!("Cleanup complete");
    result.map(|_| ())
}

// ============================================================================
// Tip preparation logic
// ============================================================================

enum Outcome {
    Completed,
    StoppedByUser,
    CycleLimit(usize),
    TimedOut(Duration),
}

fn run_tip_prep(
    mut controller: NanonisController,
    events: &EventBus,
    shutdown: &ShutdownFlag,
    config: &AppConfig,
    freq_shift_index: u32,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    let mut store = DataStore::new();
    let mut ctx = ActionContext {
        controller: &mut controller,
        store: &mut store,
        events,
    };

    // Pre-loop initialization
    info!("Initializing...");
    SetBias {
        voltage: config.tip_prep.initial_bias_v as f64,
    }
    .execute(&mut ctx)?;

    SetZSetpoint {
        setpoint: config.tip_prep.initial_z_setpoint_a as f64,
    }
    .execute(&mut ctx)?;

    AutoApproach::default().execute(&mut ctx)?;
    CenterFreqShift.execute(&mut ctx)?;

    Wait {
        duration_ms: config.tip_prep.timing.post_approach_settle_ms,
    }
    .execute(&mut ctx)?;

    let bounds = (
        config.tip_prep.sharp_tip_bounds[0] as f64,
        config.tip_prep.sharp_tip_bounds[1] as f64,
    );

    let max_cycles = config.tip_prep.max_cycles.unwrap_or(usize::MAX);
    let max_duration = config
        .tip_prep
        .max_duration_secs
        .map(Duration::from_secs);
    let start_time = std::time::Instant::now();
    let mut pulse_state = PulseState::new(config);

    // Main loop: Blunt -> Sharp
    // (Stability checking will be added as a composite action later)
    for cycle in 1..=max_cycles {
        // Check shutdown
        if shutdown.is_requested() {
            return Ok(Outcome::StoppedByUser);
        }

        // Check timeout
        if let Some(max_dur) = max_duration {
            if start_time.elapsed() > max_dur {
                return Ok(Outcome::TimedOut(max_dur));
            }
        }

        // Periodic status
        if cycle % config.tip_prep.timing.status_interval == 0 {
            info!(
                "Cycle {}: elapsed={:.1}s",
                cycle,
                start_time.elapsed().as_secs_f64()
            );
        }

        // Read signal before pulse (for pulse strategy decisions)
        let output = ReadSignal {
            index: freq_shift_index,
            ..Default::default()
        }
        .execute_and_store(&mut ctx, "freq_shift")?;

        let current_freq_shift = match output {
            ActionOutput::Value(v) => Some(v),
            _ => None,
        };

        // Pulse with strategy-determined voltage (magnitude + polarity sign)
        pulse_state.update_voltage(config, current_freq_shift);
        let pulse_voltage = pulse_state.signed_voltage();
        BiasPulse {
            voltage: pulse_voltage,
            duration_ms: config.tip_prep.timing.pulse_width_ms,
            ..Default::default()
        }
        .execute(&mut ctx)?;

        Wait {
            duration_ms: config.tip_prep.timing.post_pulse_settle_ms,
        }
        .execute(&mut ctx)?;

        // Read signal after pulse and check tip state
        let output = ReadSignal {
            index: freq_shift_index,
            ..Default::default()
        }
        .execute_and_store(&mut ctx, "freq_shift")?;

        if let ActionOutput::Value(freq_shift) = output {
            let is_sharp = freq_shift >= bounds.0 && freq_shift <= bounds.1;

            if is_sharp {
                info!(
                    "Tip sharp at cycle {} (freq_shift={:.3} Hz)",
                    cycle, freq_shift
                );

                match check_stability(
                    &mut ctx,
                    freq_shift_index,
                    bounds,
                    config,
                    shutdown,
                    &mut pulse_state,
                )? {
                    StabilityOutcome::Stable => {
                        info!("Tip confirmed stable!");
                        return Ok(Outcome::Completed);
                    }
                    StabilityOutcome::NotSharp => {
                        info!("Tip not confirmed sharp - continuing");
                    }
                    StabilityOutcome::Unstable => {
                        info!("Stability check failed - reset to blunt, continuing");
                    }
                }
            }
        }

        // Reposition for next cycle (withdraw + motor move + approach)
        reposition(
            &mut ctx,
            config.tip_prep.timing.reposition_steps,
            config.tip_prep.timing.post_reposition_settle_ms,
            config.tip_prep.timing.post_approach_settle_ms,
        )?;
    }

    Ok(Outcome::CycleLimit(max_cycles))
}

/// Withdraw, move motor to a new XY position, and re-approach.
///
/// This mirrors the old SafeReposition: withdraw -> motor move (x, y, z=-3) -> settle -> approach -> center -> settle.
fn reposition(
    ctx: &mut ActionContext,
    reposition_steps: [i16; 2],
    post_reposition_settle_ms: u64,
    post_approach_settle_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    Withdraw::default().execute(ctx)?;

    MoveMotor3D {
        x: reposition_steps[0],
        y: reposition_steps[1],
        z: -3,
        wait: true,
    }
    .execute(ctx)?;

    Wait {
        duration_ms: post_reposition_settle_ms,
    }
    .execute(ctx)?;

    AutoApproach::default().execute(ctx)?;
    CenterFreqShift.execute(ctx)?;

    Wait {
        duration_ms: post_approach_settle_ms,
    }
    .execute(ctx)?;

    Ok(())
}

/// Pre-stability confirmation: reposition and re-read freq_shift 3 times.
/// All readings must fall within sharp bounds to confirm the tip is actually sharp
/// (not just a sample artifact at one location).
///
/// Returns (confirmed_sharp, baseline_freq_shift).
fn confirm_sharp(
    ctx: &mut ActionContext,
    freq_shift_index: u32,
    bounds: (f64, f64),
    config: &AppConfig,
    shutdown: &ShutdownFlag,
) -> Result<(bool, Option<f64>), Box<dyn std::error::Error>> {
    const CONFIRMATION_READS: usize = 3;
    let mut last_freq_shift = None;

    for i in 0..CONFIRMATION_READS {
        if shutdown.is_requested() {
            return Err("Shutdown requested during confirmation".into());
        }

        reposition(
            ctx,
            config.tip_prep.timing.reposition_steps,
            config.tip_prep.timing.post_reposition_settle_ms,
            config.tip_prep.timing.post_approach_settle_ms,
        )?;

        if shutdown.is_requested() {
            return Err("Shutdown requested during confirmation".into());
        }

        let output = ReadSignal {
            index: freq_shift_index,
            ..Default::default()
        }
        .execute(ctx)?;

        if let ActionOutput::Value(fs) = output {
            let in_bounds = fs >= bounds.0 && fs <= bounds.1;
            info!(
                "Confirmation {}/{}: freq_shift={:.3} Hz, in_bounds={}",
                i + 1,
                CONFIRMATION_READS,
                fs,
                in_bounds
            );
            if !in_bounds {
                return Ok((false, None));
            }
            last_freq_shift = Some(fs);
        }
    }

    Ok((true, last_freq_shift))
}

// ============================================================================
// Stability sweep
// ============================================================================

/// A single bias sweep plan.
struct SweepPlan {
    starting_bias: f64,
    bias_range: (f64, f64),
    index: usize,
    total: usize,
}

/// Build sweep plans based on polarity mode.
fn build_sweep_plans(config: &AppConfig) -> Vec<SweepPlan> {
    let sc = &config.tip_prep.stability;
    let range = sc.bias_range;

    match sc.polarity_mode {
        BiasSweepPolarity::Positive => vec![SweepPlan {
            starting_bias: range.1 as f64,
            bias_range: (range.1 as f64, range.0 as f64),
            index: 1,
            total: 1,
        }],
        BiasSweepPolarity::Negative => vec![SweepPlan {
            starting_bias: -(range.1 as f64),
            bias_range: (-(range.1 as f64), -(range.0 as f64)),
            index: 1,
            total: 1,
        }],
        BiasSweepPolarity::Both => vec![
            SweepPlan {
                starting_bias: range.1 as f64,
                bias_range: (range.1 as f64, range.0 as f64),
                index: 1,
                total: 2,
            },
            SweepPlan {
                starting_bias: -(range.1 as f64),
                bias_range: (-(range.1 as f64), -(range.0 as f64)),
                index: 2,
                total: 2,
            },
        ],
    }
}

/// Prepare for a stability sweep: withdraw, motor move, set bias, approach.
fn prepare_for_sweep(
    ctx: &mut ActionContext,
    plan: &SweepPlan,
    config: &AppConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    Withdraw::default().execute(ctx)?;

    MoveMotor3D {
        x: config.tip_prep.timing.reposition_steps[0],
        y: config.tip_prep.timing.reposition_steps[1],
        z: -3,
        wait: true,
    }
    .execute(ctx)?;

    Wait { duration_ms: 200 }.execute(ctx)?;

    SetBias {
        voltage: plan.starting_bias,
    }
    .execute(ctx)?;

    AutoApproach::default().execute(ctx)?;
    CenterFreqShift.execute(ctx)?;

    Wait {
        duration_ms: config.tip_prep.timing.post_approach_settle_ms,
    }
    .execute(ctx)?;

    Ok(())
}

/// Execute a single stability sweep: start scan, step bias through range,
/// read freq_shift at each step via ReadSignal.
fn execute_stability_sweep(
    ctx: &mut ActionContext,
    plan: &SweepPlan,
    config: &AppConfig,
    shutdown: &ShutdownFlag,
) -> Result<(), Box<dyn std::error::Error>> {
    let sc = &config.tip_prep.stability;

    info!(
        "Sweep {}/{}: bias {:.2}V -> {:.2}V",
        plan.index, plan.total, plan.bias_range.0, plan.bias_range.1
    );

    // Configure scan for stability check: continuous + bouncy
    use nanonis_rs::scan::ScanPropsBuilder;
    let original_props = ctx.controller.scan_props_get()?;
    ctx.controller.scan_props_set(
        ScanPropsBuilder::new()
            .continuous_scan(true)
            .bouncy_scan(true),
    )?;

    // Start scan
    ScanControl {
        action: ScanActionParam::Start,
        direction: ScanDirectionParam::Down,
    }
    .execute(ctx)?;

    // Wait for scan to actually start (max 5 seconds)
    let mut scan_started = false;
    for _ in 0..50 {
        if shutdown.is_requested() {
            let _ = ScanControl {
                action: ScanActionParam::Stop,
                direction: ScanDirectionParam::Up,
            }
            .execute(ctx);
            restore_scan_props(ctx, &original_props);
            return Err("Shutdown requested".into());
        }
        std::thread::sleep(Duration::from_millis(100));
        if ctx.controller.scan_status()? {
            scan_started = true;
            break;
        }
    }

    if !scan_started {
        restore_scan_props(ctx, &original_props);
        return Err("Scan failed to start within 5 seconds".into());
    }

    // Step bias through range
    let bias_step_size =
        (plan.bias_range.1 - plan.bias_range.0) / sc.bias_steps as f64;
    let mut current_bias = plan.bias_range.0;
    let step_duration = Duration::from_millis(sc.step_period_ms);

    for step in 0..sc.bias_steps {
        if shutdown.is_requested() {
            let _ = ScanControl {
                action: ScanActionParam::Stop,
                direction: ScanDirectionParam::Up,
            }
            .execute(ctx);
            restore_scan_props(ctx, &original_props);
            return Err("Shutdown requested".into());
        }

        SetBias {
            voltage: current_bias,
        }
        .execute(ctx)?;

        log::debug!(
            "Step {}/{}: bias={:.3}V",
            step + 1,
            sc.bias_steps,
            current_bias
        );

        // Interruptible sleep
        interruptible_sleep(step_duration, shutdown)?;

        current_bias += bias_step_size;
    }

    info!("Bias sweep completed");

    // Stop scan
    let _ = ScanControl {
        action: ScanActionParam::Stop,
        direction: ScanDirectionParam::Up,
    }
    .execute(ctx);

    // Restore scan properties
    restore_scan_props(ctx, &original_props);

    Ok(())
}

/// Restore scan properties after stability sweep.
fn restore_scan_props(ctx: &mut ActionContext, original: &nanonis_rs::scan::ScanProps) {
    let builder = original.to_builder();
    if let Err(e) = ctx.controller.scan_props_set(builder) {
        log::error!("Failed to restore scan properties: {}", e);
    }
}

/// After all sweeps: withdraw, restore initial bias, approach, read freq_shift.
fn measure_final_freq_shift(
    ctx: &mut ActionContext,
    config: &AppConfig,
    freq_shift_index: u32,
) -> Result<Option<f64>, Box<dyn std::error::Error>> {
    info!("Measuring final freq_shift after sweeps");

    Withdraw::default().execute(ctx)?;
    Wait { duration_ms: 200 }.execute(ctx)?;

    SetBias {
        voltage: config.tip_prep.initial_bias_v as f64,
    }
    .execute(ctx)?;

    AutoApproach::default().execute(ctx)?;
    CenterFreqShift.execute(ctx)?;

    Wait {
        duration_ms: config.tip_prep.timing.post_approach_settle_ms,
    }
    .execute(ctx)?;

    let output = ReadSignal {
        index: freq_shift_index,
        ..Default::default()
    }
    .execute(ctx)?;

    match output {
        ActionOutput::Value(v) => Ok(Some(v)),
        _ => Ok(None),
    }
}

/// Full stability check: confirm sharpness, then run bias sweeps,
/// then compare baseline vs final freq_shift.
///
/// Returns Ok(true) if tip is stable, Ok(false) if not.
fn check_stability(
    ctx: &mut ActionContext,
    freq_shift_index: u32,
    bounds: (f64, f64),
    config: &AppConfig,
    shutdown: &ShutdownFlag,
    pulse_state: &mut PulseState,
) -> Result<StabilityOutcome, Box<dyn std::error::Error>> {
    // Step 1: Confirm sharpness with repositioning (3 reads)
    let (confirmed, baseline) = confirm_sharp(ctx, freq_shift_index, bounds, config, shutdown)?;

    if !confirmed {
        info!("Tip not confirmed sharp during pre-check");
        return Ok(StabilityOutcome::NotSharp);
    }

    if !config.tip_prep.stability.check_stability {
        info!("Stability checking disabled - accepting sharp tip");
        return Ok(StabilityOutcome::Stable);
    }

    let baseline = match baseline {
        Some(v) => v,
        None => {
            error!("No baseline freq_shift available");
            return Ok(StabilityOutcome::NotSharp);
        }
    };

    info!("Baseline freq_shift: {:.3} Hz", baseline);

    // Step 2: Save and set scan speed
    let original_speed = if config.tip_prep.stability.scan_speed_m_s.is_some() {
        match ctx.controller.scan_speed_get() {
            Ok(speed) => Some(speed),
            Err(e) => {
                log::warn!("Could not read scan speed: {}", e);
                None
            }
        }
    } else {
        None
    };

    if let Some(target_speed) = config.tip_prep.stability.scan_speed_m_s {
        if let Some(ref orig) = original_speed {
            let mut new_config = *orig;
            new_config.forward_linear_speed_m_s = target_speed;
            new_config.backward_linear_speed_m_s = target_speed;
            new_config.keep_parameter_constant = 1; // keep linear speed constant
            if let Err(e) = ctx.controller.scan_speed_set(new_config) {
                log::warn!("Failed to set scan speed: {}", e);
            } else {
                info!("Set scan speed to {:.2e} m/s for stability check", target_speed);
            }
        }
    }

    // Step 3: Run sweep plans
    let sweep_plans = build_sweep_plans(config);
    info!(
        "Starting stability check: {:?} polarity, {} sweep(s)",
        config.tip_prep.stability.polarity_mode,
        sweep_plans.len()
    );

    for plan in &sweep_plans {
        if shutdown.is_requested() {
            restore_scan_speed(ctx, original_speed);
            return Err("Shutdown requested".into());
        }

        prepare_for_sweep(ctx, plan, config)?;
        execute_stability_sweep(ctx, plan, config, shutdown)?;
    }

    // Step 4: Restore scan speed
    restore_scan_speed(ctx, original_speed);

    // Step 5: Measure final freq_shift
    let final_fs = measure_final_freq_shift(ctx, config, freq_shift_index)?;

    let final_fs = match final_fs {
        Some(v) => v,
        None => {
            error!("Failed to read final freq_shift");
            return Ok(StabilityOutcome::NotSharp);
        }
    };

    // Step 6: Compare
    let change = (final_fs - baseline).abs();
    let threshold = config.tip_prep.stability.stable_tip_allowed_change as f64;
    let is_stable = change <= threshold;

    info!(
        "Stability: baseline={:.3} Hz, final={:.3} Hz, change={:.3} Hz, threshold={:.3} Hz, stable={}",
        baseline, final_fs, change, threshold, is_stable
    );

    if is_stable {
        Ok(StabilityOutcome::Stable)
    } else {
        // Fire max pulse and reset to blunt
        info!("Stability failed - executing max voltage pulse");
        let max_voltage = pulse_state.signed_voltage().abs().max(
            match &config.pulse_method {
                rusty_tip::PulseMethod::Fixed { voltage, .. } => *voltage as f64,
                rusty_tip::PulseMethod::Stepping { voltage_bounds, .. } => voltage_bounds.1 as f64,
                rusty_tip::PulseMethod::Linear { voltage_bounds, .. } => voltage_bounds.1 as f64,
            },
        );
        BiasPulse {
            voltage: max_voltage,
            duration_ms: config.tip_prep.timing.pulse_width_ms,
            ..Default::default()
        }
        .execute(ctx)?;

        reposition(
            ctx,
            config.tip_prep.timing.reposition_steps,
            config.tip_prep.timing.post_reposition_settle_ms,
            config.tip_prep.timing.post_approach_settle_ms,
        )?;

        Ok(StabilityOutcome::Unstable)
    }
}

fn restore_scan_speed(ctx: &mut ActionContext, original: Option<nanonis_rs::scan::ScanConfig>) {
    if let Some(config) = original {
        if let Err(e) = ctx.controller.scan_speed_set(config) {
            log::error!("Failed to restore scan speed: {}", e);
        }
    }
}

/// Sleep in small chunks so shutdown can interrupt.
fn interruptible_sleep(
    duration: Duration,
    shutdown: &ShutdownFlag,
) -> Result<(), Box<dyn std::error::Error>> {
    let chunk = Duration::from_millis(10);
    let mut remaining = duration;
    while remaining > Duration::ZERO {
        if shutdown.is_requested() {
            return Err("Shutdown requested".into());
        }
        let sleep_for = remaining.min(chunk);
        std::thread::sleep(sleep_for);
        remaining = remaining.saturating_sub(sleep_for);
    }
    Ok(())
}

enum StabilityOutcome {
    Stable,
    NotSharp,
    Unstable,
}

/// Mutable state for pulse voltage strategies that evolve across cycles.
struct PulseState {
    current_voltage: f64,
    cycles_without_change: usize,
    last_freq_shift: Option<f64>,
    /// Base polarity from config
    base_polarity: PolaritySign,
    /// Pulse counter for random polarity switching
    pulse_count: u32,
    /// Random polarity switch config (cloned from PulseMethod)
    random_switch: Option<rusty_tip::RandomPolaritySwitch>,
}

impl PulseState {
    fn new(config: &AppConfig) -> Self {
        let (initial_voltage, base_polarity, random_switch) = match &config.pulse_method {
            rusty_tip::PulseMethod::Fixed {
                voltage,
                polarity,
                random_polarity_switch,
            } => (*voltage as f64, *polarity, random_polarity_switch.clone()),
            rusty_tip::PulseMethod::Stepping {
                voltage_bounds,
                polarity,
                random_polarity_switch,
                ..
            } => (voltage_bounds.0 as f64, *polarity, random_polarity_switch.clone()),
            rusty_tip::PulseMethod::Linear {
                voltage_bounds,
                polarity,
                random_polarity_switch,
                ..
            } => (voltage_bounds.0 as f64, *polarity, random_polarity_switch.clone()),
        };
        Self {
            current_voltage: initial_voltage,
            cycles_without_change: 0,
            last_freq_shift: None,
            base_polarity,
            pulse_count: 0,
            random_switch,
        }
    }

    /// Get the signed voltage for the next pulse.
    ///
    /// Applies polarity sign and random polarity switching to the magnitude.
    fn signed_voltage(&mut self) -> f64 {
        self.pulse_count += 1;

        let effective_polarity = if self.should_use_opposite_polarity() {
            self.base_polarity.opposite()
        } else {
            self.base_polarity
        };

        let sign = match effective_polarity {
            PolaritySign::Positive => 1.0,
            PolaritySign::Negative => -1.0,
        };

        sign * self.current_voltage
    }

    fn should_use_opposite_polarity(&self) -> bool {
        if let Some(ref switch) = self.random_switch {
            switch.enabled
                && self.pulse_count > 0
                && self.pulse_count % switch.switch_every_n_pulses == 0
        } else {
            false
        }
    }

    /// Update pulse voltage magnitude based on the latest freq_shift reading.
    fn update_voltage(&mut self, config: &AppConfig, freq_shift: Option<f64>) {
        match &config.pulse_method {
            rusty_tip::PulseMethod::Fixed { .. } => {
                // Fixed: voltage never changes
            }

            rusty_tip::PulseMethod::Stepping {
                voltage_bounds,
                voltage_steps,
                cycles_before_step,
                threshold_value,
                ..
            } => {
                let (significant, positive_change) = match (freq_shift, self.last_freq_shift) {
                    (Some(current), Some(previous)) => {
                        let change = current - previous;
                        (change.abs() > *threshold_value as f64, change >= 0.0)
                    }
                    _ => (true, true),
                };

                if significant && positive_change {
                    // Positive change: tip is improving, reset to minimum voltage
                    self.cycles_without_change = 0;
                    self.current_voltage = voltage_bounds.0 as f64;
                } else if significant {
                    // Negative significant change: increment counter
                    self.cycles_without_change += 1;
                } else {
                    // No significant change: increment counter
                    self.cycles_without_change += 1;
                }

                if self.cycles_without_change >= *cycles_before_step as usize {
                    let step_size = (voltage_bounds.1 - voltage_bounds.0) as f64
                        / *voltage_steps as f64;
                    let new_voltage =
                        (self.current_voltage + step_size).min(voltage_bounds.1 as f64);
                    if new_voltage > self.current_voltage {
                        info!(
                            "Stepping pulse voltage: {:.3}V -> {:.3}V",
                            self.current_voltage, new_voltage
                        );
                        self.current_voltage = new_voltage;
                    }
                    self.cycles_without_change = 0;
                }

                self.last_freq_shift = freq_shift;
            }

            rusty_tip::PulseMethod::Linear {
                voltage_bounds,
                linear_clamp,
                ..
            } => {
                if let Some(fs) = freq_shift {
                    if fs < linear_clamp.0 as f64 || fs > linear_clamp.1 as f64 {
                        self.current_voltage = voltage_bounds.1 as f64;
                    } else {
                        let slope = (voltage_bounds.1 - voltage_bounds.0) as f64
                            / (linear_clamp.1 - linear_clamp.0) as f64;
                        let intercept = voltage_bounds.0 as f64 - slope * linear_clamp.0 as f64;
                        self.current_voltage = slope * fs + intercept;
                    }
                }

                self.last_freq_shift = freq_shift;
            }
        }
    }
}

// ============================================================================
// Setup helpers
// ============================================================================

fn build_signal_registry(signal_names: &[String], config: &AppConfig) -> SignalRegistry {
    let mut builder = SignalRegistry::builder().with_standard_map();

    if let Some(ref mappings) = config.tcp_channel_mapping {
        let tcp_map: Vec<(u8, u8)> = mappings
            .iter()
            .map(|m| (m.nanonis_index, m.tcp_channel))
            .collect();
        builder = builder.add_tcp_map(&tcp_map);
    }

    builder
        .from_signal_names(signal_names)
        .create_aliases()
        .build()
}

fn setup_event_bus(config: &AppConfig) -> Result<EventBus, Box<dyn std::error::Error>> {
    let mut events = EventBus::new();
    events.add_observer(Box::new(ConsoleLogger));

    if config.experiment_logging.enabled {
        let log_path = create_log_file_path(&config.experiment_logging.output_path)?;
        info!("Event log: {}", log_path.display());
        let file = fs::File::create(&log_path)?;
        events.add_observer(Box::new(FileLogger::new(file)));
    }

    events.add_observer(Box::new(EventAccumulator::new(500)));

    Ok(events)
}

fn setup_shutdown_handler() -> ShutdownFlag {
    let shutdown = ShutdownFlag::new();
    let flag = shutdown.arc();

    ctrlc::set_handler(move || {
        info!("Ctrl+C received - initiating graceful shutdown...");
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    })
    .expect("Error setting Ctrl+C handler");

    shutdown
}

fn wait_for_user_confirmation() -> Result<(), Box<dyn std::error::Error>> {
    println!();
    println!("Press Enter to start tip preparation (or Ctrl+C to cancel)...");
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(())
}

fn initialize_logging(log_level: &str) -> Result<(), Box<dyn std::error::Error>> {
    let level = match log_level.to_lowercase().as_str() {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "info" => LevelFilter::Info,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        _ => {
            eprintln!("Warning: Invalid log level '{}', using 'info'", log_level);
            LevelFilter::Info
        }
    };

    env_logger::Builder::from_env(Env::default())
        .filter_level(level)
        .format_timestamp_millis()
        .init();

    Ok(())
}

fn create_log_file_path(log_path: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = PathBuf::from(log_path);
    fs::create_dir_all(&dir)?;
    let filename = format!("tip_prep_{}.jsonl", Utc::now().format("%Y%m%d_%H%M%S"));
    Ok(dir.join(filename))
}

#[cfg(windows)]
fn ensure_console_allocated() {
    unsafe {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStrExt;

        if winapi::um::consoleapi::AllocConsole() != 0 {
            println!("Console allocated for tip preparation tool");
        }

        let title = "Rusty Tip Preparation Tool (v2)";
        let wide_title: Vec<u16> = OsString::from(title)
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        winapi::um::wincon::SetConsoleTitleW(wide_title.as_ptr());

        let stdout_handle =
            winapi::um::processenv::GetStdHandle(winapi::um::winbase::STD_OUTPUT_HANDLE);
        if stdout_handle != winapi::um::handleapi::INVALID_HANDLE_VALUE {
            let mut mode: u32 = 0;
            if winapi::um::consoleapi::GetConsoleMode(stdout_handle, &mut mode) != 0 {
                mode |= winapi::um::wincon::ENABLE_VIRTUAL_TERMINAL_PROCESSING;
                winapi::um::consoleapi::SetConsoleMode(stdout_handle, mode);
            }
        }
    }
}
