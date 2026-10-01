use std::borrow::Cow;
use std::time::Duration;

use crate::action::scan::ScanDirectionParam;
use crate::config::{AppConfig, TipPrepConfig};
use crate::controller_types::{BiasSweepPolarity, PolaritySign};
use crate::controllers::{ApplyPreset, ControllerId, PresetStore, TomlPresetStore};
use crate::event::{Event, EventBus};
use crate::routine::{
    Cycles, ExitPolicy, LandingGate, RepositionSpec, Routine, Rt, RunSetup, SafeTipSetup,
    StableReadSpec, ZHome, run_routine,
};
use crate::shutdown::ShutdownFlag;
use crate::signal_registry::{SignalIndex, SignalRegistry};
use crate::spm_controller::{SpmController, ZHomeMode};
use crate::spm_error::SpmError;

use nanonis_rs::scan::ScanPropsBuilder;

use super::PulseState;
use super::events::{ConfigReloadedEvent, CycleEvent, MaxPulseEvent, PhaseEvent};
use super::reload::{ConfigReload, differs, keep_frozen};

pub use crate::routine::Outcome;

// ============================================================================
// Public types
// ============================================================================

/// Everything a tip-preparation run needs besides the controller.
pub struct TipPrepParams<'a> {
    /// Event sink for observers (console, file log, GUI).
    pub events: &'a EventBus,
    /// Cooperative cancellation flag (Ctrl+C, GUI stop button).
    pub shutdown: &'a ShutdownFlag,
    pub config: &'a AppConfig,
    /// The frequency-shift signal driving sharpness decisions.
    pub freq_shift: SignalIndex,
    /// The Z loop's input, the current, which a landing is judged on.
    pub current: SignalIndex,
}

/// The two signals tip prep reads, found by the names the registry gives
/// them: the frequency shift it judges the tip on, and the current its
/// landings wait on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TipPrepSignals {
    pub freq_shift: SignalIndex,
    pub current: SignalIndex,
}

impl TipPrepSignals {
    pub fn resolve(registry: &SignalRegistry) -> Result<Self, SpmError> {
        let find = |name: &str| {
            registry
                .get_by_name(name)
                .map(|s| s.signal_index())
                .ok_or_else(|| SpmError::Workflow(format!("the controller has no {name} signal")))
        };
        Ok(Self {
            freq_shift: find("freq shift")?,
            current: find("current")?,
        })
    }
}

/// Run the full tip preparation algorithm on a controller of its own.
///
/// Convenience wrapper that builds a [`TipPrep`] routine and hands it to
/// [`run_routine`], which owns the run (prepare, Z home and safe-tip,
/// withdraw and retract on exit, teardown). The controller is consumed:
/// dropping it at the end is what stops the data stream. To run tip prep as
/// one of several routines on a persistent connection, build the
/// [`TipPrep`] yourself and call [`run_routine`] with a borrowed controller.
///
/// A shutdown request (Ctrl+C, GUI stop) is an expected way for a run to
/// end, so it surfaces as `Ok(Outcome::StoppedByUser)`, never as an error.
pub fn run_tip_prep(
    mut controller: Box<dyn SpmController>,
    params: TipPrepParams<'_>,
) -> Result<Outcome, SpmError> {
    let TipPrepParams {
        events,
        shutdown,
        config,
        freq_shift,
        current,
    } = params;
    let mut routine = TipPrep::new(config, freq_shift, current);
    run_routine(&mut *controller, events, shutdown, &mut routine)
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
    /// Borrowed from the caller until a reload hands the run its own.
    config: Cow<'a, AppConfig>,
    freq_shift: SignalIndex,
    pulse: PulseState,
    bounds: (f64, f64),
    read_spec: StableReadSpec,
    /// What "landed" means for every approach of the run.
    landing: LandingGate,
    /// Where a new config for the running loop arrives, if anyone can send
    /// one.
    reload: Option<ConfigReload>,
}

/// How a stability check ended. The two that carry on pulsing carry the
/// reading at the site the tip now sits over, since the check moved it and
/// the next pulse fires there.
enum StabilityOutcome {
    Stable,
    NotSharp(f64),
    Unstable(f64),
}

struct SweepPlan {
    starting_bias: f64,
    bias_range: (f64, f64),
    index: usize,
    total: usize,
}

impl<'a> TipPrep<'a> {
    /// `current` is the Z loop's input, which every landing of the run is
    /// judged on against `tip_prep.initial_z_setpoint_a`.
    pub fn new(config: &'a AppConfig, freq_shift: SignalIndex, current: SignalIndex) -> Self {
        let (bounds, read_spec, landing) = gates(config, current);
        Self {
            config: Cow::Borrowed(config),
            freq_shift,
            pulse: PulseState::new(&config.pulse_method),
            bounds,
            read_spec,
            landing,
            reload: None,
        }
    }

    /// Take a new config from `reload` at the top of every cycle. See
    /// [`crate::tip_prep::reload`] for what changes and what the run keeps.
    pub fn with_reload(mut self, reload: ConfigReload) -> Self {
        self.reload = Some(reload);
        self
    }

    /// Switch to the config waiting in the mailbox, if there is one, between
    /// cycles, before the budgets are checked for the next. Writes nothing
    /// to the controller.
    fn take_reload(&mut self, rt: &mut Rt, after_cycle: usize, cycles: &mut Cycles) {
        let Some(incoming) = self.reload.as_ref().and_then(ConfigReload::take) else {
            return;
        };
        let (config, kept) = keep_frozen(&self.config, incoming);
        if !kept.is_empty() {
            log::warn!(
                "Config reload: kept the run's {} until the next run",
                kept.join(", ")
            );
        }
        if differs(&self.config.pulse_method, &config.pulse_method) {
            // A new method starts from its own first voltage; the count
            // carries on, since the random polarity switch counts pulses.
            let count = self.pulse.pulse_count;
            self.pulse = PulseState::new(&config.pulse_method);
            self.pulse.pulse_count = count;
        }
        (self.bounds, self.read_spec, self.landing) = gates(&config, self.landing.index);
        cycles.set_limits(
            config.tip_prep.max_cycles,
            config.tip_prep.max_duration_secs.map(Duration::from_secs),
        );
        log::info!("Config reloaded after cycle {after_cycle}");
        rt.emit(Event::typed(&ConfigReloadedEvent {
            after_cycle,
            config: serde_json::to_value(&config).unwrap_or(serde_json::Value::Null),
            kept,
        }));
        self.config = Cow::Owned(config);
    }

    fn is_sharp(&self, freq_shift: f64) -> bool {
        freq_shift >= self.bounds.0 && freq_shift <= self.bounds.1
    }

    fn read_stable(&self, rt: &mut Rt) -> Result<f64, SpmError> {
        rt.signals()?.read_stable(self.freq_shift, &self.read_spec)
    }

    /// Write the configured Z-controller preset, if there is one, with
    /// the run's setpoint kept. Before the first approach, and after the
    /// setpoint is set, so the loop lands on the gains it will run on.
    /// A preset tuned elsewhere than this run's bias and setpoint gets a
    /// warning when its gains depend on that, and none when they do not.
    fn apply_z_preset(&self, rt: &mut Rt) -> Result<(), SpmError> {
        let tp = &self.config.tip_prep;
        let Some(name) = tp.z_controller_preset.as_deref() else {
            return Ok(());
        };
        let path = &self.config.controllers.presets_file;
        let store = TomlPresetStore::new(path);
        let preset = store
            .get(name)
            .map_err(SpmError::Workflow)?
            .ok_or_else(|| {
                SpmError::Workflow(format!("no controller preset called {name:?} in {path}"))
            })?;
        if preset.id != ControllerId::Z {
            return Err(SpmError::Workflow(format!(
                "preset {name:?} is for the {}, not the Z-controller",
                preset.id
            )));
        }
        let (controller, events) = rt.controller_with_events();
        if preset.depends_on_operating_point() {
            let differs = |tuned: Option<f64>, run: Option<f64>| match (tuned, run) {
                (Some(t), Some(r)) => (t - r).abs() > 0.1 * t.abs().max(r.abs()),
                _ => false,
            };
            // The run sets no amplitude, so the one to compare is the
            // amplitude loop's, as the pane records it when saving.
            let amplitude_m = controller
                .read_controller(ControllerId::PllAmplitude { modulator: 1 })
                .ok()
                .and_then(|r| r.params.setpoint());
            if differs(preset.tuned_at.setpoint, Some(tp.initial_z_setpoint_a))
                || differs(preset.tuned_at.bias_v, Some(tp.initial_bias_v))
                || differs(preset.tuned_at.amplitude_m, amplitude_m)
            {
                log::warn!(
                    "preset {name:?} was tuned at setpoint {:?}, bias {:?} V, amplitude {:?} m; \
                     this run uses {:.3e}, {:.3} V and {:?} m, and this loop's gains depend \
                     on that",
                    preset.tuned_at.setpoint,
                    preset.tuned_at.bias_v,
                    preset.tuned_at.amplitude_m,
                    tp.initial_z_setpoint_a,
                    tp.initial_bias_v,
                    amplitude_m
                );
            }
        }
        log::info!("Applying Z-controller preset {name:?} from {path}");
        ApplyPreset::apply(&preset, controller, events)?;
        Ok(())
    }

    /// Move to a fresh surface spot: withdraw, step the motors, re-approach.
    ///
    /// The landing gate stands where 0.2.3 had fixed settles; what is left
    /// of them, `post_reposition_settle_ms`, is an extra wait after the
    /// gate and none by default.
    fn reposition(&self, rt: &mut Rt) -> Result<(), SpmError> {
        let t = &self.config.tip_prep.timing;
        rt.motor()?.reposition(&RepositionSpec {
            x_steps: t.reposition_steps[0],
            y_steps: t.reposition_steps[1],
            post_approach_settle_ms: t.post_reposition_settle_ms,
            approach_timeout_ms: t.reposition_approach_timeout_ms,
            landing: Some(self.landing.clone()),
            ..Default::default()
        })
    }

    /// `None` once the tip is confirmed stable, otherwise the reading the
    /// next pulse is to be chosen from.
    fn handle_stability(&mut self, rt: &mut Rt) -> Result<Option<f64>, SpmError> {
        let fs = match self.check_stability(rt)? {
            StabilityOutcome::Stable => {
                log::info!("Tip confirmed stable!");
                return Ok(None);
            }
            StabilityOutcome::NotSharp(fs) => {
                log::info!("Tip not confirmed sharp - continuing");
                fs
            }
            StabilityOutcome::Unstable(fs) => {
                log::info!("Stability check failed - reset to blunt, continuing");
                self.pulse.reset(&self.config.pulse_method);
                fs
            }
        };
        // Back to pulsing, or whoever shows the phase keeps showing the
        // check's last step for the rest of the run.
        rt.emit(Event::typed(&PhaseEvent::Pulsing));
        Ok(Some(fs))
    }

    // ------------------------------------------------------------------
    // Confirm sharpness
    // ------------------------------------------------------------------

    /// Whether every read held sharp, and the last reading taken, which is
    /// the one at the site the tip now sits over.
    fn confirm_sharp(&self, rt: &mut Rt) -> Result<(bool, f64), SpmError> {
        const CONFIRMATION_READS: usize = 3;
        let mut last_freq_shift = f64::NAN;

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
                return Ok((false, fs));
            }
            last_freq_shift = fs;
        }

        Ok((true, last_freq_shift))
    }

    // ------------------------------------------------------------------
    // Stability check
    // ------------------------------------------------------------------

    fn check_stability(&mut self, rt: &mut Rt) -> Result<StabilityOutcome, SpmError> {
        rt.emit(Event::typed(&PhaseEvent::Confirming));

        // Step 1: Confirm sharpness with repositioning (3 reads)
        let (confirmed, baseline) = self.confirm_sharp(rt)?;

        if !confirmed {
            log::info!("Tip not confirmed sharp during pre-check");
            return Ok(StabilityOutcome::NotSharp(baseline));
        }

        let stability = &self.config.tip_prep.stability;
        if !stability.check_stability {
            log::info!("Stability checking disabled - accepting sharp tip");
            return Ok(StabilityOutcome::Stable);
        }

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

        rt.emit(Event::typed(&PhaseEvent::StabilityCheck {
            baseline_freq_shift: baseline,
        }));

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
            rt.emit(Event::typed(&PhaseEvent::Stable {
                final_freq_shift: final_fs,
            }));
            Ok(StabilityOutcome::Stable)
        } else {
            rt.emit(Event::typed(&PhaseEvent::Unstable {
                final_freq_shift: final_fs,
                change,
                threshold,
            }));
            // The tip is engaged here: `measure_final_freq_shift` approached
            // it to take the reading being compared. That is the point. A
            // pulse is a field at the apex, and the apex is only in a field
            // when it is near the surface, so a max pulse fired withdrawn
            // reshapes nothing and the next cycle inherits the same unstable
            // apex. 0.2.3 fired engaged; an earlier v2 revision withdrew
            // first and turned this branch into a no-op.
            //
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
            rt.emit(Event::typed(&MaxPulseEvent {
                pulse_voltage: signed_max,
            }));

            self.reposition(rt)?;
            Ok(StabilityOutcome::Unstable(self.read_stable(rt)?))
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
        rt.z()?
            .calibrated_approach_within(self.approach_timeout(), Some(self.landing.clone()))?;
        rt.settle(t.post_approach_settle_ms)?;

        Ok(())
    }

    /// Budget for an approach that starts from a full withdraw.
    fn approach_timeout(&self) -> Duration {
        Duration::from_millis(self.config.tip_prep.timing.approach_timeout_ms)
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
        rt.z()?
            .calibrated_approach_within(self.approach_timeout(), Some(self.landing.clone()))?;
        rt.settle(self.config.tip_prep.timing.post_approach_settle_ms)?;

        self.read_stable(rt)
    }
}

impl Routine for TipPrep<'_> {
    fn name(&self) -> &str {
        "tip_prep"
    }

    fn run_setup(&self) -> RunSetup {
        RunSetup {
            // Spelled out rather than defaulted: the home step of every
            // calibrated approach is "back off 50 nm from wherever the tip
            // is". Absolute mode would make it "go to Z = +50 nm", surface
            // or not.
            z_home: Some(ZHome {
                mode: ZHomeMode::Relative,
                position_m: 50e-9,
            }),
            // Off for the run, restored on exit. A pulse is a current spike
            // by design, and safe-tip would retract on every one.
            safe_tip: Some(SafeTipSetup {
                threshold_a: self.config.tip_prep.safe_tip_threshold,
                disable: true,
            }),
        }
    }

    fn exit_policy(&self) -> ExitPolicy {
        ExitPolicy::Withdraw {
            retract_steps: self.config.tip_prep.timing.exit_retract_steps,
        }
    }

    fn run(&mut self, rt: &mut Rt) -> Result<Outcome, SpmError> {
        // Read off `self.config` where used, never held: a reload replaces
        // it between cycles.

        // The stable read judges drift at the rate the controller measured
        // on its stream and falls back to the config value; say which.
        match rt.controller().stream_rate_hz() {
            Some(hz) => {
                let configured = self.read_spec.sample_rate_hz;
                if (hz - configured).abs() > 0.1 * configured {
                    log::warn!(
                        "data_acquisition.sample_rate says {configured:.0} Hz but the stream \
                         delivers {hz:.0} Hz; using the measured rate for the drift gate"
                    );
                }
            }
            None => log::warn!(
                "Stream rate unknown; drift gate uses data_acquisition.sample_rate = {:.0} Hz",
                self.read_spec.sample_rate_hz
            ),
        }

        log::info!("Initializing...");
        rt.bias()?.set(self.config.tip_prep.initial_bias_v)?;
        rt.z()?
            .set_setpoint(self.config.tip_prep.initial_z_setpoint_a)?;
        self.apply_z_preset(rt)?;
        rt.z()?
            .calibrated_approach_within(self.approach_timeout(), Some(self.landing.clone()))?;

        // Clear the stream buffer to discard stale pre-approach data
        let timing = &self.config.tip_prep.timing;
        let waits = [timing.buffer_clear_wait_ms, timing.post_approach_settle_ms];
        rt.signals()?.clear_buffer();
        for ms in waits {
            rt.settle(ms)?;
        }

        // The budget clock starts here, before the initial measurement, so
        // an initial stability check already counts against max_duration.
        let mut cycles = rt.cycles(
            self.config.tip_prep.max_cycles,
            self.config
                .tip_prep
                .max_duration_secs
                .map(Duration::from_secs),
        );

        // Check if tip is already sharp after initial approach
        let initial_fs = self.read_stable(rt)?;
        let initial_sharp = self.is_sharp(initial_fs);
        log::info!(
            "Initial tip state: freq_shift={:.3} Hz, sharp={}",
            initial_fs,
            initial_sharp
        );

        let mut site_fs = initial_fs;
        if initial_sharp {
            log::info!("Tip already sharp after approach - running stability check");
            match self.handle_stability(rt)? {
                None => return Ok(Outcome::Completed),
                Some(fs) => site_fs = fs,
            }
        }
        // The first pulse fires at this site, so it is chosen from this
        // reading like every later one is from the reading before it.
        self.pulse
            .update_voltage(&self.config.pulse_method, Some(site_fs));
        if !initial_sharp {
            rt.emit(Event::typed(&PhaseEvent::Pulsing));
        }

        // Main loop: pulse -> settle -> reposition -> measure -> check sharp
        // Matches V1 ordering: minimize time at pulsed position to avoid
        // unintended tip changes from continued surface interaction.
        let mut last_cycle = 0;
        loop {
            // A config sent while running lands here, between cycles and
            // before the budgets are checked, so a lowered limit holds.
            self.take_reload(rt, last_cycle, &mut cycles);
            let Some(cycle) = cycles.next() else { break };
            last_cycle = cycle;
            let timing = self.config.tip_prep.timing.clone();

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
                self.config.pulse_method.method_name(),
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
            let mut freq_shift = self.read_stable(rt)?;
            let is_sharp = self.is_sharp(freq_shift);

            rt.emit(Event::typed(&CycleEvent {
                cycle,
                elapsed_secs: cycles.elapsed().as_secs_f64(),
                freq_shift,
                // The voltage that was fired, sign included. The pulse
                // state's `current_voltage` is a magnitude, and reporting
                // it hid every polarity switch from the GUI.
                pulse_voltage,
                is_sharp,
            }));

            if is_sharp {
                log::info!(
                    "Tip sharp at cycle {} (freq_shift={:.3} Hz)",
                    cycle,
                    freq_shift
                );
                match self.handle_stability(rt)? {
                    None => return Ok(Outcome::Completed),
                    // The check moved the tip, and the next pulse fires
                    // where it left it.
                    Some(fs) => freq_shift = fs,
                }
            }

            // Update voltage strategy for next cycle (uses post-reposition measurement)
            self.pulse
                .update_voltage(&self.config.pulse_method, Some(freq_shift));
        }

        Ok(cycles.outcome())
    }
}

/// The sharp window, the stable-read gate and the landing gate a config
/// asks for, landings judged on `current`.
fn gates(config: &AppConfig, current: SignalIndex) -> ((f64, f64), StableReadSpec, LandingGate) {
    let tp = &config.tip_prep;
    let samples = config.data_acquisition.stable_signal_samples;
    let rate = config.data_acquisition.sample_rate as f64;
    (
        (tp.sharp_tip_bounds[0], tp.sharp_tip_bounds[1]),
        StableReadSpec {
            num_samples: samples,
            max_std_dev: tp.signal_stability.max_std_dev_hz,
            max_slope: tp.signal_stability.max_slope_hz_per_s,
            max_retries: tp.signal_stability.read_retry_count as usize,
            sample_rate_hz: rate,
        },
        LandingGate {
            index: current,
            setpoint: tp.initial_z_setpoint_a,
            tolerance: tp.timing.landing_tolerance,
            num_samples: samples,
            timeout_ms: tp.timing.landing_timeout_ms,
            sample_rate_hz: rate,
        },
    )
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
