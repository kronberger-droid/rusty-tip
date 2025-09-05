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
    BadLoop,    // Recovery actions
    GoodLoop,   // Monitoring, building to stable
    StableLoop, // Success condition
}

/// Enhanced tip controller with pulse voltage stepping
pub struct TipController {
    driver: ActionDriver,
    signal_index: SignalIndex,
    
    // Pulse stepping parameters
    pulse_voltage: f32,              // Current pulse voltage (gets stepped up)
    pulse_voltage_step: f32,         // How much to increase pulse voltage per step
    change_threshold: f32,           // Minimum signal change to consider significant
    cycles_before_step: u32,         // Cycles without change before stepping pulse voltage
    max_pulse_voltage: f32,          // Maximum pulse voltage allowed
    
    // Step tracking
    cycles_without_change: u32,
    last_significant_signal: f32,
    
    // Signal bounds and thresholds
    min_bound: f32,
    max_bound: f32,
    target_signal: f32,         // Ideal signal value (center of bounds)
    
    // State tracking
    good_count: u32,
    stable_threshold: u32,
    move_count: u32,
    max_moves: u32,
    
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
        let target_signal = (min_bound + max_bound) / 2.0;
        
        Self {
            driver,
            signal_index,
            pulse_voltage,
            pulse_voltage_step: 0.1,    // Default 0.1V pulse steps
            change_threshold: 0.05,     // Signal must change by at least 0.05 
            cycles_before_step: 3,      // 3 cycles without change triggers pulse step
            max_pulse_voltage: 5.0,     // Maximum pulse voltage
            cycles_without_change: 0,
            last_significant_signal: 0.0,
            min_bound,
            max_bound,
            target_signal,
            good_count: 0,
            stable_threshold: 3,  // 3 consecutive good readings = stable
            move_count: 0,
            max_moves: 10,        // Max moves before withdraw/approach
            signal_history: Vec::new(),
            max_history_size: 10, // Keep last 10 signal readings
        }
    }
    
    
    /// Set pulse stepping parameters
    pub fn set_pulse_stepping(&mut self, pulse_step: f32, change_threshold: f32, cycles_before_step: u32, max_pulse: f32) -> &mut Self {
        self.pulse_voltage_step = pulse_step.abs(); // Ensure positive
        self.change_threshold = change_threshold.abs();
        self.cycles_before_step = cycles_before_step.max(1); // At least 1
        self.max_pulse_voltage = max_pulse.abs();
        self
    }
    
    /// Set stability threshold (how many good readings needed for stable)
    pub fn set_stability_threshold(&mut self, threshold: u32) -> &mut Self {
        self.stable_threshold = threshold.max(1); // At least 1
        self
    }
    
    /// Set maximum moves before giving up
    pub fn set_max_moves(&mut self, max_moves: u32) -> &mut Self {
        self.max_moves = max_moves;
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
    
    /// Update signal history and step pulse voltage if needed
    fn update_signal_and_pulse(&mut self, signal: f32) {
        // Add to history
        self.signal_history.insert(0, signal);
        if self.signal_history.len() > self.max_history_size {
            self.signal_history.truncate(self.max_history_size);
        }
        
        // Check if signal has changed significantly since last significant change
        let signal_change = (signal - self.last_significant_signal).abs();
        
        if signal_change >= self.change_threshold {
            // Signal changed significantly - reset cycle counter
            debug!("Signal changed significantly: {:.3} -> {:.3} (change: {:.3})", 
                   self.last_significant_signal, signal, signal_change);
            self.last_significant_signal = signal;
            self.cycles_without_change = 0;
        } else {
            // Signal hasn't changed enough - increment counter
            self.cycles_without_change += 1;
            debug!("Signal unchanged: {:.3}, cycles without change: {}/{}", 
                   signal, self.cycles_without_change, self.cycles_before_step);
            
            // Check if we need to step the pulse voltage
            if self.cycles_without_change >= self.cycles_before_step {
                let new_pulse = (self.pulse_voltage + self.pulse_voltage_step).min(self.max_pulse_voltage);
                if new_pulse > self.pulse_voltage {
                    info!("Stepping pulse voltage: {:.3}V -> {:.3}V", self.pulse_voltage, new_pulse);
                    self.pulse_voltage = new_pulse;
                } else {
                    debug!("Pulse voltage already at maximum: {:.3}V", self.max_pulse_voltage);
                }
                self.cycles_without_change = 0; // Reset counter after stepping
            }
        }
    }
    
}


impl TipController {
    /// Main control loop - with pulse voltage stepping
    pub fn run_loop(&mut self, timeout: Duration) -> Result<TipState, NanonisError> {
        info!("Starting tip control loop with pulse stepping (timeout: {:?})", timeout);

        let start = Instant::now();
        let mut cycle = 0;

        while start.elapsed() < timeout {
            cycle += 1;

            // 1. Read signal (with error handling)
            let signal = match self.read_signal() {
                Ok(s) => s,
                Err(e) => {
                    warn!("Cycle {}: Failed to read signal: {}", cycle, e);
                    std::thread::sleep(Duration::from_millis(500));
                    continue; // Skip this cycle
                }
            };
            
            // 2. Initialize first signal reference if needed
            if self.last_significant_signal == 0.0 {
                self.last_significant_signal = signal;
                debug!("Initialized first signal reference: {:.3}", signal);
            }
            
            // 3. Update signal history and step pulse voltage if needed
            self.update_signal_and_pulse(signal);
            
            info!("Cycle {}: Signal = {:.6}, Pulse = {:.3}V, Cycles w/o change = {}/{}", 
                  cycle, signal, self.pulse_voltage, self.cycles_without_change, self.cycles_before_step);

            // 4. Classify
            let state = self.classify(signal);
            info!("Cycle {}: State = {:?}", cycle, state);

            // 5. Execute based on state
            match state {
                TipState::Bad => {
                    self.bad_loop(cycle)?; // Execute full recovery sequence
                }
                TipState::Good => {
                    self.good_loop(cycle)?; // Monitor and count
                }
                TipState::Stable => {
                    info!("STABLE achieved after {} cycles! Final pulse voltage: {:.3}V", cycle, self.pulse_voltage);
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
        self.driver
            .spm_interface_mut()
            .osci1t_ch_set(self.signal_index.into())?;

        let (_, _, _, osci_screen) = self
            .driver
            .spm_interface_mut()
            .osci1t_data_get(DataToGet::NextTrigger)?;

        debug!("Osci_data: {osci_screen:?}");
        let max: f32 = osci_screen.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b)) as f32;
        debug!("Result of osci max = {max:?}");

        Ok(max)
    }
}

// Implement Job trait for TipController
impl Job for TipController {
    type Output = TipState;
    
    fn run(&mut self, timeout: Duration) -> Result<Self::Output, NanonisError> {
        self.run_loop(timeout)
    }
}
