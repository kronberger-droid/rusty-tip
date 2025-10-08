use crate::action_driver::ActionDriver;
use crate::actions::{Action, ActionChain, TipCheckMethod};
use crate::error::NanonisError;
use crate::types::SignalIndex;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

/// Simple tip state - matches original controller
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

pub enum PulseMethod {
    Fixed(f32),
    Stepping {
        voltage_bounds: (f32, f32),
        voltage_steps: u16,
        cycles_before_step: u16,
        threshold: Box<dyn Fn(f32) -> f32 + Send + Sync>,
    },
}

impl PulseMethod {
    pub fn stepping_fixed_threshold(
        voltage_bounds: (f32, f32),
        voltage_steps: u16,
        cycles_before_step: u16,
        threshold: f32,
    ) -> PulseMethod {
        let threshold = threshold.abs();
        PulseMethod::Stepping {
            voltage_bounds,
            voltage_steps,
            cycles_before_step,
            threshold: Box::new(move |_| threshold),
        }
    }
}

pub struct TipControllerConfig {
    freq_shift_index: SignalIndex,
    sharp_tip_bounds: (f32, f32),
    pulse_method: PulseMethod,
    stable_tip_bounds: (f32, f32),
}

/// Enhanced tip controller with pulse voltage stepping
pub struct TipController {
    driver: ActionDriver,
    config: TipControllerConfig,

    // State tracking
    current_pulse_voltage: f32,
    current_tip_state: TipState,
    cycles_without_change: u32,
    good_count: u32,
    cycle_count: u32,

    // Multi-signal history for bias adjustment and analysis
    signal_histories: HashMap<SignalIndex, VecDeque<f32>>,
    max_history_size: usize,
}

impl TipController {
    /// Create new tip controller with basic signal bounds
    pub fn new(driver: ActionDriver, config: TipControllerConfig) -> Self {
        let intial_voltage = match config.pulse_method {
            PulseMethod::Fixed(value) => value,
            PulseMethod::Stepping { voltage_bounds, .. } => voltage_bounds.0,
        };
        Self {
            driver,
            config,
            current_pulse_voltage: intial_voltage,
            current_tip_state: TipState::Bad,
            cycles_without_change: 0,
            good_count: 0,
            cycle_count: 0,
            signal_histories: HashMap::new(),
            max_history_size: 100,
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

    /// Clear all signal histories
    pub fn clear_all_histories(&mut self) {
        self.signal_histories.clear();
    }

    /// Clear history for a specific signal
    pub fn clear_signal_history(&mut self, signal_index: SignalIndex) {
        self.signal_histories.remove(&signal_index);
    }

    // /// Check if current signal represents a significant change from recent stable period
    // fn has_significant_change(&self, signal_index: SignalIndex) -> (bool, f32) {
    //     if let Some(history) = self.signal_histories.get(&signal_index) {
    //         if history.len() < 2 {
    //             // First signal - consider it a significant change to initialize properly
    //             (true, 0.0)
    //         } else {
    //             // Compare only against signals from the current stable period
    //             // cycles_without_change tells us how many recent signals were stable
    //             let stable_period_size =
    //                 (self.cycles_without_change as usize).min(history.len() - 1);

    //             if stable_period_size == 0 {
    //                 // No stable period yet, compare against last signal
    //                 let signal = history[0];
    //                 let last_signal = history[1];
    //                 info!(
    //                     "Last signal: {} | Current threshold: {}",
    //                     last_signal,
    //                     (self.change_threshold)(signal)
    //                 );
    //                 let has_change =
    //                     (signal - last_signal).abs() >= (self.change_threshold)(signal);

    //                 (has_change, (signal - last_signal))
    //             } else {
    //                 // Compare against mean of current stable period (skip current signal at index 0)
    //                 let signal = history[0];
    //                 let stable_signals: Vec<f32> = history
    //                     .iter()
    //                     .skip(1)
    //                     .take(stable_period_size)
    //                     .cloned()
    //                     .collect();
    //                 let stable_mean =
    //                     stable_signals.iter().sum::<f32>() / stable_signals.len() as f32;

    //                 info!(
    //                     "Stable mean: {} | Current threshold: {}",
    //                     stable_mean,
    //                     (self.change_threshold)(signal)
    //                 );
    //                 let has_change =
    //                     (signal - stable_mean).abs() >= (self.change_threshold)(signal);
    //                 (has_change, (signal - stable_mean))
    //             }
    //         }
    //     } else {
    //         // No history yet - consider it a significant change
    //         (true, 0.0)
    //     }
    // }

    // /// Step up the pulse voltage if possible
    // fn step_pulse_voltage(&mut self) -> bool {
    //     let new_pulse = (self.pulse_voltage + self.pulse_voltage_step).min(self.max_pulse_voltage);
    //     if new_pulse > self.pulse_voltage {
    //         info!(
    //             "Stepping pulse voltage: {:.3}V -> {:.3}V",
    //             self.pulse_voltage, new_pulse
    //         );
    //         self.pulse_voltage = new_pulse;
    //         self.cycles_without_change = 0; // Reset counter after stepping
    //         true
    //     } else {
    //         debug!(
    //             "Pulse voltage already at maximum: {:.3}V",
    //             self.max_pulse_voltage
    //         );
    //         self.cycles_without_change = 0; // Reset counter even if at max
    //         false
    //     }
    // }

    // /// Update signal history and step pulse voltage if needed
    // fn update_pulse_voltage(&mut self) {
    //     let (is_significant, change) = self.has_significant_change(self.signal_index);

    //     // 2. Check for significant change and respond accordingly
    //     if is_significant && change >= 0.0 {
    //         self.cycles_without_change = 0;
    //         self.pulse_voltage = self.min_pulse_voltage;
    //     } else if is_significant {
    //         warn!("Positive change significant change!");
    //         self.cycles_without_change += 1;

    //         // Check if we need to step the pulse voltage
    //         if self.cycles_without_change >= self.cycles_before_step {
    //             self.step_pulse_voltage();
    //         }
    //     } else {
    //         self.cycles_without_change += 1;

    //         // Check if we need to step the pulse voltage
    //         if self.cycles_without_change >= self.cycles_before_step {
    //             self.step_pulse_voltage();
    //         }
    //     }
    // }
}

impl TipController {
    /// Main control loop - with pulse voltage stepping
    pub fn run(&mut self) -> Result<(), NanonisError> {
        self.pre_loop_initialization()?;

        while self.current_tip_state != TipState::Stable {
            // Execute one control cycle
            self.cycle_count += 1;

            // Execute based on state
            match self.current_tip_state {
                TipState::Bad => {
                    self.bad_loop()?;
                    continue;
                }
                TipState::Good => {
                    self.good_loop()?;
                    continue;
                }
                TipState::Stable => {
                    info!(
                        "STABLE achieved after {} cycles! Final pulse voltage: {:.3}V",
                        self.cycle_count, self.current_pulse_voltage
                    );
                    break;
                }
            }
        }
        Ok(())
    }

    /// Bad loop - execute recovery sequence with stable signal monitoring
    /// Sequence: capture_stable_before → pulse → capture_stable_after → withdraw → move → approach → check
    fn bad_loop(&mut self) -> Result<(), NanonisError> {
        // Reset good count
        self.good_count = 0;

        self.driver.execute(Action::BiasPulse {
            wait_until_done: true,
            pulse_width: Duration::from_millis(50),
            bias_value_v: self.current_pulse_voltage,
            z_controller_hold: 1,
            pulse_mode: 2,
        })?;

        self.driver.execute_chain(ActionChain::new(vec![
            Action::Withdraw {
                wait_until_finished: true,
                timeout: Duration::from_secs(5),
            },
            Action::MoveMotor3D {
                displacement: crate::types::MotorDisplacement::new(2, 2, -3),
                blocking: true,
            },
            Action::AutoApproach {
                wait_until_finished: true,
                timeout: Duration::from_secs(5),
            },
            Action::Wait {
                duration: Duration::from_secs(1),
            },
        ]))?;

        let ampl_setpoint = self.driver.client_mut().pll_amp_ctrl_setpnt_get(1)?;
        let ampl_current = self.driver.client_mut().signal_val_get(75, true)?;

        if !(ampl_setpoint - 5e-12..ampl_setpoint + 5e-12).contains(&ampl_current) {
            self.current_tip_state = TipState::Bad;
        };
        let check_method = TipCheckMethod::SignalBounds {
            signal: self.config.freq_shift_index,
            bounds: self.config.sharp_tip_bounds,
        };

        self.current_tip_state = self
            .driver
            .run(Action::CheckTipState {
                method: check_method,
            })
            .expecting()?;

        Ok(())
    }

    /// Good loop - monitoring, increment good count
    fn good_loop(&mut self) -> Result<(), NanonisError> {
        self.good_count += 1;

        Ok(())
    }

    fn pre_loop_initialization(&mut self) -> Result<(), NanonisError> {
        self.driver.client_mut().set_bias(-500e-3)?;

        self.driver.client_mut().z_ctrl_setpoint_set(100e-12)?;

        self.driver.execute(Action::AutoApproach {
            wait_until_finished: true,
            timeout: Duration::from_secs(1),
        })?;

        Ok(())
    }
}
