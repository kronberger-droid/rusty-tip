use crate::action_driver::ActionDriver;
use crate::actions::{Action, TipCheckMethod, TipState};
use crate::error::NanonisError;
use crate::types::{SignalIndex, TipShape};
use log::info;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

/// Loop types based on tip shape - simple and direct
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
    pub freq_shift_index: SignalIndex,
    pub sharp_tip_bounds: (f32, f32),
    pub pulse_method: PulseMethod,
    pub allowed_change_for_stable: f32,
}

impl Default for TipControllerConfig {
    fn default() -> Self {
        Self {
            freq_shift_index: 76u8.into(),
            sharp_tip_bounds: (-2.0, 0.0),
            pulse_method: PulseMethod::Fixed(4.0),
            allowed_change_for_stable: 0.2,
        }
    }
}

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
    signal_histories: HashMap<SignalIndex, VecDeque<f32>>,
    max_history_size: usize,
}

impl TipController {
    /// Create new tip controller with basic signal bounds
    pub fn new(driver: ActionDriver, config: TipControllerConfig) -> Self {
        let initial_voltage = match config.pulse_method {
            PulseMethod::Fixed(value) => value,
            PulseMethod::Stepping { voltage_bounds, .. } => voltage_bounds.0,
        };
        Self {
            driver,
            config,
            current_pulse_voltage: initial_voltage,
            current_tip_shape: TipShape::Blunt,
            cycles_without_change: 0,
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

    /// Check if current signal represents a significant change from recent stable period
    fn has_significant_change(&self, signal_index: SignalIndex) -> (bool, f32) {
        // Only check for stepping if PulseMethod is Stepping
        let threshold_fn = match &self.config.pulse_method {
            PulseMethod::Stepping { threshold, .. } => threshold,
            PulseMethod::Fixed(_) => return (false, 0.0), // No stepping for fixed method
        };

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
                    let current_signal = history[0];
                    let last_signal = history[1];
                    let threshold = threshold_fn(current_signal);

                    log::info!(
                        "Last signal: {:.3e} | Current threshold: {:.3e}",
                        last_signal,
                        threshold
                    );

                    let change = current_signal - last_signal;
                    let has_change = change.abs() >= threshold;

                    (has_change, change)
                } else {
                    // Compare against mean of current stable period (skip current signal at index 0)
                    let current_signal = history[0];
                    let stable_signals: Vec<f32> = history
                        .iter()
                        .skip(1)
                        .take(stable_period_size)
                        .cloned()
                        .collect();
                    let stable_mean =
                        stable_signals.iter().sum::<f32>() / stable_signals.len() as f32;

                    let threshold = threshold_fn(current_signal);
                    log::info!(
                        "Current: {:.3e} | Stable mean: {:.3e} | Threshold: {:.3e}",
                        current_signal,
                        stable_mean,
                        threshold
                    );

                    let change = current_signal - stable_mean;
                    let has_change = change.abs() >= threshold;
                    (has_change, change)
                }
            }
        } else {
            // No history yet - consider it a significant change
            (true, 0.0)
        }
    }

    /// Step up the pulse voltage if possible
    fn step_pulse_voltage(&mut self) -> bool {
        // Only step if using stepping method
        let (voltage_bounds, voltage_steps) = match &self.config.pulse_method {
            PulseMethod::Stepping {
                voltage_bounds,
                voltage_steps,
                ..
            } => (*voltage_bounds, *voltage_steps),
            PulseMethod::Fixed(_) => return false, // No stepping for fixed method
        };

        // Calculate step size
        let step_size = (voltage_bounds.1 - voltage_bounds.0) / voltage_steps as f32;
        let new_pulse = (self.current_pulse_voltage + step_size).min(voltage_bounds.1);

        if new_pulse > self.current_pulse_voltage {
            info!(
                "Stepping pulse voltage: {:.3}V -> {:.3}V",
                self.current_pulse_voltage, new_pulse
            );
            self.current_pulse_voltage = new_pulse;
            self.cycles_without_change = 0; // Reset counter after stepping
            true
        } else {
            log::debug!("Pulse voltage already at maximum: {:.3}V", voltage_bounds.1);
            self.cycles_without_change = 0; // Reset counter even if at max
            false
        }
    }

    /// Update signal history and step pulse voltage if needed
    fn update_pulse_voltage(&mut self) {
        let (is_significant, change) = self.has_significant_change(self.config.freq_shift_index);

        // Get cycles_before_step from config, or use 0 for Fixed method
        let cycles_before_step = match &self.config.pulse_method {
            PulseMethod::Stepping {
                cycles_before_step,
                voltage_bounds,
                ..
            } => (*cycles_before_step, *voltage_bounds),
            PulseMethod::Fixed(_) => return, // No stepping for fixed method
        };

        // Check for significant change and respond accordingly
        if is_significant && change >= 0.0 {
            // Positive significant change - reset to minimum voltage
            self.cycles_without_change = 0;
            self.current_pulse_voltage = cycles_before_step.1 .0; // voltage_bounds.0 (min)
            log::info!(
                "Positive significant change detected, resetting pulse voltage to minimum: {:.3}V",
                self.current_pulse_voltage
            );
        } else if is_significant {
            log::warn!("Negative significant change detected!");
            self.cycles_without_change += 1;

            // Check if we need to step the pulse voltage
            if self.cycles_without_change >= cycles_before_step.0 as u32 {
                self.step_pulse_voltage();
            }
        } else {
            // No significant change
            self.cycles_without_change += 1;

            // Check if we need to step the pulse voltage
            if self.cycles_without_change >= cycles_before_step.0 as u32 {
                self.step_pulse_voltage();
            }
        }
    }
}

impl TipController {
    /// Main control loop - with pulse voltage stepping
    pub fn run(&mut self) -> Result<(), NanonisError> {
        self.pre_loop_initialization()?;

        while self.current_tip_shape != TipShape::Stable {
            // Execute one control cycle
            self.cycle_count += 1;

            // Execute based on state
            match self.current_tip_shape {
                TipShape::Blunt => {
                    self.bad_loop()?;
                    continue;
                }
                TipShape::Sharp => {
                    self.good_loop()?;
                    continue;
                }
                TipShape::Stable => {
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
        info!(
            "Executing PulseRetract Sequence with pulse = {} V",
            self.current_pulse_voltage
        );

        self.driver
            .run(Action::PulseRetract {
                pulse_width: Duration::from_millis(500),
                pulse_height_v: self.current_pulse_voltage,
            })
            .with_data_collection(Duration::from_millis(50), Duration::from_millis(50))
            .execute()?;

        info!("Repositioning...");

        self.driver
            .run(Action::SafeReposition {
                x_steps: 2,
                y_steps: 2,
            })
            .go()?;

        // let amplitude_reached: bool = self
        //     .driver
        //     .run(Action::ReachedTargedAmplitude)
        //     .expecting()?;
        let amplitude_reached = true;

        if amplitude_reached {
            let tip_state: TipState = self
                .driver
                .run(Action::CheckTipState {
                    method: TipCheckMethod::SignalBounds {
                        signal: self.config.freq_shift_index,
                        bounds: self.config.sharp_tip_bounds,
                    },
                })
                .expecting()?;

            self.current_tip_shape = tip_state.shape;

            self.track_signal(
                self.config.freq_shift_index,
                tip_state
                    .measured_signals
                    .get(&self.config.freq_shift_index)
                    .copied()
                    .expect("Calling Check should definitly return a measured value"),
            );

            // Update pulse voltage based on signal changes (stepping logic)
            self.update_pulse_voltage();
        } else {
            info!("Amplitude not reached. Assuming blunt tip");
            self.current_tip_shape = TipShape::Blunt;
        }

        Ok(())
    }

    /// Good loop - monitoring, increment good count
    fn good_loop(&mut self) -> Result<(), NanonisError> {
        let stability_result: crate::actions::StabilityResult = self
            .driver
            .run(Action::CheckTipStability {
                method: crate::actions::TipStabilityMethod::BiasSweepResponse {
                    signal: self.config.freq_shift_index,
                    bias_range: (-2.0, 2.0),
                    sweep_steps: 1000,
                    period: Duration::from_millis(200),
                    allowed_signal_change: self.config.allowed_change_for_stable,
                },
                max_duration: Duration::from_secs(100),
                abort_on_damage_signs: false,
            })
            .expecting()?;

        // Track signal values from stability monitoring (use last measured value)
        if let Some(signal_values) = stability_result
            .measured_values
            .get(&self.config.freq_shift_index)
        {
            if let Some(&last_value) = signal_values.last() {
                self.track_signal(self.config.freq_shift_index, last_value);
            }
        }

        // Update tip shape based on stability result
        self.current_tip_shape = if stability_result.is_stable {
            TipShape::Stable
        } else {
            self.driver
                .run(Action::SafeReposition {
                    x_steps: 2,
                    y_steps: 2,
                })
                .go()?;

            self.driver
                .run(Action::CheckTipState {
                    method: TipCheckMethod::SignalBounds {
                        signal: self.config.freq_shift_index,
                        bounds: self.config.sharp_tip_bounds,
                    },
                })
                .expecting()?
        };

        Ok(())
    }

    fn pre_loop_initialization(&mut self) -> Result<(), NanonisError> {
        info!("Running pre loop initialization");

        self.driver.client_mut().set_bias(-500e-3)?;
        self.driver.client_mut().z_ctrl_setpoint_set(100e-12)?;

        info!("Executing Initial Approach");

        self.driver
            .run(Action::AutoApproach {
                wait_until_finished: true,
                timeout: Duration::from_secs(10),
            })
            .go()?;

        let initial_tip_state: TipState = self
            .driver
            .run(Action::CheckTipState {
                method: TipCheckMethod::SignalBounds {
                    signal: self.config.freq_shift_index,
                    bounds: self.config.sharp_tip_bounds,
                },
            })
            .expecting()?;

        info!("Current tip shape: {initial_tip_state:?}");

        self.current_tip_shape = initial_tip_state.shape;

        Ok(())
    }
}
