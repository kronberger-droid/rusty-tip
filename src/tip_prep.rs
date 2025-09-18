use crate::action_driver::ActionDriver;
use crate::actions::{Action, ActionChain};
use crate::error::NanonisError;
use crate::job::Job;
use crate::types::{DataToGet, MotorDirection, SignalIndex};
use crate::{stability, Logger};
use log::{debug, info, warn};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Simple tip state - matches original controller
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum TipState {
    Bad,
    Good,
    Stable,
}

/// Loop types based on tip state - simple and direct
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoopType {
    BadLoop,
    GoodLoop,
    StableLoop,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LogLine {
    cycle: u32,
    freq_shift: f32,
    tip_state: TipState,
    pulse_voltage: f32,
    freq_shift_change: Option<f32>,
    z_change: Option<f32>,
}

/// Enhanced tip controller with pulse voltage stepping
pub struct TipController {
    driver: ActionDriver,
    signal_index: SignalIndex,

    // Pulse stepping parameters
    pulse_voltage: f32,
    pulse_voltage_step: f32,
    change_threshold: Box<dyn Fn(f32) -> f32 + Send + Sync>,
    cycles_before_step: u32,
    min_pulse_voltage: f32,
    max_pulse_voltage: f32,

    // Step tracking
    cycles_without_change: u32,

    // Signal bounds and thresholds
    bound: (f32, f32),

    // State tracking
    good_count: u32,
    stable_threshold: u32,
    cycle_count: u32,

    // Multi-signal history for bias adjustment and analysis
    signal_histories: HashMap<SignalIndex, VecDeque<f32>>,
    max_history_size: usize,

    // Json Logger
    logger: Option<Logger<LogLine>>,
}

impl TipController {
    /// Create new tip controller with basic signal bounds
    pub fn new(
        driver: ActionDriver,
        signal_index: SignalIndex,
        pulse_voltage: f32,
        bound: (f32, f32),
    ) -> Self {
        Self {
            driver,
            signal_index,
            pulse_voltage,
            pulse_voltage_step: 0.1,
            change_threshold: Box::new(|_| 0.1),
            cycles_before_step: 3,
            min_pulse_voltage: pulse_voltage,
            max_pulse_voltage: 5.0,
            cycles_without_change: 0,
            bound,
            good_count: 0,
            stable_threshold: 3,
            cycle_count: 0,
            signal_histories: HashMap::new(),
            max_history_size: 10,
            logger: None,
        }
    }

    /// Set pulse stepping parameters with closure-based threshold
    pub fn set_pulse_stepping(
        &mut self,
        pulse_step: f32,
        change_threshold: Box<dyn Fn(f32) -> f32 + Send + Sync>,
        cycles_before_step: u32,
        max_pulse: f32,
    ) -> &mut Self {
        self.pulse_voltage_step = pulse_step.abs(); // Ensure positive
        self.change_threshold = change_threshold;
        self.cycles_before_step = cycles_before_step.max(1); // At least 1
        self.max_pulse_voltage = max_pulse.abs();
        self
    }

    /// Set pulse stepping parameters with fixed threshold (convenience method)
    pub fn set_pulse_stepping_fixed(
        &mut self,
        pulse_step: f32,
        change_threshold: f32,
        cycles_before_step: u32,
        max_pulse: f32,
    ) -> &mut Self {
        let threshold = change_threshold.abs();
        self.set_pulse_stepping(
            pulse_step,
            Box::new(move |_| threshold),
            cycles_before_step,
            max_pulse,
        )
    }

    /// Provide Json File logger for inspecting behavior
    pub fn with_logger(&mut self, logger: Logger<LogLine>) -> &mut Self {
        self.logger = Some(logger);
        self
    }

    /// Flush the logger (useful for signal handlers)
    pub fn flush_logger(&mut self) -> Result<(), NanonisError> {
        if let Some(ref mut logger) = self.logger {
            logger.flush()?;
        }
        Ok(())
    }

    /// Set stability threshold (how many good readings needed for stable)
    pub fn set_stability_threshold(&mut self, threshold: u32) -> &mut Self {
        self.stable_threshold = threshold.max(1); // At least 1
        self
    }

    /// Get current pulse voltage
    pub fn current_pulse_voltage(&self) -> f32 {
        self.pulse_voltage
    }

    /// Get signal history (most recent first) for the frequency shift signal
    pub fn signal_history(&self) -> Option<&VecDeque<f32>> {
        self.get_signal_history(self.signal_index)
    }

    /// Calculate average of recent signals for the frequency shift signal
    pub fn average_signal(&self) -> Option<f32> {
        self.average_signal_for(self.signal_index)
    }

    /// Calculate average of recent signals for a specific signal
    pub fn average_signal_for(&self, signal_index: SignalIndex) -> Option<f32> {
        if let Some(history) = self.signal_histories.get(&signal_index) {
            if history.is_empty() {
                None
            } else {
                Some(history.iter().sum::<f32>() / history.len() as f32)
            }
        } else {
            None
        }
    }

    /// Track a signal value in history
    pub fn track_signal(&mut self, signal_index: SignalIndex, value: f32) {
        let history = self.signal_histories.entry(signal_index).or_default();

        // Add new value to front
        history.push_front(value);

        // Maintain size limit
        while history.len() > self.max_history_size {
            history.pop_back();
        }
    }

    /// Get signal change (latest - previous) for a specific signal
    pub fn get_signal_change(&self, signal_index: SignalIndex) -> Option<f32> {
        if let Some(history) = self.signal_histories.get(&signal_index) {
            if history.len() >= 2 {
                Some(history[0] - history[1]) // Latest - Previous
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Get signal history for a specific signal (most recent first)
    pub fn get_signal_history(&self, signal_index: SignalIndex) -> Option<&VecDeque<f32>> {
        self.signal_histories.get(&signal_index)
    }

    pub fn get_last_signal(&self, signal_index: SignalIndex) -> Option<f32> {
        match self.get_signal_history(signal_index) {
            Some(history) => history.iter().last().copied(),
            None => None,
        }
    }

    /// Clear all signal histories (useful for logger integration)
    pub fn clear_all_histories(&mut self) {
        self.signal_histories.clear();
    }

    /// Clear history for a specific signal
    pub fn clear_signal_history(&mut self, signal_index: SignalIndex) {
        self.signal_histories.remove(&signal_index);
    }

    /// Check if current signal represents a significant change from recent stable period
    fn has_significant_change(&self, signal_index: SignalIndex) -> (bool, f32) {
        if let Some(history) = self.signal_histories.get(&signal_index) {
            if history.len() < 2 {
                // First signal - consider it a significant change to initialize properly
                (true, 0.0)
            } else {
                // Compare only against signals from the current stable period
                // cycles_without_change tells us how many recent signals were stable
                let stable_period_size =
                    (self.cycles_without_change as usize).min(history.len() - 1);

                if stable_period_size == 0 {
                    // No stable period yet, compare against last signal
                    let signal = history[0];
                    let last_signal = history[1];
                    info!(
                        "Last signal: {} | Current threshold: {}",
                        last_signal,
                        (self.change_threshold)(signal)
                    );
                    let has_change =
                        (signal - last_signal).abs() >= (self.change_threshold)(signal);

                    (has_change, (signal - last_signal))
                } else {
                    // Compare against mean of current stable period (skip current signal at index 0)
                    let signal = history[0];
                    let stable_signals: Vec<f32> = history
                        .iter()
                        .skip(1)
                        .take(stable_period_size)
                        .cloned()
                        .collect();
                    let stable_mean =
                        stable_signals.iter().sum::<f32>() / stable_signals.len() as f32;

                    info!(
                        "Stable mean: {} | Current threshold: {}",
                        stable_mean,
                        (self.change_threshold)(signal)
                    );
                    let has_change =
                        (signal - stable_mean).abs() >= (self.change_threshold)(signal);
                    (has_change, (signal - stable_mean))
                }
            }
        } else {
            // No history yet - consider it a significant change
            (true, 0.0)
        }
    }

    /// Step up the pulse voltage if possible
    fn step_pulse_voltage(&mut self) -> bool {
        let new_pulse = (self.pulse_voltage + self.pulse_voltage_step).min(self.max_pulse_voltage);
        if new_pulse > self.pulse_voltage {
            info!(
                "Stepping pulse voltage: {:.3}V -> {:.3}V",
                self.pulse_voltage, new_pulse
            );
            self.pulse_voltage = new_pulse;
            self.cycles_without_change = 0; // Reset counter after stepping
            true
        } else {
            debug!(
                "Pulse voltage already at maximum: {:.3}V",
                self.max_pulse_voltage
            );
            self.cycles_without_change = 0; // Reset counter even if at max
            false
        }
    }

    /// Update signal history and step pulse voltage if needed
    fn update_pulse_voltage(&mut self) {
        let (is_significant, change) = self.has_significant_change(self.signal_index);

        // 2. Check for significant change and respond accordingly
        if is_significant && change >= 0.0 {
            self.cycles_without_change = 0;
            self.pulse_voltage = self.min_pulse_voltage;
        } else if is_significant {
            warn!("Positive change significant change!");
            self.cycles_without_change += 1;

            // Check if we need to step the pulse voltage
            if self.cycles_without_change >= self.cycles_before_step {
                self.step_pulse_voltage();
            }
        } else {
            self.cycles_without_change += 1;

            // Check if we need to step the pulse voltage
            if self.cycles_without_change >= self.cycles_before_step {
                self.step_pulse_voltage();
            }
        }
    }
}

impl TipController {
    /// Main control loop - with pulse voltage stepping
    pub fn run_loop(&mut self, timeout: Duration) -> Result<TipState, NanonisError> {
        let start = Instant::now();
        let mut freq_shift;
        let mut tip_state;

        self.pre_loop_initialization()?;

        let z_signal_index = SignalIndex(30);

        while start.elapsed() < timeout {
            self.cycle_count += 1;

            let ampl_setpoint = self.driver.client_mut().pll_amp_ctrl_setpnt_get(1)?;
            let ampl_current = self.driver.client_mut().signal_val_get(75, true)?;

            if (ampl_setpoint - 5e-12..ampl_setpoint + 5e-12).contains(&ampl_current) {
                info!("Amplitude reached the target range");
                self.read_and_track_freq_shift()?;

                freq_shift = self
                    .get_last_signal(self.signal_index)
                    .expect("Should have failed earlier");

                self.update_pulse_voltage();

                tip_state = self.classify(freq_shift);
            } else {
                info!("Amplitude did not reach the target range");
                freq_shift = -76.3; // add client call
                tip_state = TipState::Bad;

                self.track_signal(self.signal_index, freq_shift);

                self.update_pulse_voltage();
            }

            info!("Cycle {}: State = {:?}", self.cycle_count, tip_state);

            self.read_and_track_z_pos(z_signal_index)?;

            info!(
                "Cycle {}: Freq Shift = {:.6?}, Pulse = {:.3}V, Cycles w/o change = {}/{}",
                self.cycle_count,
                freq_shift,
                self.pulse_voltage,
                self.cycles_without_change,
                self.cycles_before_step
            );

            // Execute based on state
            match tip_state {
                TipState::Bad => {
                    self.bad_loop(self.cycle_count)?; // Execute full recovery sequence
                }
                TipState::Good => {
                    self.good_loop(self.cycle_count)?; // Monitor and count
                }
                TipState::Stable => {
                    info!(
                        "STABLE achieved after {} cycles! Final pulse voltage: {:.3}V",
                        self.cycle_count, self.pulse_voltage
                    );
                    return Ok(TipState::Stable);
                }
            }

            // Add information about this cycle to the logger buffer
            if self.logger.is_some() {
                // Calculate changes before borrowing logger mutably
                let freq_shift_change = self.get_signal_change(self.signal_index);
                let z_change = self.get_signal_change(z_signal_index);

                if let Some(ref mut logger) = self.logger {
                    logger.add(LogLine {
                        cycle: self.cycle_count,
                        freq_shift,
                        tip_state,
                        pulse_voltage: self.pulse_voltage,
                        freq_shift_change,
                        z_change,
                    })?
                }
            }
        }

        debug!("Tip control loop reached timeout");
        Err(NanonisError::InvalidCommand("Loop timeout".to_string()))
    }

    /// Bad loop - execute recovery sequence with stable signal monitoring
    /// Sequence: capture_stable_before → pulse → capture_stable_after → withdraw → move → approach → check
    fn bad_loop(&mut self, cycle: u32) -> Result<(), NanonisError> {
        info!(
            "Cycle {}: Executing bad signal recovery sequence with stability detection",
            cycle
        );

        let z_signal_index = SignalIndex(30); // Z (m) signal index

        // Reset good count
        self.good_count = 0;

        // Execute bias pulse
        info!(
            "Cycle {}: Executing bias pulse at {:.3}V",
            cycle, self.pulse_voltage
        );
        self.driver.execute(Action::BiasPulse {
            wait_until_done: true,
            pulse_width: Duration::from_millis(50),
            bias_value_v: self.pulse_voltage,
            z_controller_hold: 1,
            pulse_mode: 2,
        })?;

        self.read_and_track_z_pos(z_signal_index)?;

        // Continue with rest of recovery sequence
        info!("Cycle {}: Continuing with withdraw and movement...", cycle);
        self.driver.execute_chain(ActionChain::new(vec![
            Action::Withdraw {
                wait_until_finished: true,
                timeout: Duration::from_secs(5),
            },
            Action::MoveMotor {
                direction: MotorDirection::ZMinus,
                steps: 3,
            },
            Action::MoveMotor {
                direction: MotorDirection::XPlus,
                steps: 2,
            },
            Action::MoveMotor {
                direction: MotorDirection::YPlus,
                steps: 2,
            },
            Action::AutoApproach {
                wait_until_finished: true,
                timeout: Duration::from_secs(5),
            },
            Action::Wait {
                duration: Duration::from_secs(1),
            },
        ]))?;

        info!(
            "Cycle {}: Recovery sequence completed - checking tip state... \n",
            cycle
        );

        Ok(())
    }

    /// Good loop - monitoring, increment good count
    fn good_loop(&mut self, cycle: u32) -> Result<(), NanonisError> {
        self.good_count += 1;
        debug!("Cycle {}: Good signal (count: {})", cycle, self.good_count);
        // Just wait and continue monitoring
        Ok(())
    }

    /// Simple classification based on bounds
    fn classify(&mut self, signal: f32) -> TipState {
        if signal < self.bound.0 || signal > self.bound.1 {
            TipState::Bad
        } else if self.good_count >= self.stable_threshold {
            TipState::Stable
        } else {
            TipState::Good
        }
    }

    fn pre_loop_initialization(&mut self) -> Result<(), NanonisError> {
        self.driver.client_mut().set_bias(-500e-3)?;

        self.driver.client_mut().z_ctrl_setpoint_set(100e-12)?;

        self.driver.client_mut().auto_approach_and_wait()?;

        Ok(())
    }

    fn read_and_track_freq_shift(&mut self) -> Result<(), NanonisError> {
        let freq_shift;

        if let Some(freq_shift_frame) = self.driver.read_oscilloscope_with_stability(
            self.signal_index,
            None,
            DataToGet::Stable {
                readings: 5,
                timeout: Duration::from_secs(10),
            },
            stability::dual_threshold_stability,
        )? {
            crate::plot_values(
                &freq_shift_frame.data,
                Some("Frequency Shift Oscilloscope Frame"),
                None,
                None,
            )
            .unwrap();
            freq_shift =
                freq_shift_frame.data.iter().sum::<f64>() as f32 / freq_shift_frame.size as f32;
        } else {
            warn!("Using single value read fallback for frequency shift");
            let result = self.driver.execute(Action::ReadSignal {
                signal: self.signal_index,
                wait_for_newest: true,
            })?;
            freq_shift = result
                .as_f64()
                .expect("Must be able to Read from Interface") as f32;
        }

        self.track_signal(self.signal_index, freq_shift);

        Ok(())
    }
    fn read_and_track_z_pos(&mut self, z_signal_index: SignalIndex) -> Result<(), NanonisError> {
        let z_pos;

        if let Some(z_pos_frame) = self.driver.read_oscilloscope_with_stability(
            z_signal_index,
            None,
            DataToGet::Stable {
                readings: 5,
                timeout: Duration::from_secs(10),
            },
            stability::dual_threshold_stability,
        )? {
            // crate::plot_values(
            //     &z_pos_frame.data,
            //     Some("Z-Position Oscilloscope Frame"),
            //     None,
            //     None,
            // )
            // .unwrap();

            z_pos = z_pos_frame.data.iter().sum::<f64>() as f32 / z_pos_frame.size as f32;
        } else {
            warn!("Using read single signal fallback for z position");
            let result = self.driver.execute(Action::ReadSignal {
                signal: self.signal_index,
                wait_for_newest: true,
            })?;
            z_pos = result
                .as_f64()
                .expect("Must be able to Read from Interface") as f32;
        }

        self.track_signal(z_signal_index, z_pos);

        Ok(())
    }
}

// Implement Job trait for TipController
impl Job for TipController {
    type Output = TipState;

    fn run(&mut self, timeout: Duration) -> Result<Self::Output, NanonisError> {
        self.run_loop(timeout)
    }
}
