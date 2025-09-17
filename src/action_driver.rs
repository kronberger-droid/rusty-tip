use log::info;
use ndarray::Array1;

use crate::actions::{Action, ActionChain, ActionResult};
use crate::error::NanonisError;
use crate::nanonis::{NanonisClient, PulseMode, SPMInterface, ZControllerHold};
use crate::types::{
    DataToGet, MotorGroup, OsciData, Position, ScanDirection, SignalIndex,
    SignalRegistry, SignalStats, SignalValue, TriggerConfig,
};
use std::collections::HashMap;
use std::thread;

/// Direct 1:1 translation layer between Actions and SPM interface calls
/// No safety checks, no validation - maximum performance and flexibility
pub struct ActionDriver {
    client: Box<dyn SPMInterface>,
    registry: SignalRegistry,
    /// Storage for Store/Retrieve actions
    stored_values: HashMap<String, ActionResult>,
}

impl ActionDriver {
    /// Create a new ActionDriver with the given client and auto-discover signals
    pub fn new(addr: &str, port: u16) -> Result<Self, NanonisError> {
        let mut client = NanonisClient::new(addr, port)?;

        let names = client.signal_names_get(false)?;
        let registry = SignalRegistry::from_names(names);

        Ok(Self {
            client: Box::new(client),
            registry,
            stored_values: HashMap::new(),
        })
    }

    /// Create ActionDriver with any SPM interface implementation
    pub fn with_spm_interface(
        mut client: Box<dyn SPMInterface>,
    ) -> Result<Self, NanonisError> {
        let names = client.get_signal_names()?;
        let registry = SignalRegistry::from_names(names);
        Ok(Self {
            client,
            registry,
            stored_values: HashMap::new(),
        })
    }

    /// Create a new ActionDriver with a provided registry (for testing)
    pub fn with_registry(
        client: Box<dyn SPMInterface>,
        registry: SignalRegistry,
    ) -> Self {
        Self {
            client,
            registry,
            stored_values: HashMap::new(),
        }
    }

    /// Convenience method to create with NanonisClient
    pub fn with_nanonis_client(
        mut client: NanonisClient,
    ) -> Result<Self, NanonisError> {
        let names = client.signal_names_get(false)?;
        let registry = SignalRegistry::from_names(names);
        Ok(Self {
            client: Box::new(client),
            registry,
            stored_values: HashMap::new(),
        })
    }

    /// Get a reference to the underlying SPM interface
    pub fn spm_interface(&self) -> &dyn SPMInterface {
        self.client.as_ref()
    }

    /// Get a mutable reference to the underlying SPM interface
    pub fn spm_interface_mut(&mut self) -> &mut dyn SPMInterface {
        self.client.as_mut()
    }

    /// Get a reference to the signal registry
    pub fn registry(&self) -> &SignalRegistry {
        &self.registry
    }

    /// Execute a single action with direct 1:1 mapping to client methods
    pub fn execute(&mut self, action: Action) -> Result<ActionResult, NanonisError> {
        match action {
            // === Signal Operations ===
            Action::ReadSignal {
                signal,
                wait_for_newest,
            } => {
                let value = self
                    .client
                    .read_signals(vec![signal.into()], wait_for_newest)?;
                let signal_value = SignalValue::Unitless(value[0] as f64);
                Ok(ActionResult::Signals(vec![signal_value]))
            }

            Action::ReadSignals {
                signals,
                wait_for_newest,
            } => {
                let indices: Vec<i32> =
                    signals.iter().map(|s| (*s).into()).collect();
                let values = self.client.read_signals(indices, wait_for_newest)?;

                // Convert to SignalValue with basic type inference
                let signal_values: Vec<SignalValue> = values
                    .into_iter()
                    .map(|v| SignalValue::Unitless(v as f64))
                    .collect();

                Ok(ActionResult::Signals(signal_values))
            }

            Action::ReadSignalNames => {
                let names = self.client.get_signal_names()?;
                Ok(ActionResult::SignalNames(names))
            }

            // === Bias Operations ===
            Action::ReadBias => {
                let bias = self.client.get_bias()?;
                Ok(ActionResult::BiasVoltage(bias))
            }

            Action::SetBias { voltage } => {
                self.client.set_bias(voltage)?;
                Ok(ActionResult::Success)
            }

            // === Oscilloscope Operations ===
            Action::ReadOsci {
                signal,
                trigger,
                data_to_get,
                is_stable,
            } => {
                self.client.osci1t_run()?;

                self.client.osci1t_ch_set(signal.0)?;

                if let Some(trigger) = trigger {
                    self.client.osci1t_trig_set(
                        trigger.mode,
                        trigger.slope,
                        trigger.level,
                        trigger.hysteresis,
                    )?;
                }

                match data_to_get {
                    crate::types::DataToGet::Stable { readings, timeout } => {
                        match self.find_stable_oscilloscope_data(
                            data_to_get,
                            readings,
                            timeout,
                            0.01,   // relative_threshold (1%)
                            50e-15, // absolute_threshold (50 fA)
                            0.1,    // min_window_percent (10%)
                            is_stable,
                        )? {
                            Some(osci_data) => {
                                Ok(ActionResult::OscilloscopeData(osci_data))
                            }
                            None => Ok(ActionResult::None),
                        }
                    }
                    _ => {
                        // Use NextTrigger for actual data reading - Stable is just for our algorithm
                        let (t0, dt, size, data) =
                            self.client.osci1t_data_get(DataToGet::NextTrigger)?;
                        let osci_data = OsciData::new(t0, dt, size, data);
                        Ok(ActionResult::OscilloscopeData(osci_data))
                    }
                }
            }

            // === Fine Positioning Operations (Piezo) ===
            Action::ReadPiezoPosition {
                wait_for_newest_data,
            } => {
                let pos = self.client.get_xy_position(wait_for_newest_data)?;
                Ok(ActionResult::PiezoPosition(pos))
            }

            Action::SetPiezoPosition {
                position,
                wait_until_finished,
            } => {
                self.client.set_xy_position(position, wait_until_finished)?;
                Ok(ActionResult::Success)
            }

            Action::MovePiezoRelative { delta } => {
                // Get current position and add delta
                let current = self.client.get_xy_position(true)?;
                info!("Current position: {current:?}");
                let new_position = Position {
                    x: current.x + delta.x,
                    y: current.y + delta.y,
                };
                self.client.set_xy_position(new_position, true)?;
                Ok(ActionResult::Success)
            }

            // === Coarse Positioning Operations (Motor) ===
            Action::MoveMotor { direction, steps } => {
                self.client.motor_start_move(
                    direction,
                    steps,
                    MotorGroup::Group1,
                    true, // wait_until_finished
                )?;
                Ok(ActionResult::Success)
            }

            Action::MoveMotorClosedLoop { target, mode } => {
                self.client.motor_start_closed_loop(
                    mode,
                    target,
                    true, // wait_until_finished
                    MotorGroup::Group1,
                )?;
                Ok(ActionResult::Success)
            }

            Action::StopMotor => {
                self.client.motor_stop_move()?;
                Ok(ActionResult::Success)
            }

            // === Control Operations ===
            Action::AutoApproach {
                wait_until_finished,
            } => {
                self.client.auto_approach(wait_until_finished)?;
                Ok(ActionResult::Success)
            }

            Action::Withdraw {
                wait_until_finished,
                timeout,
            } => {
                self.client.z_ctrl_withdraw(wait_until_finished, timeout)?;
                Ok(ActionResult::Success)
            }

            // === Scan Operations ===
            Action::ScanControl { action } => {
                self.client.scan_action(action, ScanDirection::Up)?;
                Ok(ActionResult::Success)
            }

            Action::ReadScanStatus => {
                let is_scanning = self.client.scan_status_get()?;
                Ok(ActionResult::ScanStatus(is_scanning))
            }

            // === Advanced Operations ===
            Action::BiasPulse {
                wait_until_done,
                pulse_width,
                bias_value_v,
                z_controller_hold,
                pulse_mode,
            } => {
                // Convert u16 parameters to enums (safe conversion with fallback)
                let hold_enum = match z_controller_hold {
                    0 => ZControllerHold::NoChange,
                    1 => ZControllerHold::Hold,
                    2 => ZControllerHold::Release,
                    _ => ZControllerHold::NoChange, // Safe fallback
                };

                let mode_enum = match pulse_mode {
                    0 => PulseMode::Keep,
                    1 => PulseMode::Relative,
                    2 => PulseMode::Absolute,
                    _ => PulseMode::Keep, // Safe fallback
                };

                self.client.bias_pulse(
                    wait_until_done,
                    pulse_width,
                    bias_value_v,
                    hold_enum,
                    mode_enum,
                )?;

                Ok(ActionResult::Success)
            }

            Action::Wait { duration } => {
                thread::sleep(duration);
                Ok(ActionResult::None)
            }

            // === Data Management ===
            Action::Store { key, action } => {
                let result = self.execute(*action)?;
                self.stored_values.insert(key, result.clone());
                Ok(ActionResult::StoredValue(Box::new(result)))
            }

            Action::Retrieve { key } => match self.stored_values.get(&key) {
                Some(value) => {
                    Ok(ActionResult::StoredValue(Box::new(value.clone())))
                }
                None => Err(NanonisError::InvalidCommand(format!(
                    "No stored value found for key: {}",
                    key
                ))),
            },

            // === Conditional Operations ===
            Action::Conditional { condition, action } => {
                if self.evaluate_condition(condition)? {
                    self.execute(*action)
                } else {
                    Ok(ActionResult::None) // Condition not met, skip action
                }
            }
        }
    }

    /// Find stable oscilloscope data with proper timeout handling
    ///
    /// This method implements stability detection logic with dual-threshold
    /// approach and timeout handling. It repeatedly reads oscilloscope data until
    /// stable values are found or timeout is reached.
    fn find_stable_oscilloscope_data(
        &mut self,
        _data_to_get: DataToGet,
        readings: u32,
        timeout: std::time::Duration,
        relative_threshold: f64,
        absolute_threshold: f64,
        min_window_percent: f64,
        stability_fn: Option<fn(&[f64]) -> bool>,
    ) -> Result<Option<OsciData>, NanonisError> {
        let start_time = std::time::Instant::now();

        while start_time.elapsed() < timeout {
            for _attempt in 0..readings {
                // Check timeout before each reading attempt
                if start_time.elapsed() >= timeout {
                    return Ok(None);
                }

                let (t0, dt, size, data) =
                    self.client.osci1t_data_get(DataToGet::Current)?;

                let min_window = (size as f64 * min_window_percent) as usize;
                let mut start = 0;
                let mut end = size as usize;

                while (end - start) > min_window {
                    // Check timeout during window analysis
                    if start_time.elapsed() >= timeout {
                        return Ok(None);
                    }

                    let window = &data[start..end];
                    let arr = Array1::from_vec(window.to_vec());
                    let mean = arr.mean().expect("There must be an non-empty array, osci1t_data_get would have returned early.");
                    let std_dev = arr.std(0.0);
                    let relative_std = std_dev / mean.abs();

                    // Use custom stability function if provided, otherwise default dual-threshold
                    let is_stable = if let Some(stability_fn) = stability_fn {
                        stability_fn(window)
                    } else {
                        // Default dual-threshold approach: relative OR absolute
                        let is_relative_stable = relative_std < relative_threshold;
                        let is_absolute_stable = std_dev < absolute_threshold;
                        is_relative_stable || is_absolute_stable
                    };

                    if is_stable {
                        let stable_data = window.to_vec();
                        let stability_method = if stability_fn.is_some() {
                            "custom".to_string()
                        } else {
                            // Default dual-threshold logic
                            let is_relative_stable =
                                relative_std < relative_threshold;
                            let is_absolute_stable = std_dev < absolute_threshold;
                            match (is_relative_stable, is_absolute_stable) {
                                (true, true) => "both".to_string(),
                                (true, false) => "relative".to_string(),
                                (false, true) => "absolute".to_string(),
                                (false, false) => unreachable!(),
                            }
                        };

                        let stats = SignalStats {
                            mean,
                            std_dev,
                            relative_std,
                            window_size: stable_data.len(),
                            stability_method,
                        };

                        let osci_data = OsciData::new_with_stats(
                            t0,
                            dt,
                            stable_data.len() as i32,
                            stable_data,
                            stats,
                        );
                        return Ok(Some(osci_data));
                    }

                    let shrink = ((end - start) / 10).max(1);
                    start += shrink;
                    end -= shrink;
                }

                // Small delay between attempts to avoid overwhelming the system
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            // Brief pause between reading cycles
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        // Timeout reached without finding stable data
        Ok(None)
    }

    /// Execute a chain of actions sequentially
    pub fn execute_chain(
        &mut self,
        chain: impl Into<ActionChain>,
    ) -> Result<Vec<ActionResult>, NanonisError> {
        let chain = chain.into();
        let mut results = Vec::with_capacity(chain.len());

        for action in chain.into_iter() {
            let result = self.execute(action)?;
            results.push(result);
        }

        Ok(results)
    }

    /// Execute chain and return only the final result
    pub fn execute_chain_final(
        &mut self,
        chain: impl Into<ActionChain>,
    ) -> Result<ActionResult, NanonisError> {
        let results = self.execute_chain(chain)?;
        Ok(results.into_iter().last().unwrap_or(ActionResult::None))
    }

    /// Execute chain with early termination on error, returning partial results
    pub fn execute_chain_partial(
        &mut self,
        chain: impl Into<ActionChain>,
    ) -> Result<Vec<ActionResult>, (Vec<ActionResult>, NanonisError)> {
        let chain = chain.into();
        let mut results = Vec::new();

        for action in chain.into_iter() {
            match self.execute(action) {
                Ok(result) => results.push(result),
                Err(error) => return Err((results, error)),
            }
        }

        Ok(results)
    }

    /// Clear all stored values
    pub fn clear_storage(&mut self) {
        self.stored_values.clear();
    }

    /// Get all stored value keys
    pub fn stored_keys(&self) -> Vec<&String> {
        self.stored_values.keys().collect()
    }

    /// Evaluate action condition (basic implementation)
    fn evaluate_condition(
        &mut self,
        condition: crate::types::ActionCondition,
    ) -> Result<bool, NanonisError> {
        use crate::types::ActionCondition;

        match condition {
            ActionCondition::Always => Ok(true),
            ActionCondition::Never => Ok(false),

            ActionCondition::BiasInRange { min, max } => {
                let bias = self.client.get_bias()?;
                Ok(bias >= min && bias <= max)
            }

            ActionCondition::SignalInRange { signal, min, max } => {
                let values = self.client.read_signals(vec![signal.into()], true)?;
                let value = values[0];
                Ok(value >= min && value <= max)
            }

            ActionCondition::PositionInBounds { bounds, tolerance } => {
                let current = self.client.get_xy_position(true)?;
                let dx = (current.x - bounds.x).abs();
                let dy = (current.y - bounds.y).abs();
                Ok(dx <= tolerance && dy <= tolerance)
            }

            ActionCondition::Custom(_func) => {
                // Custom conditions are not supported without full system position reading
                // Return false as safe default
                Ok(false)
            }
        }
    }

    /// Convenience method to read oscilloscope data directly
    pub fn read_oscilloscope(
        &mut self,
        signal: SignalIndex,
        trigger: Option<TriggerConfig>,
        data_to_get: DataToGet,
    ) -> Result<Option<OsciData>, NanonisError> {
        match self.execute(Action::ReadOsci {
            signal,
            trigger,
            data_to_get,
            is_stable: None,
        })? {
            ActionResult::OscilloscopeData(osci_data) => Ok(Some(osci_data)),
            ActionResult::None => Ok(None),
            _ => Err(NanonisError::InvalidCommand(
                "Expected oscilloscope data".into(),
            )),
        }
    }

    /// Convenience method to read oscilloscope data with custom stability function
    pub fn read_oscilloscope_with_stability(
        &mut self,
        signal: SignalIndex,
        trigger: Option<TriggerConfig>,
        data_to_get: DataToGet,
        is_stable: fn(&[f64]) -> bool,
    ) -> Result<Option<OsciData>, NanonisError> {
        match self.execute(Action::ReadOsci {
            signal,
            trigger,
            data_to_get,
            is_stable: Some(is_stable),
        })? {
            ActionResult::OscilloscopeData(osci_data) => Ok(Some(osci_data)),
            ActionResult::None => Ok(None),
            _ => Err(NanonisError::InvalidCommand(
                "Expected oscilloscope data".into(),
            )),
        }
    }
}

/// Simple stability detection functions for oscilloscope windows
pub mod stability {
    /// Dual threshold stability (current default behavior)
    /// Uses relative (1%) OR absolute (50fA) thresholds
    pub fn dual_threshold_stability(window: &[f64]) -> bool {
        if window.len() < 3 {
            return false;
        }

        let mean = window.iter().sum::<f64>() / window.len() as f64;
        let variance = window.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
            / window.len() as f64;
        let std_dev = variance.sqrt();
        let relative_std = std_dev / mean.abs();

        // Stable if EITHER relative OR absolute threshold is met
        relative_std < 0.01 || std_dev < 50e-15
    }

    /// Trend analysis stability detector
    /// Checks for low slope (no trend) and good signal-to-noise ratio
    pub fn trend_analysis_stability(window: &[f64]) -> bool {
        if window.len() < 5 {
            return false;
        }

        // Calculate linear regression slope
        let n = window.len() as f64;
        let x_mean = (n - 1.0) / 2.0; // 0, 1, 2, ... n-1 mean
        let y_mean = window.iter().sum::<f64>() / n;

        let mut numerator = 0.0;
        let mut denominator = 0.0;

        for (i, &y) in window.iter().enumerate() {
            let x = i as f64;
            numerator += (x - x_mean) * (y - y_mean);
            denominator += (x - x_mean).powi(2);
        }

        let slope = if denominator != 0.0 {
            numerator / denominator
        } else {
            0.0
        };

        // Calculate signal-to-noise ratio
        let signal_level = y_mean.abs();
        let noise_level = {
            let variance =
                window.iter().map(|y| (y - y_mean).powi(2)).sum::<f64>() / n;
            variance.sqrt()
        };

        let snr = if noise_level != 0.0 {
            signal_level / noise_level
        } else {
            f64::INFINITY
        };

        // Thresholds: very low slope and decent SNR
        slope.abs() < 0.001 && snr > 10.0
    }
}

/// Statistics about action execution
#[derive(Debug, Clone)]
pub struct ExecutionStats {
    pub total_actions: usize,
    pub successful_actions: usize,
    pub failed_actions: usize,
    pub total_duration: std::time::Duration,
}

impl ExecutionStats {
    pub fn success_rate(&self) -> f64 {
        if self.total_actions == 0 {
            0.0
        } else {
            self.successful_actions as f64 / self.total_actions as f64
        }
    }
}

/// Extension for ActionDriver with execution statistics
impl ActionDriver {
    /// Execute chain with detailed statistics
    pub fn execute_chain_with_stats(
        &mut self,
        chain: impl Into<ActionChain>,
    ) -> Result<(Vec<ActionResult>, ExecutionStats), NanonisError> {
        let chain = chain.into();
        let start_time = std::time::Instant::now();
        let mut results = Vec::with_capacity(chain.len());
        let mut successful = 0;
        let failed = 0;

        for action in chain.into_iter() {
            match self.execute(action) {
                Ok(result) => {
                    results.push(result);
                    successful += 1;
                }
                Err(e) => {
                    // For stats purposes, we want to continue executing but track failures
                    // In a real application, you might want to decide whether to continue or stop
                    // For now, return the error to maintain proper error handling
                    return Err(e);
                }
            }
        }

        let stats = ExecutionStats {
            total_actions: results.len(),
            successful_actions: successful,
            failed_actions: failed,
            total_duration: start_time.elapsed(),
        };

        Ok((results, stats))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    // Note: These tests will fail without actual Nanonis hardware
    // They're included to show the intended interface

    #[test]
    fn test_action_translator_interface() {
        // This test shows how the translator would be used
        // It will fail without actual hardware, but demonstrates the API

        let driver_result = ActionDriver::new("127.0.0.1", 6501);
        match driver_result {
            Ok(mut driver) => {
                // Test single action
                let action = Action::ReadBias;
                let _result = driver.execute(action);

                // With real hardware, this would succeed
                // Without hardware, it will error, which is expected

                // Test chain
                let chain = ActionChain::new(vec![
                    Action::ReadBias,
                    Action::Wait {
                        duration: Duration::from_millis(500),
                    },
                    Action::SetBias { voltage: 1.0 },
                ]);

                let _chain_result = driver.execute_chain(chain);
            }
            Err(_) => {
                // Expected when signals can't be discovered
                println!(
                    "Signal discovery failed - this is expected without hardware"
                );
            }
        }
    }

    #[test]
    fn test_storage_functionality() {
        // Test the storage system using a test registry
        let client = NanonisClient::new("127.0.0.1", 6501);

        match client {
            Ok(client) => {
                // Create test registry
                let registry =
                    SignalRegistry::from_names(vec!["Test Signal".to_string()]);
                let mut driver =
                    ActionDriver::with_registry(Box::new(client), registry);

                // Test storage operations
                driver
                    .stored_values
                    .insert("test_key".to_string(), ActionResult::BiasVoltage(2.5));

                assert_eq!(driver.stored_keys().len(), 1);
                assert!(driver.stored_keys().contains(&&"test_key".to_string()));

                driver.clear_storage();
                assert_eq!(driver.stored_keys().len(), 0);
            }
            Err(_) => {
                // Expected without hardware
            }
        }
    }

    #[test]
    fn test_signal_registry() {
        use crate::SignalIndex;
        // Test registry functionality without requiring hardware
        let registry = SignalRegistry::from_names(vec![
            "Bias (V)".to_string(),
            "Current (A)".to_string(),
            "Z (m)".to_string(),
        ]);

        // Test signal lookup by name
        let bias_signal = registry.get_signal("Bias (V)");
        assert!(bias_signal.is_some());
        assert_eq!(bias_signal.unwrap().0, 0);

        // Test helper methods
        let bias_voltage = registry.bias_voltage();
        assert!(bias_voltage.is_some());
        assert_eq!(bias_voltage.unwrap().0, 0);

        let current = registry.current();
        assert!(current.is_some());
        assert_eq!(current.unwrap().0, 1);

        // Test name retrieval
        let signal_index = SignalIndex(0);
        let name = registry.get_name(signal_index);
        assert_eq!(name, Some("Bias (V)"));
    }
}
