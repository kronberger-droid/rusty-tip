use crossbeam_channel::Sender;
use log::info;
use nanonis_rs::signals::SignalIndex;
use nanonis_rs::z_ctrl::ZHomeMode;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::ScanConfig;
use crate::Signal;
use crate::action_driver::ActionDriver;
use crate::actions::{Action, TipCheckMethod, TipState};
use crate::controller_types::{
    BiasSweepPolarity, ControllerAction, ControllerState, PolaritySign,
    PulseMethod, TipControllerConfig,
};
use crate::error::{Error, RunOutcome};
use crate::types::{MotorDisplacement, TipShape};

// ============================================================================
// SWEEP PLAN (internal)
// ============================================================================

/// Plan for a single bias stability sweep
struct SweepPlan {
    /// Bias voltage to set before approach (extreme value, far from zero)
    starting_bias: f32,
    /// Sweep range: (from, to) -- always sweeps from extreme toward zero
    bias_range: (f32, f32),
    /// 1-based index of this sweep
    index: usize,
    /// Total number of sweeps
    total: usize,
}

// ============================================================================
// TIP CONTROLLER
// ============================================================================

/// Enhanced tip controller with pulse voltage stepping
pub struct TipController {
    driver: ActionDriver,
    config: TipControllerConfig,

    // State tracking
    current_pulse_voltage: f32,
    current_tip_shape: TipShape,
    cycles_without_change: u32,
    cycle_count: u32,

    // Multi-signal history for bias adjustment and analysis
    signal_histories: HashMap<u8, VecDeque<f32>>, // Key is signal.index
    max_history_size: usize,

    // Loop termination safeguards
    max_cycles: Option<usize>,
    max_duration: Option<Duration>,
    loop_start_time: Option<std::time::Instant>,

    // Graceful shutdown support
    shutdown_requested: Option<Arc<AtomicBool>>,

    // Polarity tracking
    base_polarity: PolaritySign,
    pulse_count_for_random: u32,

    // State reporting for GUI
    state_sender: Option<Sender<ControllerState>>,
    current_action: ControllerAction,
    measured_freq_shift: Option<f32>,
}

impl TipController {
    /// Create new tip controller with basic signal bounds
    pub fn new(driver: ActionDriver, config: TipControllerConfig) -> Self {
        let initial_voltage = match &config.pulse_method {
            PulseMethod::Fixed { voltage, .. } => *voltage,
            PulseMethod::Stepping { voltage_bounds, .. } => voltage_bounds.0,
            PulseMethod::Linear { voltage_bounds, .. } => voltage_bounds.0,
        };
        let base_polarity = match &config.pulse_method {
            PulseMethod::Fixed { polarity, .. } => *polarity,
            PulseMethod::Stepping { polarity, .. } => *polarity,
            PulseMethod::Linear { polarity, .. } => *polarity,
        };
        let max_cycles = config.max_cycles;
        let max_duration = config.max_duration;
        Self {
            driver,
            config,
            current_pulse_voltage: initial_voltage,
            current_tip_shape: TipShape::Blunt,
            cycles_without_change: 0,
            cycle_count: 0,
            signal_histories: HashMap::new(),
            max_history_size: 100,
            max_cycles,
            max_duration,
            loop_start_time: None,
            shutdown_requested: None,
            base_polarity,
            pulse_count_for_random: 0,
            state_sender: None,
            current_action: ControllerAction::Idle,
            measured_freq_shift: None,
        }
    }

    /// Set a channel to send state updates to (for GUI)
    pub fn set_state_sender(&mut self, sender: Sender<ControllerState>) {
        self.state_sender = Some(sender);
    }

    /// Get current controller state snapshot
    pub fn snapshot(&self) -> ControllerState {
        let elapsed_secs = self
            .loop_start_time
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);

        let freq_shift = self.measured_freq_shift;

        ControllerState {
            tip_shape: self.current_tip_shape,
            cycle_count: self.cycle_count,
            pulse_voltage: self.current_pulse_voltage,
            freq_shift,
            elapsed_secs,
            current_action: self.current_action.clone(),
        }
    }

    /// Send current state to the GUI (if connected)
    fn send_state(&self) {
        if let Some(sender) = &self.state_sender {
            let _ = sender.try_send(self.snapshot());
        }
    }

    /// Update current action and send state
    fn set_action(&mut self, action: ControllerAction) {
        self.current_action = action;
        self.send_state();
    }

    /// Set shutdown flag for graceful termination
    pub fn set_shutdown_flag(&mut self, flag: Arc<AtomicBool>) {
        self.shutdown_requested = Some(flag.clone());
        self.driver.set_shutdown_flag(flag);
    }

    /// Check if shutdown has been requested
    fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested
            .as_ref()
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Return Err(Error::Shutdown) if shutdown was requested
    fn check_shutdown(&self) -> Result<(), Error> {
        if self.is_shutdown_requested() {
            info!("Shutdown requested at cycle {}", self.cycle_count);
            Err(Error::Shutdown)
        } else {
            Ok(())
        }
    }

    /// Check if this pulse should use opposite polarity
    fn should_use_opposite_polarity(&self) -> bool {
        match &self.config.pulse_method {
            PulseMethod::Stepping {
                random_polarity_switch: Some(switch),
                ..
            }
            | PulseMethod::Fixed {
                random_polarity_switch: Some(switch),
                ..
            }
            | PulseMethod::Linear {
                random_polarity_switch: Some(switch),
                ..
            } => {
                switch.enabled
                    && self.pulse_count_for_random > 0
                    && self.pulse_count_for_random
                        % switch.switch_every_n_pulses
                        == 0
            }
            _ => false,
        }
    }

    /// Get the signed pulse voltage based on current polarity
    fn get_signed_pulse_voltage(&self) -> f32 {
        let polarity = if self.should_use_opposite_polarity() {
            self.base_polarity.opposite()
        } else {
            self.base_polarity
        };

        match polarity {
            PolaritySign::Positive => self.current_pulse_voltage,
            PolaritySign::Negative => -self.current_pulse_voltage,
        }
    }

    /// Track a signal value in history
    pub fn track_signal(&mut self, signal: &Signal, value: f32) {
        let history = self.signal_histories.entry(signal.index).or_default();
        history.push_front(value);
        while history.len() > self.max_history_size {
            history.pop_back();
        }
    }

    /// Get signal change (latest - previous) for a specific signal
    #[allow(dead_code)]
    pub fn get_signal_change(&self, signal: &Signal) -> Option<f32> {
        if let Some(history) = self.signal_histories.get(&signal.index) {
            if history.len() >= 2 {
                Some(history[0] - history[1])
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Get signal history for a specific signal (most recent first)
    #[allow(dead_code)]
    pub fn get_signal_history(
        &self,
        signal: &Signal,
    ) -> Option<&VecDeque<f32>> {
        self.signal_histories.get(&signal.index)
    }

    #[allow(dead_code)]
    pub fn get_last_signal(&self, signal: &Signal) -> Option<f32> {
        match self.get_signal_history(signal) {
            Some(history) => history.front().copied(),
            None => None,
        }
    }

    /// Clear all signal histories
    #[allow(dead_code)]
    pub fn clear_all_histories(&mut self) {
        self.signal_histories.clear();
    }

    /// Clear history for a specific signal
    #[allow(dead_code)]
    pub fn clear_signal_history(&mut self, signal: &Signal) {
        self.signal_histories.remove(&signal.index);
    }

    /// Execute a single pulse at maximum voltage to aggressively reshape the tip
    /// Used when stability check fails to force tip into a different state
    fn execute_max_pulse(&mut self) -> Result<(), Error> {
        self.pulse_count_for_random += 1;

        let max_voltage = self.config.pulse_method.max_voltage();
        let using_opposite = self.should_use_opposite_polarity();
        let signed_voltage = if using_opposite {
            match self.base_polarity.opposite() {
                PolaritySign::Positive => max_voltage,
                PolaritySign::Negative => -max_voltage,
            }
        } else {
            match self.base_polarity {
                PolaritySign::Positive => max_voltage,
                PolaritySign::Negative => -max_voltage,
            }
        };

        let current_polarity = if using_opposite {
            self.base_polarity.opposite()
        } else {
            self.base_polarity
        };

        info!(
            "Executing MAX pulse #{} due to stability failure: {:.3}V ({:?}{})",
            self.pulse_count_for_random,
            signed_voltage,
            current_polarity,
            if using_opposite { " - SWITCHED" } else { "" }
        );

        self.driver
            .run(Action::BiasPulse {
                wait_until_done: true,
                pulse_width: self.config.pulse_width,
                bias_value_v: signed_voltage,
                z_controller_hold: 0,
                pulse_mode: 0,
            })
            .go()?;

        Ok(())
    }

    /// Check if current signal represents a significant change from recent stable period
    fn has_significant_change(&self, signal: &Signal) -> (bool, f32) {
        let threshold_value = match &self.config.pulse_method {
            PulseMethod::Stepping {
                threshold_value, ..
            } => *threshold_value,
            PulseMethod::Fixed { .. } => return (false, 0.0),
            PulseMethod::Linear { .. } => return (false, 0.0),
        };

        if let Some(history) = self.signal_histories.get(&signal.index) {
            if history.len() < 2 {
                (true, 0.0)
            } else {
                let stable_period_size = (self.cycles_without_change as usize)
                    .min(history.len() - 1);

                if stable_period_size == 0 {
                    let current_signal = history[0];
                    let last_signal = history[1];

                    log::debug!(
                        "Last signal: {:.3e} | Current threshold: {:.3e}",
                        last_signal,
                        threshold_value
                    );

                    let change = current_signal - last_signal;
                    let has_change = change.abs() >= threshold_value;
                    (has_change, change)
                } else {
                    let current_signal = history[0];
                    let stable_signals: Vec<f32> = history
                        .iter()
                        .skip(1)
                        .take(stable_period_size)
                        .cloned()
                        .collect();
                    let stable_mean = stable_signals.iter().sum::<f32>()
                        / stable_signals.len() as f32;

                    log::debug!(
                        "Current: {:.3e} | Stable mean: {:.3e} | Threshold: {:.3e}",
                        current_signal,
                        stable_mean,
                        threshold_value
                    );

                    let change = current_signal - stable_mean;
                    let has_change = change.abs() >= threshold_value;
                    (has_change, change)
                }
            }
        } else {
            (true, 0.0)
        }
    }

    /// Step up the pulse voltage if possible
    fn step_pulse_voltage(&mut self) -> bool {
        let (voltage_bounds, voltage_steps) = match &self.config.pulse_method {
            PulseMethod::Stepping {
                voltage_bounds,
                voltage_steps,
                ..
            } => (*voltage_bounds, *voltage_steps),
            PulseMethod::Fixed { .. } => return false,
            PulseMethod::Linear { .. } => return false,
        };

        let step_size =
            (voltage_bounds.1 - voltage_bounds.0) / voltage_steps as f32;
        let new_pulse =
            (self.current_pulse_voltage + step_size).min(voltage_bounds.1);

        if new_pulse > self.current_pulse_voltage {
            info!(
                "Stepping pulse voltage: {:.3}V -> {:.3}V",
                self.current_pulse_voltage, new_pulse
            );
            self.current_pulse_voltage = new_pulse;
            self.cycles_without_change = 0;
            true
        } else {
            log::debug!(
                "Pulse voltage already at maximum: {:.3}V",
                voltage_bounds.1
            );
            self.cycles_without_change = 0;
            false
        }
    }

    /// Update signal history and step pulse voltage if needed
    fn update_pulse_voltage(&mut self) {
        match &self.config.pulse_method {
            PulseMethod::Stepping {
                voltage_bounds,
                cycles_before_step,
                ..
            } => {
                let (is_significant, change) =
                    self.has_significant_change(&self.config.freq_shift_signal);
                if is_significant && change >= 0.0 {
                    self.cycles_without_change = 0;
                    self.current_pulse_voltage = voltage_bounds.0;
                    log::debug!(
                        "Positive significant change detected, resetting pulse voltage to minimum: {:.3}V",
                        self.current_pulse_voltage
                    );
                } else if is_significant {
                    log::warn!("Negative significant change detected!");
                    self.cycles_without_change += 1;
                    if self.cycles_without_change >= *cycles_before_step as u32
                    {
                        self.step_pulse_voltage();
                    }
                } else {
                    self.cycles_without_change += 1;
                    if self.cycles_without_change >= *cycles_before_step as u32
                    {
                        self.step_pulse_voltage();
                    }
                }
            }
            PulseMethod::Fixed { .. } => {}
            PulseMethod::Linear {
                voltage_bounds,
                linear_clamp,
                ..
            } => {
                let current_freq_shift;
                let mut pulse_voltage = self.current_pulse_voltage;

                if let Some(freq_shift_history) = self
                    .signal_histories
                    .get(&self.config.freq_shift_signal.index)
                {
                    current_freq_shift = freq_shift_history[0];

                    if !(linear_clamp.0..linear_clamp.1)
                        .contains(&current_freq_shift)
                    {
                        log::info!(
                            "Linear pulse: freq_shift {:.2} Hz outside range [{:.2}, {:.2}] Hz -> using max voltage {:.2}V",
                            current_freq_shift,
                            linear_clamp.0,
                            linear_clamp.1,
                            voltage_bounds.1
                        );
                        pulse_voltage = voltage_bounds.1;
                    } else {
                        let slope = (voltage_bounds.1 - voltage_bounds.0)
                            / (linear_clamp.1 - linear_clamp.0);
                        let d = voltage_bounds.0 - slope * linear_clamp.0;
                        pulse_voltage = slope * current_freq_shift + d;
                        log::info!(
                            "Linear pulse: freq_shift {:.2} Hz in range [{:.2}, {:.2}] Hz -> calculated voltage {:.2}V",
                            current_freq_shift,
                            linear_clamp.0,
                            linear_clamp.1,
                            pulse_voltage
                        );
                    }
                }

                self.current_pulse_voltage = pulse_voltage;
            }
        }
    }
}

// ============================================================================
// MAIN CONTROL LOOP
// ============================================================================

impl TipController {
    /// Main control loop - returns RunOutcome on success, Error on failure.
    /// Shutdown is treated as a successful outcome (StoppedByUser), not an error.
    pub fn run(&mut self) -> Result<RunOutcome, Error> {
        self.loop_start_time = Some(std::time::Instant::now());
        self.set_action(ControllerAction::Initializing);

        match self.run_inner() {
            Ok(()) => {
                self.set_action(ControllerAction::Completed);
                Ok(RunOutcome::Completed)
            }
            Err(Error::Shutdown) => {
                self.set_action(ControllerAction::Stopped);
                Ok(RunOutcome::StoppedByUser)
            }
            Err(e) => Err(e),
        }
    }

    /// Inner loop that uses Error::Shutdown for shutdown signaling
    fn run_inner(&mut self) -> Result<(), Error> {
        self.pre_loop_initialization()?;

        while self.current_tip_shape != TipShape::Stable {
            // Check cycle limit
            if let Some(max) = self.max_cycles
                && self.cycle_count >= max as u32
            {
                self.set_action(ControllerAction::Error(
                    "Max cycles exceeded".to_string(),
                ));
                return Err(Error::CycleLimit(max as u32));
            }

            // Check wall-clock timeout
            if let Some(max_dur) = self.max_duration
                && let Some(start_time) = self.loop_start_time
                && start_time.elapsed() > max_dur
            {
                self.set_action(ControllerAction::Error(
                    "Max duration exceeded".to_string(),
                ));
                return Err(Error::TimedOut(max_dur));
            }

            // Check shutdown flag
            self.check_shutdown()?;

            // Execute one control cycle
            self.cycle_count += 1;
            self.measured_freq_shift = None;

            // Periodic status report
            if self.cycle_count % self.config.status_interval as u32 == 0
                && let Some(start_time) = self.loop_start_time
            {
                let elapsed = start_time.elapsed();
                info!(
                    "Status: cycle={}, state={:?}, pulse_v={:.2}V, elapsed={:.1}s",
                    self.cycle_count,
                    self.current_tip_shape,
                    self.current_pulse_voltage,
                    elapsed.as_secs_f32()
                );
            }

            // Execute based on state
            match self.current_tip_shape {
                TipShape::Blunt => {
                    info!(
                        "Cycle {}: running bad loop ==============",
                        self.cycle_count
                    );
                    self.set_action(ControllerAction::Pulsing);
                    self.bad_loop()?;
                }
                TipShape::Sharp => {
                    info!(
                        "Cycle {}: running good loop ==============",
                        self.cycle_count
                    );
                    self.set_action(ControllerAction::StabilityCheck);
                    self.good_loop()?;
                }
                TipShape::Stable => {
                    info!("STABLE achieved after {} cycles!", self.cycle_count);
                    break;
                }
            }

            // Send state after the cycle so measured_freq_shift reflects any measurement taken
            self.send_state();
        }
        Ok(())
    }

    /// Bad loop - execute recovery sequence with stable signal monitoring
    fn bad_loop(&mut self) -> Result<(), Error> {
        self.pulse_count_for_random += 1;

        let using_opposite = self.should_use_opposite_polarity();
        let current_polarity = if using_opposite {
            self.base_polarity.opposite()
        } else {
            self.base_polarity
        };
        let signed_voltage = self.get_signed_pulse_voltage();

        info!(
            "Executing pulse #{}: {:.3}V ({} method, {:?}{})",
            self.pulse_count_for_random,
            signed_voltage,
            self.config.pulse_method.method_name(),
            current_polarity,
            if using_opposite { " - SWITCHED" } else { "" }
        );

        self.driver
            .run(Action::BiasPulse {
                wait_until_done: true,
                pulse_width: self.config.pulse_width,
                bias_value_v: signed_voltage,
                z_controller_hold: 0,
                pulse_mode: 0,
            })
            .go()?;

        log::debug!(
            "Waiting {:?} for signal to settle after pulse...",
            self.config.post_pulse_settle
        );
        self.driver
            .run(Action::Wait {
                duration: self.config.post_pulse_settle,
            })
            .go()?;

        log::debug!("Repositioning...");

        self.set_action(ControllerAction::Repositioning);
        self.driver
            .run(Action::SafeReposition {
                x_steps: self.config.reposition_steps.0,
                y_steps: self.config.reposition_steps.1,
            })
            .go()?;

        // Wait for signal to stabilize after reposition/approach
        log::debug!(
            "Waiting {:?} for signal to stabilize after reposition...",
            self.config.post_reposition_settle
        );
        self.driver
            .run(Action::Wait {
                duration: self.config.post_reposition_settle,
            })
            .go()?;

        self.set_action(ControllerAction::MeasuringSignal);
        let tip_state: TipState = self
            .driver
            .run(Action::CheckTipState {
                method: TipCheckMethod::SignalBounds {
                    signal: self.config.freq_shift_signal.clone(),
                    bounds: self.config.sharp_tip_bounds,
                },
            })
            .expecting()?;

        self.current_tip_shape = tip_state.shape;

        // Track the frequency shift signal if available
        if let Some(freq_shift_value) = tip_state
            .measured_signals
            .get(&SignalIndex::new(self.config.freq_shift_signal.index))
            .copied()
        {
            let signal = self.config.freq_shift_signal.clone();
            self.track_signal(&signal, freq_shift_value);
            self.measured_freq_shift = Some(freq_shift_value);
        } else {
            log::warn!(
                "CheckTipState did not return frequency shift signal (index: {})",
                self.config.freq_shift_signal.index
            );
        }

        // Update pulse voltage based on signal changes (stepping logic)
        self.update_pulse_voltage();

        Ok(())
    }

    /// Good loop - monitoring, increment good count
    fn good_loop(&mut self) -> Result<(), Error> {
        self.set_action(ControllerAction::StabilityCheck);

        let (confident_tip_shape, baseline_freq_shift) =
            self.pre_good_loop_check()?;

        if matches!(confident_tip_shape, TipShape::Blunt) {
            info!("Tip Shape was wrongly measured as good");
            self.current_tip_shape = TipShape::Blunt;
            self.send_state();
            return Ok(());
        }

        // If stability checking is disabled, mark tip as stable immediately
        if !self.config.check_stability {
            info!("Stability checking disabled - marking tip as stable");
            self.current_tip_shape = TipShape::Stable;
            self.send_state();
            return Ok(());
        }

        let baseline_freq_shift = match baseline_freq_shift {
            Some(v) => v,
            None => {
                log::error!(
                    "No baseline freq_shift available for stability check"
                );
                self.current_tip_shape = TipShape::Blunt;
                self.send_state();
                return Ok(());
            }
        };

        info!(
            "Baseline freq_shift for stability comparison: {:.3} Hz",
            baseline_freq_shift
        );

        let sweep_plans = self.build_sweep_plans();

        info!(
            "Starting stability check with polarity mode: {:?}, {} sweep(s)",
            self.config.stability_config.polarity_mode,
            sweep_plans.len()
        );

        let original_scan_config = self.save_and_set_scan_speed()?;

        let mut shutdown_requested = false;
        for plan in &sweep_plans {
            if self.is_shutdown_requested() {
                log::info!(
                    "Shutdown requested before stability sweep {}",
                    plan.index
                );
                shutdown_requested = true;
                break;
            }

            self.prepare_for_sweep(plan.starting_bias)?;
            let _ = self.execute_stability_sweep(plan)?;
        }

        self.restore_scan_speed(original_scan_config);

        if shutdown_requested {
            return Err(Error::Shutdown);
        }

        // After all sweeps: withdraw, restore initial bias, approach, read freq_shift
        let final_freq_shift = self.measure_final_freq_shift()?;

        // Compare baseline vs final freq_shift
        let signal_change = (final_freq_shift - baseline_freq_shift).abs();
        let is_stable = signal_change <= self.config.allowed_change_for_stable;

        info!(
            "Stability comparison: baseline={:.3} Hz, final={:.3} Hz, change={:.3} Hz, threshold={:.3} Hz, stable={}",
            baseline_freq_shift,
            final_freq_shift,
            signal_change,
            self.config.allowed_change_for_stable,
            is_stable
        );

        self.handle_stability_outcome(is_stable, sweep_plans.len())?;

        Ok(())
    }

    /// After all stability sweeps, withdraw, restore initial bias, approach, and read freq_shift.
    fn measure_final_freq_shift(&mut self) -> Result<f32, Error> {
        info!("Measuring final freq_shift after all sweeps");

        self.set_action(ControllerAction::Withdrawing);
        self.driver
            .run(Action::Withdraw {
                wait_until_finished: true,
                timeout: Duration::from_secs(5),
            })
            .go()?;

        self.driver
            .run(Action::Wait {
                duration: Duration::from_millis(200),
            })
            .go()?;

        self.driver
            .run(Action::SetBias {
                voltage: self.config.initial_bias_v,
            })
            .go()?;
        info!(
            "Bias restored to initial value: {:.3} V",
            self.config.initial_bias_v
        );

        self.set_action(ControllerAction::CenteringFreqShift);
        self.driver
            .run(Action::AutoApproach {
                wait_until_finished: true,
                timeout: Duration::from_secs(600),
                center_freq_shift: true,
            })
            .go()?;

        info!(
            "Waiting {:?} for signal to stabilize after approach...",
            self.config.post_approach_settle
        );
        self.driver
            .run(Action::Wait {
                duration: self.config.post_approach_settle,
            })
            .go()?;

        self.set_action(ControllerAction::MeasuringSignal);
        let tip_state: TipState = self
            .driver
            .run(Action::CheckTipState {
                method: TipCheckMethod::SignalBounds {
                    signal: self.config.freq_shift_signal.clone(),
                    bounds: self.config.sharp_tip_bounds,
                },
            })
            .expecting()?;

        let final_freq_shift = tip_state
            .measured_signals
            .get(&SignalIndex::new(self.config.freq_shift_signal.index))
            .copied()
            .ok_or_else(|| {
                Error::Nanonis(crate::NanonisError::Protocol(
                    "Failed to read final freq_shift after stability sweeps"
                        .to_string(),
                ))
            })?;

        let signal = self.config.freq_shift_signal.clone();
        self.track_signal(&signal, final_freq_shift);
        self.measured_freq_shift = Some(final_freq_shift);

        info!("Final freq_shift: {:.3} Hz", final_freq_shift);
        Ok(final_freq_shift)
    }

    /// Build sweep plans based on polarity mode.
    fn build_sweep_plans(&self) -> Vec<SweepPlan> {
        let stability_config = &self.config.stability_config;
        let polarity_mode = stability_config.polarity_mode;
        let bias_range = stability_config.bias_range;

        match polarity_mode {
            BiasSweepPolarity::Positive => {
                vec![SweepPlan {
                    starting_bias: bias_range.1,
                    bias_range: (bias_range.1, bias_range.0),
                    index: 1,
                    total: 1,
                }]
            }
            BiasSweepPolarity::Negative => {
                vec![SweepPlan {
                    starting_bias: -bias_range.1,
                    bias_range: (-bias_range.1, -bias_range.0),
                    index: 1,
                    total: 1,
                }]
            }
            BiasSweepPolarity::Both => {
                vec![
                    SweepPlan {
                        starting_bias: bias_range.1,
                        bias_range: (bias_range.1, bias_range.0),
                        index: 1,
                        total: 2,
                    },
                    SweepPlan {
                        starting_bias: -bias_range.1,
                        bias_range: (-bias_range.1, -bias_range.0),
                        index: 2,
                        total: 2,
                    },
                ]
            }
        }
    }

    /// Prepare the tip for a stability sweep
    fn prepare_for_sweep(&mut self, starting_bias: f32) -> Result<(), Error> {
        info!(
            "Preparing for sweep: withdrawing and repositioning, starting bias = {:.3}V",
            starting_bias
        );

        self.set_action(ControllerAction::Withdrawing);
        self.driver
            .run(Action::Withdraw {
                wait_until_finished: true,
                timeout: Duration::from_secs(5),
            })
            .go()?;

        self.set_action(ControllerAction::Repositioning);
        self.driver
            .run(Action::MoveMotor3D {
                displacement: MotorDisplacement::new(
                    self.config.reposition_steps.0,
                    self.config.reposition_steps.1,
                    -3,
                ),
                blocking: true,
            })
            .go()?;

        self.driver
            .run(Action::Wait {
                duration: Duration::from_millis(200),
            })
            .go()?;

        self.driver
            .run(Action::SetBias {
                voltage: starting_bias,
            })
            .go()?;
        info!("Bias set to {:.3}V before approach", starting_bias);

        self.set_action(ControllerAction::CenteringFreqShift);
        self.driver
            .run(Action::AutoApproach {
                wait_until_finished: true,
                timeout: Duration::from_secs(600),
                center_freq_shift: true,
            })
            .go()?;

        info!(
            "Waiting {:?} for signal to stabilize after approach...",
            self.config.post_approach_settle
        );
        self.driver
            .run(Action::Wait {
                duration: self.config.post_approach_settle,
            })
            .go()?;

        Ok(())
    }

    /// Execute a single stability sweep and return whether the tip was stable.
    fn execute_stability_sweep(
        &mut self,
        plan: &SweepPlan,
    ) -> Result<bool, Error> {
        let stability_config = self.config.stability_config.clone();
        let step_duration =
            Duration::from_millis(stability_config.step_period_ms);
        let max_duration =
            Duration::from_secs(stability_config.max_duration_secs);
        let bias_steps = stability_config.bias_steps;

        self.set_action(ControllerAction::StabilitySweep {
            sweep: plan.index as u32,
            total: plan.total as u32,
        });
        info!(
            "Stability sweep {}/{}: {:.2}V to {:.2}V",
            plan.index, plan.total, plan.bias_range.0, plan.bias_range.1
        );

        let stability_result: crate::actions::StabilityResult = self
            .driver
            .run(Action::CheckTipStability {
                method: crate::actions::TipStabilityMethod::BiasSweepResponse {
                    signal: self.config.freq_shift_signal.clone(),
                    bias_range: plan.bias_range,
                    bias_steps,
                    step_duration,
                    allowed_signal_change: self
                        .config
                        .allowed_change_for_stable,
                },
                max_duration,
            })
            .expecting()?;

        if let Some(signal_values) = stability_result
            .measured_values
            .get(&self.config.freq_shift_signal)
            && let Some(&last_value) = signal_values.last()
        {
            let signal = self.config.freq_shift_signal.clone();
            self.track_signal(&signal, last_value);
        }

        if !stability_result.is_stable {
            info!(
                "Tip unstable during sweep {}/{} ({:.2}V to {:.2}V)",
                plan.index, plan.total, plan.bias_range.0, plan.bias_range.1
            );
        }

        Ok(stability_result.is_stable)
    }

    /// Save the current scan speed and set the stability-check speed if configured.
    fn save_and_set_scan_speed(&mut self) -> Result<Option<ScanConfig>, Error> {
        let target_speed = self.config.stability_config.scan_speed_m_s;

        if let Some(target_speed) = target_speed {
            match self.driver.client_mut().scan_speed_get() {
                Ok(config) => {
                    info!(
                        "Saving original scan speed: {:.2e} m/s (forward), {:.2e} m/s (backward)",
                        config.forward_linear_speed_m_s,
                        config.backward_linear_speed_m_s
                    );
                    let mut new_config = config;
                    new_config.forward_linear_speed_m_s = target_speed;
                    new_config.backward_linear_speed_m_s = target_speed;
                    new_config.keep_parameter_constant = 1;
                    if let Err(e) =
                        self.driver.client_mut().scan_config_set(new_config)
                    {
                        log::warn!(
                            "Failed to set scan speed for stability check: {}",
                            e
                        );
                    } else {
                        info!(
                            "Set scan speed to {:.2e} m/s for stability check",
                            target_speed
                        );
                    }
                    Ok(Some(config))
                }
                Err(e) => {
                    log::warn!("Failed to get current scan speed: {}", e);
                    Ok(None)
                }
            }
        } else {
            Ok(None)
        }
    }

    /// Restore the original scan speed if it was saved.
    fn restore_scan_speed(&mut self, original: Option<ScanConfig>) {
        if let Some(config) = original {
            if let Err(e) = self.driver.client_mut().scan_config_set(config) {
                log::warn!("Failed to restore original scan speed: {}", e);
            } else {
                info!(
                    "Restored original scan speed: {:.2e} m/s (forward)",
                    config.forward_linear_speed_m_s
                );
            }
        }
    }

    /// Handle the outcome of stability sweeps.
    fn handle_stability_outcome(
        &mut self,
        overall_stable: bool,
        sweep_count: usize,
    ) -> Result<(), Error> {
        if overall_stable {
            info!("Tip is stable after {} sweep(s)", sweep_count);
            self.current_tip_shape = TipShape::Stable;
        } else {
            info!(
                "Stability check failed - executing max voltage pulse to reshape tip"
            );

            self.execute_max_pulse()?;

            self.set_action(ControllerAction::Repositioning);
            self.driver
                .run(Action::SafeReposition {
                    x_steps: self.config.reposition_steps.0,
                    y_steps: self.config.reposition_steps.1,
                })
                .go()?;

            info!(
                "Restarting tip preparation from beginning after stability failure"
            );
            self.current_tip_shape = TipShape::Blunt;
        }

        Ok(())
    }

    /// Check reliability of tip state and return baseline freq_shift for stability comparison.
    fn pre_good_loop_check(
        &mut self,
    ) -> Result<(TipShape, Option<f32>), Error> {
        log::info!("Checking reliability of tip state result");

        let mut last_freq_shift: Option<f32> = None;

        for i in 0..3 {
            self.check_shutdown().map_err(|_| {
                log::info!(
                    "Shutdown requested during pre_good_loop_check at iteration {}/3",
                    i + 1
                );
                Error::Shutdown
            })?;

            self.set_action(ControllerAction::Repositioning);
            self.driver
                .run(Action::SafeReposition {
                    x_steps: self.config.reposition_steps.0,
                    y_steps: self.config.reposition_steps.1,
                })
                .go()?;

            self.check_shutdown().map_err(|_| {
                log::info!(
                    "Shutdown requested after reposition in pre_good_loop_check"
                );
                Error::Shutdown
            })?;

            self.set_action(ControllerAction::MeasuringSignal);
            let tip_state: TipState = self
                .driver
                .run(Action::CheckTipState {
                    method: TipCheckMethod::SignalBounds {
                        signal: self.config.freq_shift_signal.clone(),
                        bounds: self.config.sharp_tip_bounds,
                    },
                })
                .expecting()?;

            if let Some(freq_shift_value) = tip_state
                .measured_signals
                .get(&SignalIndex::new(self.config.freq_shift_signal.index))
                .copied()
            {
                last_freq_shift = Some(freq_shift_value);
                let signal = self.config.freq_shift_signal.clone();
                self.track_signal(&signal, freq_shift_value);
            }

            if matches!(tip_state.shape, TipShape::Blunt) {
                self.measured_freq_shift = last_freq_shift;
                return Ok((TipShape::Blunt, None));
            }
        }

        log::info!(
            "Baseline freq_shift for stability check: {:?}",
            last_freq_shift
        );
        Ok((TipShape::Sharp, last_freq_shift))
    }

    fn pre_loop_initialization(&mut self) -> Result<(), Error> {
        log::info!("Running pre loop initialization");

        // Load layout file if specified
        if let Some(layout_path) = self.config.layout_file.clone() {
            self.set_action(ControllerAction::LoadingLayout);
            let abs_path =
                Path::new(&layout_path).canonicalize().map_err(|e| {
                    Error::Nanonis(crate::NanonisError::Protocol(format!(
                        "Layout file not found: {} ({})",
                        layout_path, e
                    )))
                })?;
            let abs_path_str = abs_path.to_string_lossy();
            info!("Loading layout from: {}", abs_path_str);
            self.driver
                .client_mut()
                .util_layout_load(&abs_path_str, false)?;
            info!("Layout loaded successfully");
        }

        // Load settings file if specified
        if let Some(settings_path) = self.config.settings_file.clone() {
            self.set_action(ControllerAction::LoadingSettings);
            let abs_path =
                Path::new(&settings_path).canonicalize().map_err(|e| {
                    Error::Nanonis(crate::NanonisError::Protocol(format!(
                        "Settings file not found: {} ({})",
                        settings_path, e
                    )))
                })?;
            let abs_path_str = abs_path.to_string_lossy();
            info!("Loading settings from: {}", abs_path_str);
            self.driver
                .client_mut()
                .util_settings_load(&abs_path_str, false)?;
            info!("Settings loaded successfully");
        }

        self.set_action(ControllerAction::SettingBias);
        self.driver
            .run(Action::SetBias {
                voltage: self.config.initial_bias_v,
            })
            .go()?;

        self.set_action(ControllerAction::SettingSetpoint);
        self.driver
            .run(Action::SetZSetpoint {
                setpoint: self.config.initial_z_setpoint_a,
            })
            .go()?;

        // Set homing config TODO: move parameter to config
        let home_position_m = 50e-9;
        self.driver
            .client_mut()
            .z_ctrl_home_props_set(ZHomeMode::Absolute, home_position_m)?;

        // Set correct safe tip config
        let safe_tip_threshold = self.config.safe_tip_threshold;
        self.driver.client_mut().safe_tip_props_set(
            false,
            true,
            safe_tip_threshold,
        )?;

        // Update some random User Output to update TCP Channel List
        // Should be fixed in next Nanonis Software Update
        let output_to_toggle = 3;
        let current_mode = self
            .driver
            .client_mut()
            .user_out_mode_get(output_to_toggle)?;

        match current_mode {
            nanonis_rs::user_out::OutputMode::UserOutput => {
                self.driver.client_mut().user_out_mode_set(
                    output_to_toggle,
                    nanonis_rs::user_out::OutputMode::Monitor,
                )?;
                self.driver
                    .client_mut()
                    .user_out_mode_set(output_to_toggle, current_mode)?;
            }
            nanonis_rs::user_out::OutputMode::CalcSignal => {
                self.driver.client_mut().user_out_mode_set(
                    output_to_toggle,
                    nanonis_rs::user_out::OutputMode::UserOutput,
                )?;
                self.driver
                    .client_mut()
                    .user_out_mode_set(output_to_toggle, current_mode)?;
            }
            nanonis_rs::user_out::OutputMode::Monitor => {
                self.driver.client_mut().user_out_mode_set(
                    output_to_toggle,
                    nanonis_rs::user_out::OutputMode::CalcSignal,
                )?;
                self.driver
                    .client_mut()
                    .user_out_mode_set(output_to_toggle, current_mode)?;
            }
            nanonis_rs::user_out::OutputMode::Override => {
                self.driver.client_mut().user_out_mode_set(
                    output_to_toggle,
                    nanonis_rs::user_out::OutputMode::Monitor,
                )?;
                self.driver
                    .client_mut()
                    .user_out_mode_set(output_to_toggle, current_mode)?;
            }
        }

        info!("Executing Initial Approach");
        self.set_action(ControllerAction::Approaching);

        self.driver
            .run(Action::AutoApproach {
                wait_until_finished: true,
                timeout: Duration::from_secs(600),
                center_freq_shift: true,
            })
            .go()?;

        // Clear TCP buffer to discard any stale data from before approach
        log::debug!("Clearing TCP buffer to get fresh frequency shift data");
        self.driver.clear_tcp_buffer();

        // Wait briefly for fresh data to accumulate in buffer
        self.driver
            .run(Action::Wait {
                duration: self.config.buffer_clear_wait,
            })
            .go()?;

        // Wait for signal to stabilize after approach
        info!(
            "Waiting {:?} for signal to stabilize after approach...",
            self.config.post_approach_settle
        );
        self.driver
            .run(Action::Wait {
                duration: self.config.post_approach_settle,
            })
            .go()?;

        let initial_tip_state: TipState = self
            .driver
            .run(Action::CheckTipState {
                method: TipCheckMethod::SignalBounds {
                    signal: self.config.freq_shift_signal.clone(),
                    bounds: self.config.sharp_tip_bounds,
                },
            })
            .expecting()?;

        info!("Current tip shape: {:?}", initial_tip_state.shape);
        self.current_tip_shape = initial_tip_state.shape;

        Ok(())
    }
}
