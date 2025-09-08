use crate::action_driver::ActionDriver;
use crate::actions::{Action, ActionChain};
use crate::error::NanonisError;
use crate::job::Job;
use crate::types::{DataToGet, MotorDirection, SignalIndex};
use log::{debug, info, warn};
use std::time::{Duration, Instant};

/// Simple tip state - matches original controller
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// Enhanced tip controller with pulse voltage stepping
pub struct TipController {
    driver: ActionDriver,
    signal_index: SignalIndex,

    // Pulse stepping parameters
    pulse_voltage: f32,
    pulse_voltage_step: f32,
    change_threshold: Box<dyn Fn(f32) -> f32>,
    cycles_before_step: u32,
    min_pulse_voltage: f32,
    max_pulse_voltage: f32,

    // Step tracking
    cycles_without_change: u32,

    // Signal bounds and thresholds
    min_bound: f32,
    max_bound: f32,

    // State tracking
    good_count: u32,
    stable_threshold: u32,

    // History for bias adjustment
    signal_history: Vec<f32>,
    max_history_size: usize,
}

impl TipController {
    /// Create new tip controller with basic signal bounds
    pub fn new(
        driver: ActionDriver,
        signal_index: SignalIndex,
        pulse_voltage: f32,
        min_bound: f32,
        max_bound: f32,
    ) -> Self {
        Self {
            driver,
            signal_index,
            pulse_voltage,
            pulse_voltage_step: 0.1,
            change_threshold: Box::new(|_| 0.1),
            cycles_before_step: 3,
            min_pulse_voltage: pulse_voltage,
            max_pulse_voltage: 5.0, // Maximum pulse voltage
            cycles_without_change: 0,
            min_bound,
            max_bound,
            good_count: 0,
            stable_threshold: 3, // 3 consecutive good readings = stable
            signal_history: Vec::new(),
            max_history_size: 10, // Keep last 10 signal readings
        }
    }

    /// Set pulse stepping parameters with closure-based threshold
    pub fn set_pulse_stepping(
        &mut self,
        pulse_step: f32,
        change_threshold: Box<dyn Fn(f32) -> f32>,
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

    /// Set stability threshold (how many good readings needed for stable)
    pub fn set_stability_threshold(&mut self, threshold: u32) -> &mut Self {
        self.stable_threshold = threshold.max(1); // At least 1
        self
    }

    /// Get current pulse voltage
    pub fn current_pulse_voltage(&self) -> f32 {
        self.pulse_voltage
    }

    /// Get signal history (most recent first)
    pub fn signal_history(&self) -> &[f32] {
        &self.signal_history
    }

    /// Calculate average of recent signals
    pub fn average_signal(&self) -> Option<f32> {
        if self.signal_history.is_empty() {
            None
        } else {
            Some(self.signal_history.iter().sum::<f32>() / self.signal_history.len() as f32)
        }
    }

    /// Update signal history with new signal value
    fn update_signal_history(&mut self, signal: f32) {
        self.signal_history.insert(0, signal);
        if self.signal_history.len() > self.max_history_size {
            self.signal_history.truncate(self.max_history_size);
        }
    }

    /// Check if current signal represents a significant change from recent stable period
    fn has_significant_change(&self, signal: f32) -> bool {
        if self.signal_history.len() < 2 {
            // First signal - consider it a significant change to initialize properly
            true
        } else {
            // Compare only against signals from the current stable period
            // cycles_without_change tells us how many recent signals were stable
            let stable_period_size = (self.cycles_without_change as usize).min(self.signal_history.len() - 1);
            
            if stable_period_size == 0 {
                // No stable period yet, compare against last signal
                let last_signal = self.signal_history[1];
                (signal - last_signal).abs() >= (self.change_threshold)(signal)
            } else {
                // Compare against mean of current stable period (skip current signal at index 0)
                let stable_signals = &self.signal_history[1..=stable_period_size];
                let stable_mean = stable_signals.iter().sum::<f32>() / stable_signals.len() as f32;
                (signal - stable_mean).abs() >= (self.change_threshold)(signal)
            }
        }
    }

    /// Handle response to significant signal change
    fn handle_significant_change(&mut self, signal: f32) {
        let comparison_signal = self.signal_history.get(1).unwrap_or(&0.0);
        debug!(
            "Signal changed significantly: {:.3} -> {:.3} (change: {:.3})",
            comparison_signal,
            signal,
            (signal - comparison_signal).abs()
        );
        self.cycles_without_change = 0;
        self.pulse_voltage = self.min_pulse_voltage;
    }

    /// Handle response to stable signal (no significant change)
    fn handle_stable_signal(&mut self, signal: f32) {
        self.cycles_without_change += 1;
        debug!(
            "Signal unchanged: {:.3}, cycles without change: {}/{}",
            signal, self.cycles_without_change, self.cycles_before_step
        );

        // Check if we need to step the pulse voltage
        if self.cycles_without_change >= self.cycles_before_step {
            self.step_pulse_voltage();
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
    fn update_signal_and_pulse(&mut self, signal: f32) {
        // 1. Update signal history
        self.update_signal_history(signal);
        
        // 2. Check for significant change and respond accordingly
        if self.has_significant_change(signal) {
            self.handle_significant_change(signal);
        } else {
            self.handle_stable_signal(signal);
        }
    }
}

impl TipController {
    /// Main control loop - with pulse voltage stepping
    pub fn run_loop(&mut self, timeout: Duration) -> Result<TipState, NanonisError> {
        info!(
            "Starting tip control loop with pulse stepping (timeout: {:?})",
            timeout
        );

        let start = Instant::now();
        let mut cycle = 0;

        while start.elapsed() < timeout {
            cycle += 1;

            // Read signal (with error handling)
            let signal = self.read_signal()?;

            // Update signal history and step pulse voltage if needed
            self.update_signal_and_pulse(signal);

            info!(
                "Cycle {}: Signal = {:.6}, Pulse = {:.3}V, Cycles w/o change = {}/{}",
                cycle,
                signal,
                self.pulse_voltage,
                self.cycles_without_change,
                self.cycles_before_step
            );

            // Classify
            let state = self.classify(signal);
            info!("Cycle {}: State = {:?}", cycle, state);

            // Execute based on state
            match state {
                TipState::Bad => {
                    self.bad_loop(cycle)?; // Execute full recovery sequence
                }
                TipState::Good => {
                    self.good_loop(cycle)?; // Monitor and count
                }
                TipState::Stable => {
                    info!(
                        "STABLE achieved after {} cycles! Final pulse voltage: {:.3}V",
                        cycle, self.pulse_voltage
                    );
                    return Ok(TipState::Stable);
                }
            }

            std::thread::sleep(Duration::from_millis(500)); // Match original timing
        }

        warn!("Tip control loop timed out");
        Err(NanonisError::InvalidCommand("Loop timeout".to_string()))
    }

    /// Bad loop - execute original controller recovery sequence
    /// Sequence: approach → pulse → withdraw → move → approach → check
    fn bad_loop(&mut self, cycle: u32) -> Result<(), NanonisError> {
        info!("Cycle {}: Executing bad signal recovery sequence", cycle);

        // Reset good count
        self.good_count = 0;

        self.driver.execute_chain(ActionChain::new(vec![
            Action::BiasPulse {
                wait_until_done: true,
                pulse_width_s: Duration::from_millis(50),
                bias_value_v: self.pulse_voltage,
                z_controller_hold: 1,
                pulse_mode: 2,
            },
            Action::Withdraw {
                wait_until_finished: true,
                timeout_ms: Duration::from_secs(5),
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
            Action::AutoApproach,
            Action::Wait {
                duration: Duration::from_secs(1),
            },
        ]))?;

        info!(
            "Cycle {}: Recovery sequence completed - checking tip state",
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
        if signal < self.min_bound || signal > self.max_bound {
            TipState::Bad
        } else if self.good_count >= self.stable_threshold {
            TipState::Stable
        } else {
            TipState::Good
        }
    }

    /// Read signal value
    fn read_signal(&mut self) -> Result<f32, NanonisError> {
        self.driver.spm_interface_mut().osci1t_run()?;

        self.driver
            .spm_interface_mut()
            .osci1t_ch_set(self.signal_index.into())?;

        let (_, _, _, osci_screen) = self
            .driver
            .spm_interface_mut()
            .osci1t_data_get(DataToGet::NextTrigger)?;

        debug!("Osci_data: {osci_screen:?}");
        let min: f32 = osci_screen.iter().fold(f64::INFINITY, |a, &b| a.min(b)) as f32;
        debug!("Result of osci max = {min:?}");

        Ok(min)
    }
}

// Implement Job trait for TipController
impl Job for TipController {
    type Output = TipState;

    fn run(&mut self, timeout: Duration) -> Result<Self::Output, NanonisError> {
        self.run_loop(timeout)
    }
}
