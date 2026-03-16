use std::time::Duration;

use serde::Serialize;

use crate::action::bias::{BiasPulse, SetBias};
use crate::action::motor::{MoveMotor3D, Reposition};
use crate::action::scan::{ScanActionParam, ScanControl, ScanDirectionParam};
use crate::action::signals::ReadStableSignal;
use crate::action::util::Wait;
use crate::action::z_controller::{CalibratedApproach, SetZSetpoint, Withdraw};
use crate::action::{Action, ActionContext, ActionOutput, DataStore};
use crate::config::{AppConfig, TipPrepConfig};
use crate::controller_types::{BiasSweepPolarity, PulseMethod};
use crate::event::{Event, EventBus};
use crate::spm_controller::SpmController;
use crate::spm_error::SpmError;
use crate::workflow::ShutdownFlag;

use super::PulseState;

// ============================================================================
// Public types
// ============================================================================

/// Snapshot of tip-prep state for GUI/observer consumption.
#[derive(Serialize, Clone, Debug)]
pub struct TipPrepSnapshot {
    pub cycle: usize,
    pub elapsed_secs: f64,
    pub freq_shift: Option<f64>,
    pub pulse_voltage: f64,
    pub is_sharp: bool,
    pub phase: &'static str,
}

/// Final outcome of a tip preparation run.
pub enum Outcome {
    Completed,
    StoppedByUser,
    CycleLimit(usize),
    TimedOut(Duration),
}

// ============================================================================
// Internal types
// ============================================================================

enum StabilityOutcome {
    Stable,
    NotSharp,
    Unstable,
}

struct SweepPlan {
    starting_bias: f64,
    bias_range: (f64, f64),
    index: usize,
    total: usize,
}

// ============================================================================
// Entry point
// ============================================================================

/// Run the full tip preparation algorithm.
///
/// This is the top-level entry point that owns the controller lifecycle
/// (prepare/teardown). It calls `run_tip_prep_inner()` for the main loop,
/// then always cleans up regardless of the outcome.
pub fn run_tip_prep(
    mut controller: Box<dyn SpmController>,
    events: &EventBus,
    shutdown: &ShutdownFlag,
    config: &AppConfig,
    freq_shift_index: u32,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    controller.prepare()?;

    let result = run_tip_prep_inner(
        &mut *controller,
        events,
        shutdown,
        config,
        freq_shift_index,
    );

    log::info!("Cleanup starting...");
    cleanup(&mut *controller, events);
    log::info!("Cleanup complete");

    result
}

// ============================================================================
// Core algorithm
// ============================================================================

fn run_tip_prep_inner(
    controller: &mut dyn SpmController,
    events: &EventBus,
    shutdown: &ShutdownFlag,
    config: &AppConfig,
    freq_shift_index: u32,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    let mut store = DataStore::new();

    // Pre-loop initialization: bias, setpoint, approach, buffer clear
    {
        let mut ctx = ActionContext {
            controller,
            store: &mut store,
            events,
        };

        log::info!("Initializing...");
        execute_logged(
            &SetBias {
                voltage: config.tip_prep.initial_bias_v as f64,
            },
            &mut ctx,
        )?;

        execute_logged(
            &SetZSetpoint {
                setpoint: config.tip_prep.initial_z_setpoint_a as f64,
            },
            &mut ctx,
        )?;

        execute_logged(&CalibratedApproach::default(), &mut ctx)?;
    }

    // Clear TCP buffer to discard stale pre-approach data
    controller.clear_data_buffer();
    {
        let mut ctx = ActionContext {
            controller,
            store: &mut store,
            events,
        };
        execute_logged(
            &Wait {
                duration_ms: config.tip_prep.timing.buffer_clear_wait_ms,
            },
            &mut ctx,
        )?;

        execute_logged(
            &Wait {
                duration_ms: config.tip_prep.timing.post_approach_settle_ms,
            },
            &mut ctx,
        )?;
    }

    let bounds = (
        config.tip_prep.sharp_tip_bounds[0] as f64,
        config.tip_prep.sharp_tip_bounds[1] as f64,
    );

    let max_cycles = config.tip_prep.max_cycles.unwrap_or(usize::MAX);
    let max_duration = config.tip_prep.max_duration_secs.map(Duration::from_secs);
    let start_time = std::time::Instant::now();
    let mut pulse_state = PulseState::new(&config.pulse_method);

    let mut ctx = ActionContext {
        controller,
        store: &mut store,
        events,
    };

    // Check if tip is already sharp after initial approach
    let initial_fs = read_stable(&mut ctx, config, freq_shift_index)?;
    let initial_sharp = initial_fs >= bounds.0 && initial_fs <= bounds.1;
    log::info!(
        "Initial tip state: freq_shift={:.3} Hz, sharp={}",
        initial_fs,
        initial_sharp
    );

    if initial_sharp {
        log::info!("Tip already sharp after approach - running stability check");
        match check_stability(
            &mut ctx,
            freq_shift_index,
            bounds,
            config,
            shutdown,
            &mut pulse_state,
        )? {
            StabilityOutcome::Stable => {
                log::info!("Tip confirmed stable!");
                return Ok(Outcome::Completed);
            }
            StabilityOutcome::NotSharp => {
                log::info!("Initial sharp not confirmed - entering pulse loop");
            }
            StabilityOutcome::Unstable => {
                log::info!("Initial sharp unstable - entering pulse loop");
                pulse_state.reset(&config.pulse_method);
            }
        }
    }

    // Main loop: pulse -> settle -> reposition -> measure -> check sharp
    // Matches V1 ordering: minimize time at pulsed position to avoid
    // unintended tip changes from continued surface interaction.
    for cycle in 1..=max_cycles {
        if shutdown.is_requested() {
            return Ok(Outcome::StoppedByUser);
        }

        if let Some(max_dur) = max_duration {
            if start_time.elapsed() > max_dur {
                return Ok(Outcome::TimedOut(max_dur));
            }
        }

        if cycle % config.tip_prep.timing.status_interval == 0 {
            log::info!(
                "Status: cycle={}, pulse_v={:.2}V, elapsed={:.1}s",
                cycle,
                pulse_state.current_voltage,
                start_time.elapsed().as_secs_f64()
            );
        }

        // Pulse with current voltage (determined by previous cycle's update)
        let pulse_voltage = pulse_state.signed_voltage();
        log::info!(
            "Executing pulse #{}: {:.3}V ({} method, {:?}{})",
            pulse_state.pulse_count,
            pulse_voltage,
            config.pulse_method.method_name(),
            pulse_state.base_polarity,
            if pulse_state.should_use_opposite_polarity() {
                " - SWITCHED"
            } else {
                ""
            }
        );
        execute_logged(
            &BiasPulse {
                voltage: pulse_voltage,
                duration_ms: config.tip_prep.timing.pulse_width_ms,
                ..Default::default()
            },
            &mut ctx,
        )?;

        execute_logged(
            &Wait {
                duration_ms: config.tip_prep.timing.post_pulse_settle_ms,
            },
            &mut ctx,
        )?;

        // Reposition immediately: get away from pulse site
        execute_logged(
            &Reposition {
                x_steps: config.tip_prep.timing.reposition_steps[0],
                y_steps: config.tip_prep.timing.reposition_steps[1],
                post_move_settle_ms: config.tip_prep.timing.post_reposition_settle_ms,
                post_approach_settle_ms: config.tip_prep.timing.post_approach_settle_ms,
                ..Default::default()
            },
            &mut ctx,
        )?;

        // Measure at new position (after reposition)
        let freq_shift = read_stable(&mut ctx, config, freq_shift_index)?;
        let is_sharp = freq_shift >= bounds.0 && freq_shift <= bounds.1;

        // Emit state snapshot for GUI observers
        ctx.events.emit(Event::custom(
            "tip_prep_state",
            serde_json::to_value(&TipPrepSnapshot {
                cycle,
                elapsed_secs: start_time.elapsed().as_secs_f64(),
                freq_shift: Some(freq_shift),
                pulse_voltage: pulse_state.current_voltage,
                is_sharp,
                phase: "pulsing",
            })
            .unwrap_or_default(),
        ));

        if is_sharp {
            log::info!(
                "Tip sharp at cycle {} (freq_shift={:.3} Hz)",
                cycle,
                freq_shift
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
                    log::info!("Tip confirmed stable!");
                    return Ok(Outcome::Completed);
                }
                StabilityOutcome::NotSharp => {
                    log::info!("Tip not confirmed sharp - continuing");
                }
                StabilityOutcome::Unstable => {
                    log::info!("Stability check failed - reset to blunt, continuing");
                    pulse_state.reset(&config.pulse_method);
                }
            }
        }

        // Update voltage strategy for next cycle (uses post-reposition measurement)
        pulse_state.update_voltage(&config.pulse_method, Some(freq_shift));
    }

    Ok(Outcome::CycleLimit(max_cycles))
}

// ============================================================================
// Cleanup
// ============================================================================

fn cleanup(controller: &mut dyn SpmController, events: &EventBus) {
    // Withdraw first while safe-tip protection is still active
    let mut store = DataStore::new();
    let mut ctx = ActionContext {
        controller,
        store: &mut store,
        events,
    };
    if let Err(e) = execute_logged(&Withdraw::default(), &mut ctx) {
        log::warn!("Cleanup withdrawal failed: {}", e);
    }

    controller.teardown();
}

// ============================================================================
// Confirm sharpness
// ============================================================================

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
            return Err(SpmError::ShutdownRequested.into());
        }

        execute_logged(
            &Reposition {
                x_steps: config.tip_prep.timing.reposition_steps[0],
                y_steps: config.tip_prep.timing.reposition_steps[1],
                post_move_settle_ms: config.tip_prep.timing.post_reposition_settle_ms,
                post_approach_settle_ms: config.tip_prep.timing.post_approach_settle_ms,
                ..Default::default()
            },
            ctx,
        )?;

        if shutdown.is_requested() {
            return Err(SpmError::ShutdownRequested.into());
        }

        let fs = read_stable(ctx, config, freq_shift_index)?;
        let in_bounds = fs >= bounds.0 && fs <= bounds.1;
        log::info!(
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

    Ok((true, last_freq_shift))
}

// ============================================================================
// Stability check
// ============================================================================

fn check_stability(
    ctx: &mut ActionContext,
    freq_shift_index: u32,
    bounds: (f64, f64),
    config: &AppConfig,
    shutdown: &ShutdownFlag,
    pulse_state: &mut PulseState,
) -> Result<StabilityOutcome, Box<dyn std::error::Error>> {
    ctx.events.emit(Event::custom(
        "tip_prep_state",
        serde_json::json!({ "phase": "confirming" }),
    ));

    // Step 1: Confirm sharpness with repositioning (3 reads)
    let (confirmed, baseline) =
        confirm_sharp(ctx, freq_shift_index, bounds, config, shutdown)?;

    if !confirmed {
        log::info!("Tip not confirmed sharp during pre-check");
        return Ok(StabilityOutcome::NotSharp);
    }

    if !config.tip_prep.stability.check_stability {
        log::info!("Stability checking disabled - accepting sharp tip");
        return Ok(StabilityOutcome::Stable);
    }

    let baseline = match baseline {
        Some(v) => v,
        None => {
            log::error!("No baseline freq_shift available");
            return Ok(StabilityOutcome::NotSharp);
        }
    };

    log::info!("Baseline freq_shift: {:.3} Hz", baseline);

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
            new_config.keep_parameter_constant = 1;
            if let Err(e) = ctx.controller.scan_speed_set(new_config) {
                log::warn!("Failed to set scan speed: {}", e);
            } else {
                log::info!(
                    "Set scan speed to {:.2e} m/s for stability check",
                    target_speed
                );
            }
        }
    }

    // Step 3: Run sweep plans
    let sweep_plans = build_sweep_plans(&config.tip_prep, &config.pulse_method);

    ctx.events.emit(Event::custom(
        "tip_prep_state",
        serde_json::json!({ "phase": "stability_check", "baseline_freq_shift": baseline }),
    ));

    log::info!(
        "Starting stability check: {:?} polarity, {} sweep(s)",
        config.tip_prep.stability.polarity_mode,
        sweep_plans.len()
    );

    for plan in &sweep_plans {
        if shutdown.is_requested() {
            restore_scan_speed(ctx, original_speed);
            return Err(SpmError::ShutdownRequested.into());
        }

        prepare_for_sweep(ctx, plan, &config.tip_prep)?;
        execute_stability_sweep(ctx, plan, &config.tip_prep, shutdown)?;
    }

    // Step 4: Restore scan speed
    restore_scan_speed(ctx, original_speed);

    // Step 5: Measure final freq_shift
    let final_fs = measure_final_freq_shift(
        ctx,
        config,
        freq_shift_index,
    )?;

    let final_fs = match final_fs {
        Some(v) => v,
        None => {
            log::error!("Failed to read final freq_shift");
            return Ok(StabilityOutcome::NotSharp);
        }
    };

    // Step 6: Compare
    let change = (final_fs - baseline).abs();
    let threshold = config.tip_prep.stability.stable_tip_allowed_change as f64;
    let is_stable = change <= threshold;

    log::info!(
        "Stability: baseline={:.3} Hz, final={:.3} Hz, change={:.3} Hz, threshold={:.3} Hz, stable={}",
        baseline, final_fs, change, threshold, is_stable
    );

    if is_stable {
        ctx.events.emit(Event::custom(
            "tip_prep_state",
            serde_json::json!({ "phase": "stable", "final_freq_shift": final_fs }),
        ));
        Ok(StabilityOutcome::Stable)
    } else {
        // Fire max pulse and reset to blunt
        let signed_max = pulse_state.fire_max_pulse_voltage(&config.pulse_method);
        log::info!(
            "Executing MAX pulse #{} due to stability failure: {:.3}V ({:?})",
            pulse_state.pulse_count,
            signed_max,
            pulse_state.base_polarity,
        );
        execute_logged(
            &BiasPulse {
                voltage: signed_max,
                duration_ms: config.tip_prep.timing.pulse_width_ms,
                ..Default::default()
            },
            ctx,
        )?;

        execute_logged(
            &Reposition {
                x_steps: config.tip_prep.timing.reposition_steps[0],
                y_steps: config.tip_prep.timing.reposition_steps[1],
                post_move_settle_ms: config.tip_prep.timing.post_reposition_settle_ms,
                post_approach_settle_ms: config.tip_prep.timing.post_approach_settle_ms,
                ..Default::default()
            },
            ctx,
        )?;

        Ok(StabilityOutcome::Unstable)
    }
}

// ============================================================================
// Stability sweep helpers
// ============================================================================

fn build_sweep_plans(tip_prep: &TipPrepConfig, _method: &PulseMethod) -> Vec<SweepPlan> {
    let sc = &tip_prep.stability;
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

fn prepare_for_sweep(
    ctx: &mut ActionContext,
    plan: &SweepPlan,
    tip_prep: &TipPrepConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    execute_logged(&Withdraw::default(), ctx)?;

    execute_logged(
        &MoveMotor3D {
            x: tip_prep.timing.reposition_steps[0],
            y: tip_prep.timing.reposition_steps[1],
            z: -3,
            wait: true,
        },
        ctx,
    )?;

    execute_logged(&Wait { duration_ms: 200 }, ctx)?;

    execute_logged(
        &SetBias {
            voltage: plan.starting_bias,
        },
        ctx,
    )?;

    execute_logged(&CalibratedApproach::default(), ctx)?;

    execute_logged(
        &Wait {
            duration_ms: tip_prep.timing.post_approach_settle_ms,
        },
        ctx,
    )?;

    Ok(())
}

fn execute_stability_sweep(
    ctx: &mut ActionContext,
    plan: &SweepPlan,
    tip_prep: &TipPrepConfig,
    shutdown: &ShutdownFlag,
) -> Result<(), Box<dyn std::error::Error>> {
    let sc = &tip_prep.stability;

    log::info!(
        "Sweep {}/{}: bias {:.2}V -> {:.2}V",
        plan.index,
        plan.total,
        plan.bias_range.0,
        plan.bias_range.1
    );

    // Configure scan for stability check: continuous + bouncy
    use nanonis_rs::scan::ScanPropsBuilder;
    let original_props = ctx.controller.scan_props_get()?;
    ctx.controller.scan_props_set(
        ScanPropsBuilder::new()
            .continuous_scan(true)
            .bouncy_scan(true),
    )?;

    // Run the sweep in an inner function so we can unconditionally clean up
    let result = execute_stability_sweep_inner(ctx, plan, sc, shutdown);

    // Always stop scan and restore properties, regardless of how the sweep ended
    let _ = ScanControl {
        action: ScanActionParam::Stop,
        direction: ScanDirectionParam::Up,
    }
    .execute(ctx);
    restore_scan_props(ctx, &original_props);

    // Propagate the inner result; on success, do the post-sweep safety sequence
    result?;

    // Withdraw before changing bias (tip is still on surface after sweep)
    execute_logged(&Withdraw::default(), ctx)?;
    execute_logged(&Wait { duration_ms: 200 }, ctx)?;

    // Restore bias to sweep starting value (not the last stepped value near 0V)
    execute_logged(
        &SetBias {
            voltage: plan.starting_bias,
        },
        ctx,
    )?;

    Ok(())
}

/// Inner sweep loop — separated so the caller can guarantee scan stop + props restore.
fn execute_stability_sweep_inner(
    ctx: &mut ActionContext,
    plan: &SweepPlan,
    sc: &crate::controller_types::StabilityConfig,
    shutdown: &ShutdownFlag,
) -> Result<(), Box<dyn std::error::Error>> {
    // Start scan
    execute_logged(
        &ScanControl {
            action: ScanActionParam::Start,
            direction: ScanDirectionParam::Down,
        },
        ctx,
    )?;

    // Wait for scan to actually start (max 5 seconds)
    let mut scan_started = false;
    for _ in 0..50 {
        if shutdown.is_requested() {
            return Err(SpmError::ShutdownRequested.into());
        }
        std::thread::sleep(Duration::from_millis(100));
        if ctx.controller.scan_status()? {
            scan_started = true;
            break;
        }
    }

    if !scan_started {
        return Err("Scan failed to start within 5 seconds".into());
    }

    // Step bias through range
    let bias_step_size =
        (plan.bias_range.1 - plan.bias_range.0) / sc.bias_steps as f64;
    let mut current_bias = plan.bias_range.0;
    let step_duration = Duration::from_millis(sc.step_period_ms);

    for step in 0..sc.bias_steps {
        if shutdown.is_requested() {
            return Err(SpmError::ShutdownRequested.into());
        }

        execute_logged(&SetBias { voltage: current_bias }, ctx)?;

        log::debug!(
            "Step {}/{}: bias={:.3}V",
            step + 1,
            sc.bias_steps,
            current_bias
        );

        interruptible_sleep(step_duration, shutdown)?;

        current_bias += bias_step_size;
    }

    log::info!("Bias sweep completed");
    Ok(())
}

fn restore_scan_props(
    ctx: &mut ActionContext,
    original: &nanonis_rs::scan::ScanProps,
) {
    let builder = original.to_builder();
    if let Err(e) = ctx.controller.scan_props_set(builder) {
        log::error!("Failed to restore scan properties: {}", e);
    }
}

fn measure_final_freq_shift(
    ctx: &mut ActionContext,
    config: &AppConfig,
    freq_shift_index: u32,
) -> Result<Option<f64>, Box<dyn std::error::Error>> {
    log::info!("Measuring final freq_shift after sweeps");

    execute_logged(&Withdraw::default(), ctx)?;
    execute_logged(&Wait { duration_ms: 200 }, ctx)?;

    execute_logged(
        &SetBias {
            voltage: config.tip_prep.initial_bias_v as f64,
        },
        ctx,
    )?;

    execute_logged(&CalibratedApproach::default(), ctx)?;

    execute_logged(
        &Wait {
            duration_ms: config.tip_prep.timing.post_approach_settle_ms,
        },
        ctx,
    )?;

    let fs = read_stable(ctx, config, freq_shift_index)?;
    Ok(Some(fs))
}

fn restore_scan_speed(
    ctx: &mut ActionContext,
    original: Option<nanonis_rs::scan::ScanConfig>,
) {
    if let Some(config) = original {
        if let Err(e) = ctx.controller.scan_speed_set(config) {
            log::error!("Failed to restore scan speed: {}", e);
        }
    }
}

// ============================================================================
// Utilities
// ============================================================================

/// Sleep in small chunks so shutdown can interrupt.
pub fn interruptible_sleep(
    duration: Duration,
    shutdown: &ShutdownFlag,
) -> Result<(), Box<dyn std::error::Error>> {
    let chunk = Duration::from_millis(10);
    let mut remaining = duration;
    while remaining > Duration::ZERO {
        if shutdown.is_requested() {
            return Err(SpmError::ShutdownRequested.into());
        }
        let sleep_for = remaining.min(chunk);
        std::thread::sleep(sleep_for);
        remaining = remaining.saturating_sub(sleep_for);
    }
    Ok(())
}

/// Execute an action with event logging (start/complete/fail events).
pub fn execute_logged(
    action: &dyn Action,
    ctx: &mut ActionContext,
) -> Result<ActionOutput, SpmError> {
    let name = action.name().to_string();
    let start = std::time::Instant::now();
    ctx.events
        .emit(Event::action_started(&name, serde_json::json!({})));
    match action.execute(ctx) {
        Ok(output) => {
            ctx.events
                .emit(Event::action_completed(&name, &output, start.elapsed()));
            Ok(output)
        }
        Err(e) => {
            ctx.events
                .emit(Event::action_failed(&name, &e.to_string(), start.elapsed()));
            Err(e)
        }
    }
}

/// Build a ReadStableSignal action from config and execute it, returning the f64 value.
fn read_stable(
    ctx: &mut ActionContext,
    config: &AppConfig,
    freq_shift_index: u32,
) -> Result<f64, Box<dyn std::error::Error>> {
    let output = execute_logged(
        &ReadStableSignal {
            index: freq_shift_index,
            num_samples: config.data_acquisition.stable_signal_samples,
            max_std_dev: config.data_acquisition.max_std_dev,
            max_slope: config.data_acquisition.max_slope,
            max_retries: config.data_acquisition.stable_read_retries,
        },
        ctx,
    )?;
    match output {
        ActionOutput::Value(v) => Ok(v),
        other => Err(format!("ReadStableSignal returned unexpected output: {:?}", other).into()),
    }
}
