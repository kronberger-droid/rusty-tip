use std::time::Duration;

use serde::Serialize;

use crate::action::scan::ScanDirectionParam;
use crate::config::{AppConfig, TipPrepConfig};
use crate::controller_types::{BiasSweepPolarity, PolaritySign};
use crate::event::{Event, EventBus};
use crate::routine::{RepositionSpec, Routine, Rt, StableReadSpec, run_routine};
use crate::shutdown::ShutdownFlag;
use crate::signal_registry::SignalIndex;
use crate::spm_controller::SpmController;
use crate::spm_error::SpmError;

use nanonis_rs::scan::ScanPropsBuilder;

use super::PulseState;

pub use crate::routine::Outcome;

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

/// Everything a tip-preparation run needs besides the controller.
pub struct TipPrepParams<'a> {
    /// Event sink for observers (console, file log, GUI).
    pub events: &'a EventBus,
    /// Cooperative cancellation flag (Ctrl+C, GUI stop button).
    pub shutdown: &'a ShutdownFlag,
    pub config: &'a AppConfig,
    /// The frequency-shift signal driving sharpness decisions.
    pub freq_shift: SignalIndex,
}

/// Run the full tip preparation algorithm.
///
/// Convenience wrapper that builds a [`TipPrep`] routine and hands it to
/// [`run_routine`], which owns the controller life cycle (prepare, withdraw
/// on exit, teardown). A shutdown request (Ctrl+C, GUI stop) is an expected
/// way for a run to end, so it surfaces as `Ok(Outcome::StoppedByUser)`,
/// never as an error.
pub fn run_tip_prep(
    controller: Box<dyn SpmController>,
    params: TipPrepParams<'_>,
) -> Result<Outcome, SpmError> {
    let TipPrepParams {
        events,
        shutdown,
        config,
        freq_shift,
    } = params;
    let mut routine = TipPrep::new(config, freq_shift);
    run_routine(controller, events, shutdown, &mut routine)
}

// ============================================================================
// The routine
// ============================================================================

/// The tip-preparation routine: pulse, reposition, measure the frequency
/// shift, repeat until the tip is sharp and provably stable.
///
/// The reference implementation of [`Routine`]; see
/// `docs/tip-prep/algorithm.md` for the cycle-by-cycle description.
pub struct TipPrep<'a> {
    config: &'a AppConfig,
    freq_shift: SignalIndex,
    pulse: PulseState,
    bounds: (f64, f64),
    read_spec: StableReadSpec,
}

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

impl<'a> TipPrep<'a> {
    pub fn new(config: &'a AppConfig, freq_shift: SignalIndex) -> Self {
        let gates = &config.tip_prep.signal_stability;
        Self {
            config,
            freq_shift,
            pulse: PulseState::new(&config.pulse_method),
            bounds: (
                config.tip_prep.sharp_tip_bounds[0],
                config.tip_prep.sharp_tip_bounds[1],
            ),
            read_spec: StableReadSpec {
                num_samples: config.data_acquisition.stable_signal_samples,
                max_std_dev: gates.max_std_dev_hz,
                max_slope: gates.max_slope_hz_per_s,
                max_retries: gates.read_retry_count as usize,
                sample_rate_hz: config.data_acquisition.sample_rate as f64,
            },
        }
    }

    fn is_sharp(&self, freq_shift: f64) -> bool {
        freq_shift >= self.bounds.0 && freq_shift <= self.bounds.1
    }

    fn read_stable(&self, rt: &mut Rt) -> Result<f64, SpmError> {
        rt.signals()?.read_stable(self.freq_shift, &self.read_spec)
    }

    /// Move to a fresh surface spot: withdraw, step the motors, re-approach.
    ///
    /// V1 parity: `post_move_settle_ms` sits between the motor move and the
    /// approach; `post_reposition_settle_ms` ends the whole reposition.
    fn reposition(&self, rt: &mut Rt) -> Result<(), SpmError> {
        let t = &self.config.tip_prep.timing;
        rt.motor()?.reposition(&RepositionSpec {
            x_steps: t.reposition_steps[0],
            y_steps: t.reposition_steps[1],
            post_move_settle_ms: t.post_move_settle_ms,
            post_approach_settle_ms: t.post_reposition_settle_ms,
            ..Default::default()
        })
    }

    fn handle_stability(&mut self, rt: &mut Rt) -> Result<bool, SpmError> {
        match self.check_stability(rt)? {
            StabilityOutcome::Stable => {
                log::info!("Tip confirmed stable!");
                Ok(true)
            }
            StabilityOutcome::NotSharp => {
                log::info!("Tip not confirmed sharp - continuing");
                Ok(false)
            }
            StabilityOutcome::Unstable => {
                log::info!("Stability check failed - reset to blunt, continuing");
                self.pulse.reset(&self.config.pulse_method);
                Ok(false)
            }
        }
    }

    // ------------------------------------------------------------------
    // Confirm sharpness
    // ------------------------------------------------------------------

    fn confirm_sharp(&self, rt: &mut Rt) -> Result<(bool, Option<f64>), SpmError> {
        const CONFIRMATION_READS: usize = 3;
        let mut last_freq_shift = None;

        for i in 0..CONFIRMATION_READS {
            rt.check_shutdown()?;
            self.reposition(rt)?;
            rt.check_shutdown()?;

            let fs = self.read_stable(rt)?;
            let in_bounds = self.is_sharp(fs);
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

    // ------------------------------------------------------------------
    // Stability check
    // ------------------------------------------------------------------

    fn check_stability(&mut self, rt: &mut Rt) -> Result<StabilityOutcome, SpmError> {
        rt.emit(Event::custom(
            "tip_prep_state",
            serde_json::json!({ "phase": "confirming" }),
        ));

        // Step 1: Confirm sharpness with repositioning (3 reads)
        let (confirmed, baseline) = self.confirm_sharp(rt)?;

        if !confirmed {
            log::info!("Tip not confirmed sharp during pre-check");
            return Ok(StabilityOutcome::NotSharp);
        }

        let stability = &self.config.tip_prep.stability;
        if !stability.check_stability {
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
        let original_speed = if stability.scan_speed_m_s.is_some() {
            match rt.scan()?.speed_get() {
                Ok(speed) => Some(speed),
                Err(e) => {
                    log::warn!("Could not read scan speed: {}", e);
                    None
                }
            }
        } else {
            None
        };

        if let Some(target_speed) = stability.scan_speed_m_s
            && let Some(ref orig) = original_speed
        {
            let mut new_config = *orig;
            // ScanConfig is the nanonis-rs wire format, which carries f32 speeds.
            new_config.forward_linear_speed_m_s = target_speed as f32;
            new_config.backward_linear_speed_m_s = target_speed as f32;
            new_config.keep_parameter_constant = 1;
            if let Err(e) = rt.scan()?.speed_set(new_config) {
                log::warn!("Failed to set scan speed: {}", e);
            } else {
                log::info!(
                    "Set scan speed to {:.2e} m/s for stability check",
                    target_speed
                );
            }
        }

        // Step 3: Run sweep plans, restoring the scan speed however they end
        let sweep_plans = build_sweep_plans(&self.config.tip_prep);

        rt.emit(Event::custom(
            "tip_prep_state",
            serde_json::json!({ "phase": "stability_check", "baseline_freq_shift": baseline }),
        ));

        log::info!(
            "Starting stability check: {:?} polarity, {} sweep(s)",
            stability.polarity_mode,
            sweep_plans.len()
        );

        rt.guarded(
            |rt| {
                for plan in &sweep_plans {
                    rt.check_shutdown()?;
                    self.prepare_for_sweep(rt, plan)?;
                    self.execute_stability_sweep(rt, plan)?;
                }
                Ok(())
            },
            |rt| {
                if let Some(config) = original_speed
                    && let Err(e) = rt.scan().and_then(|mut s| s.speed_set(config))
                {
                    log::error!("Failed to restore scan speed: {}", e);
                }
                Ok(())
            },
        )?;

        // Step 4: Measure final freq_shift
        let final_fs = self.measure_final_freq_shift(rt)?;

        // Step 5: Compare
        let change = (final_fs - baseline).abs();
        let threshold = stability.stable_tip_allowed_change;
        let is_stable = change <= threshold;

        log::info!(
            "Stability: baseline={:.3} Hz, final={:.3} Hz, change={:.3} Hz, threshold={:.3} Hz, stable={}",
            baseline,
            final_fs,
            change,
            threshold,
            is_stable
        );

        if is_stable {
            rt.emit(Event::custom(
                "tip_prep_state",
                serde_json::json!({ "phase": "stable", "final_freq_shift": final_fs }),
            ));
            Ok(StabilityOutcome::Stable)
        } else {
            // The post-sweep withdraw in execute_stability_sweep only logs
            // errors; re-withdraw here with error propagation so a max-voltage
            // pulse never fires on an engaged tip if the earlier withdraw
            // silently failed.
            rt.z()?.withdraw()?;

            // Fire max pulse and reset to blunt. `fire_max_pulse_voltage` bumps
            // pulse_count and may flip polarity, so capture the effective sign
            // from the returned voltage rather than re-reading base_polarity.
            let signed_max = self.pulse.fire_max_pulse_voltage(&self.config.pulse_method);
            let effective_polarity = if signed_max >= 0.0 {
                PolaritySign::Positive
            } else {
                PolaritySign::Negative
            };
            log::info!(
                "Executing MAX pulse #{} due to stability failure: {:.3}V ({:?}{})",
                self.pulse.pulse_count,
                signed_max,
                effective_polarity,
                if effective_polarity != self.pulse.base_polarity {
                    " - SWITCHED"
                } else {
                    ""
                },
            );
            rt.bias()?
                .pulse(signed_max, self.config.tip_prep.timing.pulse_width_ms)?;

            self.reposition(rt)?;

            Ok(StabilityOutcome::Unstable)
        }
    }

    // ------------------------------------------------------------------
    // Stability sweep
    // ------------------------------------------------------------------

    fn prepare_for_sweep(&self, rt: &mut Rt, plan: &SweepPlan) -> Result<(), SpmError> {
        let t = &self.config.tip_prep.timing;

        rt.z()?.withdraw()?;
        rt.motor()?
            .move_3d(t.reposition_steps[0], t.reposition_steps[1], -3)?;
        rt.settle(200)?;
        rt.bias()?.set(plan.starting_bias)?;
        rt.z()?.calibrated_approach()?;
        rt.settle(t.post_approach_settle_ms)?;

        Ok(())
    }

    fn execute_stability_sweep(&self, rt: &mut Rt, plan: &SweepPlan) -> Result<(), SpmError> {
        log::info!(
            "Sweep {}/{}: bias {:.2}V -> {:.2}V",
            plan.index,
            plan.total,
            plan.bias_range.0,
            plan.bias_range.1
        );

        // Configure scan for stability check: continuous + bouncy
        let original_props = rt.scan()?.props_get()?;
        rt.scan()?.props_set(
            ScanPropsBuilder::new()
                .continuous_scan(true)
                .bouncy_scan(true),
        )?;

        rt.guarded(
            |rt| self.sweep_inner(rt, plan),
            // Always stop the scan, restore properties and withdraw — the tip
            // is on the surface after the sweep whether it completed or was
            // interrupted, and none of this may shadow the sweep's own error.
            |rt| {
                match rt.scan() {
                    Ok(mut scan) => {
                        let _ = scan.stop();
                        if let Err(e) = scan.props_set(original_props.to_builder()) {
                            log::error!("Failed to restore scan properties: {}", e);
                        }
                    }
                    Err(e) => log::error!("Post-sweep scan cleanup skipped: {}", e),
                }
                if let Err(e) = rt.z().and_then(|mut z| z.withdraw()) {
                    log::error!("Post-sweep withdraw failed: {}", e);
                }
                Ok(())
            },
        )?;

        rt.settle(200)?;

        // Restore bias to sweep starting value (not the last stepped value near 0V)
        rt.bias()?.set(plan.starting_bias)?;

        Ok(())
    }

    fn sweep_inner(&self, rt: &mut Rt, plan: &SweepPlan) -> Result<(), SpmError> {
        let sc = &self.config.tip_prep.stability;

        rt.scan()?.start(ScanDirectionParam::Down)?;

        // Wait for the scan to actually start (max 5 seconds)
        let mut scan_started = false;
        for _ in 0..50 {
            rt.settle(100)?;
            if rt.scan()?.status()? {
                scan_started = true;
                break;
            }
        }

        if !scan_started {
            return Err(SpmError::Timeout(
                "scan failed to start within 5 seconds".into(),
            ));
        }

        // Step bias through range
        let bias_step_size = (plan.bias_range.1 - plan.bias_range.0) / sc.bias_steps as f64;
        let mut current_bias = plan.bias_range.0;

        for step in 0..sc.bias_steps {
            rt.check_shutdown()?;
            rt.bias()?.set(current_bias)?;

            log::debug!(
                "Step {}/{}: bias={:.3}V",
                step + 1,
                sc.bias_steps,
                current_bias
            );

            rt.settle(sc.step_period_ms)?;
            current_bias += bias_step_size;
        }

        log::info!("Bias sweep completed");
        Ok(())
    }

    fn measure_final_freq_shift(&self, rt: &mut Rt) -> Result<f64, SpmError> {
        log::info!("Measuring final freq_shift after sweeps");

        rt.z()?.withdraw()?;
        rt.settle(200)?;
        rt.bias()?.set(self.config.tip_prep.initial_bias_v)?;
        rt.z()?.calibrated_approach()?;
        rt.settle(self.config.tip_prep.timing.post_approach_settle_ms)?;

        self.read_stable(rt)
    }
}

impl Routine for TipPrep<'_> {
    fn name(&self) -> &str {
        "tip_prep"
    }

    fn run(&mut self, rt: &mut Rt) -> Result<Outcome, SpmError> {
        let cfg = self.config;
        let timing = &cfg.tip_prep.timing;

        log::info!("Initializing...");
        rt.bias()?.set(cfg.tip_prep.initial_bias_v)?;
        rt.z()?.set_setpoint(cfg.tip_prep.initial_z_setpoint_a)?;
        rt.z()?.calibrated_approach()?;

        // Clear the stream buffer to discard stale pre-approach data
        rt.signals()?.clear_buffer();
        rt.settle(timing.buffer_clear_wait_ms)?;
        rt.settle(timing.post_approach_settle_ms)?;

        // The budget clock starts here, before the initial measurement, so
        // an initial stability check already counts against max_duration.
        let mut cycles = rt.cycles(
            cfg.tip_prep.max_cycles,
            cfg.tip_prep.max_duration_secs.map(Duration::from_secs),
        );

        // Check if tip is already sharp after initial approach
        let initial_fs = self.read_stable(rt)?;
        let initial_sharp = self.is_sharp(initial_fs);
        log::info!(
            "Initial tip state: freq_shift={:.3} Hz, sharp={}",
            initial_fs,
            initial_sharp
        );

        if initial_sharp {
            log::info!("Tip already sharp after approach - running stability check");
            if self.handle_stability(rt)? {
                return Ok(Outcome::Completed);
            }
        }

        // Main loop: pulse -> settle -> reposition -> measure -> check sharp
        // Matches V1 ordering: minimize time at pulsed position to avoid
        // unintended tip changes from continued surface interaction.
        while let Some(cycle) = cycles.next() {
            if cycle % timing.status_interval == 0 {
                log::info!(
                    "Status: cycle={}, pulse_v={:.2}V, elapsed={:.1}s",
                    cycle,
                    self.pulse.current_voltage,
                    cycles.elapsed().as_secs_f64()
                );
            }

            // Pulse with current voltage (determined by previous cycle's update)
            let pulse_voltage = self.pulse.signed_voltage();
            log::info!(
                "Executing pulse #{}: {:.3}V ({} method, {:?}{})",
                self.pulse.pulse_count,
                pulse_voltage,
                cfg.pulse_method.method_name(),
                self.pulse.base_polarity,
                if self.pulse.should_use_opposite_polarity() {
                    " - SWITCHED"
                } else {
                    ""
                }
            );
            rt.bias()?.pulse(pulse_voltage, timing.pulse_width_ms)?;
            rt.settle(timing.post_pulse_settle_ms)?;

            // Reposition immediately: get away from pulse site
            self.reposition(rt)?;

            // Measure at new position (after reposition)
            let freq_shift = self.read_stable(rt)?;
            let is_sharp = self.is_sharp(freq_shift);

            // Emit state snapshot for GUI observers
            rt.emit(Event::custom(
                "tip_prep_state",
                serde_json::to_value(&TipPrepSnapshot {
                    cycle,
                    elapsed_secs: cycles.elapsed().as_secs_f64(),
                    freq_shift: Some(freq_shift),
                    pulse_voltage: self.pulse.current_voltage,
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
                if self.handle_stability(rt)? {
                    return Ok(Outcome::Completed);
                }
            }

            // Update voltage strategy for next cycle (uses post-reposition measurement)
            self.pulse
                .update_voltage(&cfg.pulse_method, Some(freq_shift));
        }

        Ok(cycles.outcome())
    }
}

// ============================================================================
// Sweep planning
// ============================================================================

fn build_sweep_plans(tip_prep: &TipPrepConfig) -> Vec<SweepPlan> {
    let sc = &tip_prep.stability;
    let range = sc.bias_range;

    match sc.polarity_mode {
        BiasSweepPolarity::Positive => vec![SweepPlan {
            starting_bias: range.1,
            bias_range: (range.1, range.0),
            index: 1,
            total: 1,
        }],
        BiasSweepPolarity::Negative => vec![SweepPlan {
            starting_bias: -range.1,
            bias_range: (-range.1, -range.0),
            index: 1,
            total: 1,
        }],
        BiasSweepPolarity::Both => vec![
            SweepPlan {
                starting_bias: range.1,
                bias_range: (range.1, range.0),
                index: 1,
                total: 2,
            },
            SweepPlan {
                starting_bias: -range.1,
                bias_range: (-range.1, -range.0),
                index: 2,
                total: 2,
            },
        ],
    }
}
