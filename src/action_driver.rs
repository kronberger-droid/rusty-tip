use log::info;
use ndarray::Array1;

use crate::actions::{Action, ActionChain, ActionResult, ExpectFromAction};
use crate::error::NanonisError;
use crate::nanonis::NanonisClient;
use crate::types::{
    DataToGet, MotorGroup, OsciData, Position, PulseMode, ScanDirection, SignalIndex, SignalStats,
    TriggerConfig, ZControllerHold,
};
use crate::utils::{poll_until, poll_with_timeout, PollError};
use crate::TipShaperConfig;
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

/// Configuration for future TCP Logger integration
#[derive(Debug, Clone)]
pub struct TCPLoggerConfig {
    pub stream_port: u16,
    pub channels: Vec<i32>,
    pub oversampling: i32,
    pub auto_start: bool,
    pub buffer_size: usize,
}

/// Builder for configuring ActionDriver with optional parameters
#[derive(Debug, Clone)]
pub struct ActionDriverBuilder {
    addr: String,
    port: u16,
    connection_timeout: Option<Duration>,
    initial_storage: HashMap<String, ActionResult>,
    tcp_logger_config: Option<TCPLoggerConfig>,
}

impl ActionDriverBuilder {
    /// Create a new builder with required connection parameters
    pub fn new(addr: &str, port: u16) -> Self {
        Self {
            addr: addr.to_string(),
            port,
            connection_timeout: None,
            initial_storage: HashMap::new(),
            tcp_logger_config: None,
        }
    }

    /// Set connection timeout for the underlying NanonisClient
    pub fn with_connection_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = Some(timeout);
        self
    }

    /// Initialize with pre-stored values
    pub fn with_initial_storage(mut self, storage: HashMap<String, ActionResult>) -> Self {
        self.initial_storage = storage;
        self
    }

    /// Add a single pre-stored value
    pub fn with_stored_value(mut self, key: String, value: ActionResult) -> Self {
        self.initial_storage.insert(key, value);
        self
    }

    /// Configure TCP Logger for future channel-based integration
    /// This prepares the architecture for streaming data directly into ActionDriver
    pub fn with_tcp_logger(mut self, config: TCPLoggerConfig) -> Self {
        self.tcp_logger_config = Some(config);
        self
    }

    /// Build the ActionDriver with configured parameters
    pub fn build(self) -> Result<ActionDriver, NanonisError> {
        let client = if let Some(timeout) = self.connection_timeout {
            NanonisClient::builder()
                .address(&self.addr)
                .port(self.port)
                .connect_timeout(timeout)
                .build()?
        } else {
            NanonisClient::new(&self.addr, self.port)?
        };

        Ok(ActionDriver {
            client,
            stored_values: self.initial_storage,
            tcp_logger_config: self.tcp_logger_config,
        })
    }
}

/// Direct 1:1 translation layer between Actions and NanonisClient calls
/// No safety checks, no validation - maximum performance and flexibility
pub struct ActionDriver {
    client: NanonisClient,
    /// Storage for Store/Retrieve actions
    stored_values: HashMap<String, ActionResult>,
    /// Future: TCP Logger configuration for channel integration
    tcp_logger_config: Option<TCPLoggerConfig>,
}

impl ActionDriver {
    /// Create a builder for configuring ActionDriver
    pub fn builder(addr: &str, port: u16) -> ActionDriverBuilder {
        ActionDriverBuilder::new(addr, port)
    }

    /// Create a new ActionDriver with default configuration (backward compatibility)
    pub fn new(addr: &str, port: u16) -> Result<Self, NanonisError> {
        Self::builder(addr, port).build()
    }

    /// Convenience method to create with existing NanonisClient (backward compatibility)
    pub fn with_nanonis_client(client: NanonisClient) -> Self {
        Self {
            client,
            stored_values: HashMap::new(),
            tcp_logger_config: None,
        }
    }

    /// Get a reference to the underlying NanonisClient
    pub fn client(&self) -> &NanonisClient {
        &self.client
    }

    /// Get a mutable reference to the underlying NanonisClient
    pub fn client_mut(&mut self) -> &mut NanonisClient {
        &mut self.client
    }

    /// Get TCP Logger configuration if set
    pub fn tcp_logger_config(&self) -> Option<&TCPLoggerConfig> {
        self.tcp_logger_config.as_ref()
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
                    .signals_vals_get(vec![signal.into()], wait_for_newest)?;
                Ok(ActionResult::Value(value[0] as f64))
            }

            Action::ReadSignals {
                signals,
                wait_for_newest,
            } => {
                let indices: Vec<i32> = signals.iter().map(|s| (*s).into()).collect();
                let values = self.client.signals_vals_get(indices, wait_for_newest)?;
                Ok(ActionResult::Values(
                    values.into_iter().map(|v| v as f64).collect(),
                ))
            }

            Action::ReadSignalNames => {
                let names = self.client.signal_names_get(false)?;
                Ok(ActionResult::Text(names))
            }

            // === Bias Operations ===
            Action::ReadBias => {
                let bias = self.client.get_bias()?;
                Ok(ActionResult::Value(bias as f64))
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
                        trigger.mode.into(),
                        trigger.slope.into(),
                        trigger.level,
                        trigger.hysteresis,
                    )?;
                }

                match data_to_get {
                    crate::types::DataToGet::Stable { readings, timeout } => {
                        let osci_data = self.find_stable_oscilloscope_data_with_fallback(
                            data_to_get,
                            readings,
                            timeout,
                            0.01,
                            50e-15,
                            0.8,
                            is_stable,
                        )?;
                        Ok(ActionResult::OsciData(osci_data))
                    }
                    _ => {
                        // Use NextTrigger for actual data reading - Stable is just for our algorithm
                        let data_mode = match data_to_get {
                            DataToGet::Current => 0,
                            DataToGet::NextTrigger => 1,
                            DataToGet::Wait2Triggers => 2,
                            DataToGet::Stable { .. } => 1, // Use NextTrigger for stable
                        };
                        let (t0, dt, size, data) = self.client.osci1t_data_get(data_mode)?;
                        let osci_data = OsciData::new_stable(t0, dt, size, data);
                        Ok(ActionResult::OsciData(osci_data))
                    }
                }
            }

            // === Fine Positioning Operations (Piezo) ===
            Action::ReadPiezoPosition {
                wait_for_newest_data,
            } => {
                let pos = self.client.folme_xy_pos_get(wait_for_newest_data)?;
                Ok(ActionResult::Position(pos))
            }

            Action::SetPiezoPosition {
                position,
                wait_until_finished,
            } => {
                self.client
                    .folme_xy_pos_set(position, wait_until_finished)?;
                Ok(ActionResult::Success)
            }

            Action::MovePiezoRelative { delta } => {
                // Get current position and add delta
                let current = self.client.folme_xy_pos_get(true)?;
                info!("Current position: {current:?}");
                let new_position = Position {
                    x: current.x + delta.x,
                    y: current.y + delta.y,
                };
                self.client.folme_xy_pos_set(new_position, true)?;
                Ok(ActionResult::Success)
            }

            // === Coarse Positioning Operations (Motor) ===
            Action::MoveMotorAxis {
                direction,
                steps,
                blocking,
            } => {
                self.client
                    .motor_start_move(direction, steps, MotorGroup::Group1, blocking)?;
                Ok(ActionResult::Success)
            }

            Action::MoveMotor3D {
                displacement,
                blocking,
            } => {
                // Convert 3D displacement to sequence of motor movements
                let movements = displacement.to_motor_movements();

                // Execute each movement in sequence
                for (direction, steps) in movements {
                    self.client
                        .motor_start_move(direction, steps, MotorGroup::Group1, blocking)?;
                }
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
                timeout,
            } => {
                log::debug!(
                    "Starting auto-approach (wait: {}, timeout: {:?})",
                    wait_until_finished,
                    timeout
                );

                // Check if already running
                match self.client.auto_approach_on_off_get() {
                    Ok(true) => {
                        log::warn!("Auto-approach already running");
                        return Ok(ActionResult::Success); // Consider already running as success
                    }
                    Ok(false) => {
                        log::debug!("Auto-approach is idle, proceeding to start");
                    }
                    Err(_) => {
                        log::warn!("Auto-approach status unknown, attempting to proceed");
                    }
                }

                // Open auto-approach module
                if let Err(e) = self.client.auto_approach_open() {
                    log::error!("Failed to open auto-approach module: {}", e);
                    return Err(NanonisError::InvalidCommand(format!(
                        "Failed to open auto-approach module: {}",
                        e
                    )));
                }

                // Wait for module initialization
                std::thread::sleep(std::time::Duration::from_millis(500));

                // Start auto-approach
                if let Err(e) = self.client.auto_approach_on_off_set(true) {
                    log::error!("Failed to start auto-approach: {}", e);
                    return Err(NanonisError::InvalidCommand(format!(
                        "Failed to start auto-approach: {}",
                        e
                    )));
                }

                if !wait_until_finished {
                    log::debug!("Auto-approach started, not waiting for completion");
                    return Ok(ActionResult::Success);
                }

                // Wait for completion with timeout
                log::debug!("Waiting for auto-approach to complete...");
                let poll_interval = std::time::Duration::from_millis(100);

                match poll_until(
                    || {
                        // Returns Ok(true) when auto-approach is complete (not running)
                        self.client
                            .auto_approach_on_off_get()
                            .map(|running| !running)
                    },
                    timeout,
                    poll_interval,
                ) {
                    Ok(()) => {
                        log::debug!("Auto-approach completed successfully");
                        Ok(ActionResult::Success)
                    }
                    Err(PollError::Timeout) => {
                        log::warn!("Auto-approach timed out after {:?}", timeout);
                        // Try to stop the auto-approach
                        let _ = self.client.auto_approach_on_off_set(false);
                        Err(NanonisError::InvalidCommand(
                            "Auto-approach timed out".to_string(),
                        ))
                    }
                    Err(PollError::ConditionError(e)) => {
                        log::error!("Error checking auto-approach status: {}", e);
                        Err(NanonisError::InvalidCommand(format!(
                            "Status check error: {}",
                            e
                        )))
                    }
                }
            }

            Action::Withdraw {
                wait_until_finished,
                timeout,
            } => {
                self.client.z_ctrl_withdraw(wait_until_finished, timeout)?;
                Ok(ActionResult::Success)
            }

            Action::SetZSetpoint { setpoint } => {
                self.client.z_ctrl_setpoint_set(setpoint)?;
                Ok(ActionResult::Success)
            }

            // === Scan Operations ===
            Action::ScanControl { action } => {
                self.client.scan_action(action, ScanDirection::Up)?;
                Ok(ActionResult::Success)
            }

            Action::ReadScanStatus => {
                let is_scanning = self.client.scan_status_get()?;
                Ok(ActionResult::Status(is_scanning))
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
                    pulse_width.as_secs_f32(),
                    bias_value_v,
                    hold_enum.into(),
                    mode_enum.into(),
                )?;

                Ok(ActionResult::Success)
            }

            Action::TipShaper {
                config,
                wait_until_finished,
                timeout,
            } => {
                // Set tip shaper configuration
                self.client.tip_shaper_props_set(config)?;

                // Start tip shaper
                self.client.tip_shaper_start(wait_until_finished, timeout)?;

                Ok(ActionResult::Success)
            }

            Action::PulseRetract {
                pulse_width,
                pulse_height_v,
            } => {
                let current_bias = self.client_mut().get_bias().unwrap_or(500e-3);
                let config = TipShaperConfig {
                    switch_off_delay: std::time::Duration::from_millis(10),
                    change_bias: true,
                    bias_v: pulse_height_v,
                    tip_lift_m: 0.0,
                    lift_time_1: pulse_width,
                    bias_lift_v: current_bias,
                    bias_settling_time: std::time::Duration::from_millis(50),
                    lift_height_m: 10e-9,
                    lift_time_2: std::time::Duration::from_millis(100),
                    end_wait_time: std::time::Duration::from_millis(50),
                    restore_feedback: false,
                };

                // Set tip shaper configuration and start
                self.client_mut().tip_shaper_props_set(config)?;
                self.client_mut()
                    .tip_shaper_start(true, Duration::from_secs(5))?;

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
                Ok(result) // Return the original result directly
            }

            Action::Retrieve { key } => match self.stored_values.get(&key) {
                Some(value) => Ok(value.clone()), // Return the stored result directly
                None => Err(NanonisError::InvalidCommand(format!(
                    "No stored value found for key: {}",
                    key
                ))),
            },
        }
    }

    /// Execute action and extract specific type with validation
    ///
    /// This is a convenience method that combines execute() with type extraction,
    /// providing better ergonomics while preserving type safety.
    ///
    /// # Example
    /// ```no_run
    /// use rusty_tip::{ActionDriver, Action, SignalIndex};
    /// use rusty_tip::types::{DataToGet, OsciData};
    ///
    /// let mut driver = ActionDriver::new("127.0.0.1", 6501)?;
    /// let osci_data: OsciData = driver.execute_expecting(Action::ReadOsci {
    ///     signal: SignalIndex(24),
    ///     trigger: None,
    ///     data_to_get: DataToGet::Current,
    ///     is_stable: None,
    /// })?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn execute_expecting<T>(&mut self, action: Action) -> Result<T, NanonisError>
    where
        ActionResult: ExpectFromAction<T>,
    {
        let result = self.execute(action.clone())?;
        Ok(result.expect_from_action(&action))
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
        match poll_with_timeout(
            || {
                // Try to find stable data in a batch of readings
                for _attempt in 0..readings {
                    let (t0, dt, size, data) = self.client.osci1t_data_get(2)?; // Wait2Triggers = 2

                    if let Some(stable_osci_data) = self.analyze_stability_window(
                        t0,
                        dt,
                        size,
                        data,
                        relative_threshold,
                        absolute_threshold,
                        min_window_percent,
                        stability_fn,
                    )? {
                        return Ok(Some(stable_osci_data));
                    }

                    // Small delay between attempts to avoid overwhelming the system
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }

                // No stable data found in this batch, continue polling
                Ok(None)
            },
            timeout,
            std::time::Duration::from_millis(50), // Brief pause between reading cycles
        ) {
            Ok(Some(result)) => Ok(Some(result)),
            Ok(None) => Ok(None), // Timeout reached
            Err(PollError::ConditionError(e)) => Err(e),
            Err(PollError::Timeout) => unreachable!(), // poll_with_timeout returns Ok(None) on timeout
        }
    }

    /// Analyze a single oscilloscope data window for stability
    fn analyze_stability_window(
        &self,
        t0: f64,
        dt: f64,
        size: i32,
        data: Vec<f64>,
        relative_threshold: f64,
        absolute_threshold: f64,
        min_window_percent: f64,
        stability_fn: Option<fn(&[f64]) -> bool>,
    ) -> Result<Option<OsciData>, NanonisError> {
        let min_window = (size as f64 * min_window_percent) as usize;
        let mut start = 0;
        let mut end = size as usize;

        while (end - start) > min_window {
            let window = &data[start..end];
            let arr = Array1::from_vec(window.to_vec());
            let mean = arr.mean().expect(
                "There must be an non-empty array, osci1t_data_get would have returned early.",
            );
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
                    let is_relative_stable = relative_std < relative_threshold;
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

                let mut osci_data =
                    OsciData::new_with_stats(t0, dt, stable_data.len() as i32, stable_data, stats);
                osci_data.is_stable = true; // Mark as stable since we found stable data
                return Ok(Some(osci_data));
            }

            let shrink = ((end - start) / 10).max(1);
            start += shrink;
            end -= shrink;
        }

        // No stable window found in this data
        Ok(None)
    }

    /// Find stable oscilloscope data with fallback to single value
    ///
    /// This method attempts to find stable oscilloscope data. If successful,
    /// it returns OsciData with is_stable=true. If no stable data is found
    /// within the timeout, it returns OsciData with is_stable=false and
    /// a fallback single value reading.
    fn find_stable_oscilloscope_data_with_fallback(
        &mut self,
        data_to_get: DataToGet,
        readings: u32,
        timeout: std::time::Duration,
        relative_threshold: f64,
        absolute_threshold: f64,
        min_window_percent: f64,
        stability_fn: Option<fn(&[f64]) -> bool>,
    ) -> Result<OsciData, NanonisError> {
        // First try to find stable data
        if let Some(stable_osci_data) = self.find_stable_oscilloscope_data(
            data_to_get,
            readings,
            timeout,
            relative_threshold,
            absolute_threshold,
            min_window_percent,
            stability_fn,
        )? {
            return Ok(stable_osci_data);
        }

        // If no stable data found, get a single reading as fallback
        let (t0, dt, size, data) = self.client.osci1t_data_get(1)?; // NextTrigger = 1

        // Calculate fallback value (mean of the data)
        let fallback_value = if !data.is_empty() {
            data.iter().sum::<f64>() / data.len() as f64
        } else {
            0.0
        };

        Ok(OsciData::new_unstable_with_fallback(
            t0,
            dt,
            size,
            data,
            fallback_value,
        ))
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
            ActionResult::OsciData(osci_data) => Ok(Some(osci_data)),
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
            ActionResult::OsciData(osci_data) => Ok(Some(osci_data)),
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
        let variance = window.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / window.len() as f64;
        let std_dev = variance.sqrt();
        let relative_std = std_dev / mean.abs();

        // Stable if EITHER relative OR absolute threshold is met
        relative_std < 0.05 || std_dev < 50e-15
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
            let variance = window.iter().map(|y| (y - y_mean).powi(2)).sum::<f64>() / n;
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

impl Drop for ActionDriver {
    fn drop(&mut self) {
        let _ = self.execute_chain(vec![
            Action::Withdraw {
                wait_until_finished: false,
                timeout: Duration::from_secs(1),
            },
            Action::MoveMotorAxis {
                direction: crate::MotorDirection::ZPlus,
                steps: 2,
                blocking: false,
            },
        ]);
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
                println!("Signal discovery failed - this is expected without hardware");
            }
        }
    }
}
