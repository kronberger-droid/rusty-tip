// chrono import removed - now using Utc::now() directly in actions.rs
use log::info;
use ndarray::Array1;

use crate::actions::{Action, ActionChain, ActionLogEntry, ActionLogResult, ActionResult, ExpectFromAction};
use crate::error::NanonisError;
use crate::nanonis::NanonisClient;
use crate::types::{
    DataToGet, MotorGroup, OsciData, Position, PulseMode, ScanDirection, SignalIndex, SignalStats,
    TCPLoggerData, TriggerConfig, ZControllerHold,
};
use crate::utils::{poll_until, poll_with_timeout, PollError};
use crate::TipShaperConfig;
use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Configuration for TCP Logger integration with always-buffer support
#[derive(Debug, Clone)]
pub struct TCPLoggerConfig {
    /// TCP data stream port (typically 6590)
    pub stream_port: u16,
    /// Signal channel indices to record (0-127)
    pub channels: Vec<i32>,
    /// Oversampling rate multiplier (0-1000)
    pub oversampling: i32,
    /// Whether to start logging automatically on connection
    pub auto_start: bool,
    /// Buffer size for always-buffer mode (None = no buffering)
    /// When Some(size), BufferedTCPReader starts automatically
    pub buffer_size: Option<usize>,
}

/// Unified input type for run() method - accepts single actions or chains
#[derive(Debug, Clone)]
pub enum ActionRequest {
    /// Single action
    Single(Action),
    /// Multiple actions as chain
    Chain(Vec<Action>),
}

impl From<Action> for ActionRequest {
    fn from(action: Action) -> Self {
        ActionRequest::Single(action)
    }
}

impl From<Vec<Action>> for ActionRequest {
    fn from(actions: Vec<Action>) -> Self {
        ActionRequest::Chain(actions)
    }
}

impl From<ActionChain> for ActionRequest {
    fn from(chain: ActionChain) -> Self {
        ActionRequest::Chain(chain.into_iter().collect())
    }
}

impl ActionRequest {
    pub fn is_single(&self) -> bool {
        matches!(self, ActionRequest::Single(_))
    }

    pub fn is_chain(&self) -> bool {
        matches!(self, ActionRequest::Chain(_))
    }
}

/// Configuration for execution behavior in the unified run() method
#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    /// Enable data collection with pre/post durations
    pub data_collection: Option<(Duration, Duration)>,
    /// Chain execution behavior
    pub chain_behavior: ChainBehavior,
    /// Logging behavior
    pub logging_behavior: LoggingBehavior,
    /// Performance optimizations
    pub performance_mode: PerformanceMode,
}

#[derive(Debug, Clone)]
pub enum ChainBehavior {
    /// Execute all actions, return all results (default)
    Complete,
    /// Execute all actions, return only final result
    FinalOnly,
    /// Execute until error, return partial results
    Partial,
}

#[derive(Debug, Clone)]
pub enum LoggingBehavior {
    /// Normal logging (default)
    Normal,
    /// No per-action logging, single chain log
    Deferred,
    /// Disable logging completely for this execution
    Disabled,
}

#[derive(Debug, Clone)]
pub enum PerformanceMode {
    /// Normal execution (default)
    Normal,
    /// Optimized for timing-critical operations
    Fast,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            data_collection: None,
            chain_behavior: ChainBehavior::Complete,
            logging_behavior: LoggingBehavior::Normal,
            performance_mode: PerformanceMode::Normal,
        }
    }
}

impl ExecutionConfig {
    /// Create new config with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable data collection with specified pre/post durations
    pub fn with_data_collection(mut self, pre_duration: Duration, post_duration: Duration) -> Self {
        self.data_collection = Some((pre_duration, post_duration));
        self
    }

    /// Set chain to return only final result
    pub fn final_only(mut self) -> Self {
        self.chain_behavior = ChainBehavior::FinalOnly;
        self
    }

    /// Set chain to allow partial execution on error
    pub fn partial(mut self) -> Self {
        self.chain_behavior = ChainBehavior::Partial;
        self
    }

    /// Use deferred logging (single chain entry instead of per-action)
    pub fn deferred_logging(mut self) -> Self {
        self.logging_behavior = LoggingBehavior::Deferred;
        self
    }

    /// Disable logging for this execution
    pub fn no_logging(mut self) -> Self {
        self.logging_behavior = LoggingBehavior::Disabled;
        self
    }

    /// Enable fast performance mode
    pub fn fast_mode(mut self) -> Self {
        self.performance_mode = PerformanceMode::Fast;
        self
    }
}

/// Result container for unified run() method
#[derive(Debug)]
pub enum ExecutionResult {
    /// Single action result
    Single(ActionResult),
    /// Multiple action results
    Chain(Vec<ActionResult>),
    /// Experiment data with signal collection
    ExperimentData(crate::types::ExperimentData),
    /// Chain experiment data with signal collection
    ChainExperimentData(crate::types::ChainExperimentData),
    /// Partial chain results (on error)
    Partial(Vec<ActionResult>, NanonisError),
}

impl ExecutionResult {
    /// Extract single result or error if not single
    pub fn into_single(self) -> Result<ActionResult, NanonisError> {
        match self {
            ExecutionResult::Single(result) => Ok(result),
            ExecutionResult::Chain(mut results) if results.len() == 1 => Ok(results.pop().unwrap()),
            _ => Err(NanonisError::InvalidCommand(
                "Expected single result".to_string(),
            )),
        }
    }

    /// Extract chain results or error if not chain
    pub fn into_chain(self) -> Result<Vec<ActionResult>, NanonisError> {
        match self {
            ExecutionResult::Chain(results) => Ok(results),
            ExecutionResult::Single(result) => Ok(vec![result]),
            ExecutionResult::Partial(results, _) => Ok(results),
            _ => Err(NanonisError::InvalidCommand(
                "Expected chain results".to_string(),
            )),
        }
    }

    /// Extract experiment data or error if not experiment
    pub fn into_experiment_data(self) -> Result<crate::types::ExperimentData, NanonisError> {
        match self {
            ExecutionResult::ExperimentData(data) => Ok(data),
            _ => Err(NanonisError::InvalidCommand(
                "Expected experiment data".to_string(),
            )),
        }
    }

    /// Extract chain experiment data or error if not chain experiment
    pub fn into_chain_experiment_data(self) -> Result<crate::types::ChainExperimentData, NanonisError> {
        match self {
            ExecutionResult::ChainExperimentData(data) => Ok(data),
            _ => Err(NanonisError::InvalidCommand(
                "Expected chain experiment data".to_string(),
            )),
        }
    }

    /// Type-safe extraction with action validation
    pub fn expecting<T>(self) -> Result<T, NanonisError>
    where
        Self: ExpectFromExecution<T>,
    {
        self.expect_from_execution()
    }
}

/// Trait for type-safe extraction from ExecutionResult
pub trait ExpectFromExecution<T> {
    fn expect_from_execution(self) -> Result<T, NanonisError>;
}

/// Builder for fluent configuration of execution
pub struct ExecutionBuilder<'a> {
    driver: &'a mut ActionDriver,
    request: ActionRequest,
    config: ExecutionConfig,
}

impl<'a> ExecutionBuilder<'a> {
    fn new(driver: &'a mut ActionDriver, request: ActionRequest) -> Self {
        Self {
            driver,
            request,
            config: ExecutionConfig::default(),
        }
    }

    /// Enable data collection with specified durations
    pub fn with_data_collection(mut self, pre_duration: Duration, post_duration: Duration) -> Self {
        self.config = self.config.with_data_collection(pre_duration, post_duration);
        self
    }

    /// Return only final result for chains
    pub fn final_only(mut self) -> Self {
        self.config = self.config.final_only();
        self
    }

    /// Allow partial execution on error
    pub fn partial(mut self) -> Self {
        self.config = self.config.partial();
        self
    }

    /// Use deferred logging
    pub fn deferred_logging(mut self) -> Self {
        self.config = self.config.deferred_logging();
        self
    }

    /// Disable logging for this execution
    pub fn no_logging(mut self) -> Self {
        self.config = self.config.no_logging();
        self
    }

    /// Enable fast performance mode
    pub fn fast_mode(mut self) -> Self {
        self.config = self.config.fast_mode();
        self
    }

    /// Execute with type-safe result extraction
    pub fn expecting<T>(self) -> Result<T, NanonisError>
    where
        ExecutionResult: ExpectFromExecution<T>,
    {
        let result = self.driver.run_with_config(self.request, self.config)?;
        result.expecting()
    }

    /// Execute and return ExecutionResult
    pub fn execute(self) -> Result<ExecutionResult, NanonisError> {
        self.driver.run_with_config(self.request, self.config)
    }
}

impl<'a> ExecutionBuilder<'a> {
    /// Convenience method for single actions - returns ActionResult directly
    pub fn go(self) -> Result<ActionResult, NanonisError> {
        match self.request {
            ActionRequest::Single(_) => {
                let result = self.driver.run_with_config(self.request, self.config)?;
                result.into_single()
            }
            ActionRequest::Chain(_) => {
                Err(NanonisError::InvalidCommand(
                    "Use .execute() for chains, .go() is only for single actions".to_string(),
                ))
            }
        }
    }
}

/// Builder for configuring ActionDriver with optional parameters
#[derive(Debug, Clone)]
pub struct ActionDriverBuilder {
    addr: String,
    port: u16,
    connection_timeout: Option<Duration>,
    initial_storage: HashMap<String, ActionResult>,
    tcp_logger_config: Option<TCPLoggerConfig>,
    action_logger_config: Option<(std::path::PathBuf, usize, bool)>, // (file_path, buffer_size, final_format_json)
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
            action_logger_config: None,
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

    /// Configure TCP Logger for basic integration (no automatic buffering)
    /// Use this when you want manual control over data collection
    pub fn with_tcp_logger(mut self, config: TCPLoggerConfig) -> Self {
        self.tcp_logger_config = Some(config);
        self
    }

    /// Configure TCP Logger with always-buffer mode (recommended)
    /// This automatically starts BufferedTCPReader when ActionDriver is built
    ///
    /// # Arguments
    /// * `config` - TCP logger configuration with buffer_size set
    ///
    /// # Usage
    /// ```rust,ignore
    /// let driver = ActionDriver::builder("127.0.0.1", 6501)
    ///     .with_tcp_logger_buffering(TCPLoggerConfig {
    ///         stream_port: 6590,
    ///         channels: vec![0, 8],
    ///         oversampling: 100,
    ///         auto_start: true,
    ///         buffer_size: Some(10_000),
    ///     })
    ///     .build()?;
    /// // Buffering is now active and ready for immediate data queries
    /// ```
    pub fn with_tcp_logger_buffering(mut self, config: TCPLoggerConfig) -> Self {
        if config.buffer_size.is_none() {
            log::warn!("TCPLoggerConfig buffer_size is None - buffering disabled");
        }
        self.tcp_logger_config = Some(config);
        self
    }

    /// Configure action logging with buffered file output
    ///
    /// # Arguments
    /// * `file_path` - Base path where action logs will be written (extension added automatically)
    /// * `buffer_size` - Number of actions to buffer before auto-flushing to file
    /// * `final_format_json` - If true, convert to JSON array on final flush; if false, keep JSONL format
    ///
    /// # File Extensions
    /// File extensions are added automatically based on the final format:
    /// - `final_format_json = false` → `.jsonl` extension (efficient streaming)
    /// - `final_format_json = true` → `.json` extension (post-analysis friendly)
    ///
    /// # Usage
    /// ```rust,ignore
    /// // JSONL format (efficient, streaming) → experiment_actions.jsonl
    /// let driver = ActionDriver::builder("127.0.0.1", 6501)
    ///     .with_action_logging("experiment_actions", 100, false)
    ///     .build()?;
    ///
    /// // JSON format (better for post-analysis) → experiment_data.json
    /// let driver = ActionDriver::builder("127.0.0.1", 6501)
    ///     .with_action_logging("experiment_data", 100, true)
    ///     .build()?;
    /// ```
    pub fn with_action_logging(mut self, file_path: impl Into<std::path::PathBuf>, buffer_size: usize, final_format_json: bool) -> Self {
        self.action_logger_config = Some((file_path.into(), buffer_size, final_format_json));
        self
    }

    /// Build the ActionDriver with configured parameters and optional automatic buffering
    pub fn build(self) -> Result<ActionDriver, NanonisError> {
        let mut client = NanonisClient::new(&self.addr, self.port)?;

        let tcp_reader = if let Some(ref config) = self.tcp_logger_config {
            if let Some(buffer_size) = config.buffer_size {
                // 1. Configure TCP logger settings first
                client.tcplog_chs_set(config.channels.clone())?;
                client.tcplog_oversampl_set(config.oversampling)?;

                // 2. Connect TCP stream BEFORE starting logger (critical sequence!)
                let reader = crate::buffered_tcp_reader::BufferedTCPReader::new(
                    "127.0.0.1",
                    config.stream_port,
                    buffer_size,
                    config.channels.len() as u32,
                    config.oversampling as f32,
                )?;
                log::info!(
                    "TCP stream connected, buffer capacity: {} frames",
                    buffer_size
                );

                // 3. NOW start TCP logger (data flows to connected reader)
                if config.auto_start {
                    // Reset TCP logger state first to ensure clean start
                    log::info!("Stopping TCP logger to ensure clean state");
                    let _ = client.tcplog_stop(); // Ignore errors - might not be running
                    std::thread::sleep(std::time::Duration::from_millis(200)); // Give it time to stop

                    // Now start TCP logger
                    client.tcplog_start()?;
                    log::info!("TCP logger started, data collection active");
                }

                Some(reader)
            } else {
                None
            }
        } else {
            None
        };

        // Create action logger if configured
        let action_logger = if let Some((file_path, buffer_size, final_format_json)) = self.action_logger_config {
            Some(crate::logger::Logger::new(file_path, buffer_size, final_format_json))
        } else {
            None
        };

        Ok(ActionDriver {
            client,
            stored_values: self.initial_storage,
            tcp_logger_config: self.tcp_logger_config,
            tcp_receiver: None,
            tcp_reader,
            action_logger,
            action_logging_enabled: true, // Default to enabled if logger is configured
        })
    }
}

/// Direct 1:1 translation layer between Actions and NanonisClient calls
/// Now with integrated always-buffer TCP data collection capability
pub struct ActionDriver {
    /// Nanonis control client for sending commands
    client: NanonisClient,
    /// Storage for Store/Retrieve actions
    stored_values: HashMap<String, ActionResult>,
    /// TCP Logger configuration for data collection
    tcp_logger_config: Option<TCPLoggerConfig>,
    /// Legacy receiver for backward compatibility (deprecated in favor of tcp_reader)
    tcp_receiver: Option<mpsc::Receiver<TCPLoggerData>>,
    /// Buffered TCP reader for always-buffer mode (automatically started if configured)
    tcp_reader: Option<crate::buffered_tcp_reader::BufferedTCPReader>,
    /// Action logger for execution tracking
    action_logger: Option<crate::logger::Logger<crate::actions::ActionLogEntry>>,
    /// Enable/disable action logging at runtime
    action_logging_enabled: bool,
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
            tcp_receiver: None,
            tcp_reader: None,
            action_logger: None,
            action_logging_enabled: false,
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

    /// Get reference to TCP logger data receiver for direct channel access
    /// This is the primary way to read TCP logger data from the background thread
    pub fn tcp_logger_receiver(&self) -> Option<&mpsc::Receiver<TCPLoggerData>> {
        self.tcp_receiver.as_ref()
    }

    /// Check if TCP logger is configured and available
    pub fn has_tcp_logger(&self) -> bool {
        self.tcp_receiver.is_some() || self.tcp_reader.is_some()
    }

    // ==================== Unified Execution API ====================

    /// Unified execution method with fluent configuration
    /// 
    /// # Usage
    /// ```rust,ignore
    /// // Simple execution
    /// let result = driver.run(action)?;
    /// let results = driver.run(actions)?;
    /// 
    /// // With data collection
    /// let data = driver.run(action).with_data_collection(pre, post).execute()?;
    /// 
    /// // Type-safe extraction
    /// let signal: f64 = driver.run(read_signal).expecting()?;
    /// 
    /// // Performance modes
    /// let results = driver.run(actions).deferred_logging().execute()?;
    /// let final_result = driver.run(actions).final_only().execute()?;
    /// ```
    pub fn run<R>(&mut self, request: R) -> ExecutionBuilder<'_>
    where
        R: Into<ActionRequest>,
    {
        ExecutionBuilder::new(self, request.into())
    }

    /// Execute with explicit configuration (for advanced use)
    pub fn run_with_config(&mut self, request: ActionRequest, config: ExecutionConfig) -> Result<ExecutionResult, NanonisError> {
        match (&request, &config.data_collection) {
            // Single action with data collection
            (ActionRequest::Single(action), Some((pre_duration, post_duration))) => {
                let experiment_data = self.execute_with_data_collection(
                    action.clone(),
                    *pre_duration,
                    *post_duration,
                )?;
                Ok(ExecutionResult::ExperimentData(experiment_data))
            }

            // Chain with data collection
            (ActionRequest::Chain(actions), Some((pre_duration, post_duration))) => {
                let chain_experiment_data = self.execute_chain_with_data_collection(
                    actions.clone(),
                    *pre_duration,
                    *post_duration,
                )?;
                Ok(ExecutionResult::ChainExperimentData(chain_experiment_data))
            }

            // Single action without data collection
            (ActionRequest::Single(action), None) => {
                let result = match config.logging_behavior {
                    LoggingBehavior::Disabled => {
                        let previous_state = self.set_action_logging_enabled(false);
                        let result = self.execute(action.clone());
                        self.set_action_logging_enabled(previous_state);
                        result
                    }
                    _ => self.execute(action.clone()),
                }?;
                Ok(ExecutionResult::Single(result))
            }

            // Chain without data collection
            (ActionRequest::Chain(actions), None) => {
                let results = match (&config.chain_behavior, &config.logging_behavior) {
                    (ChainBehavior::Complete, LoggingBehavior::Normal) => {
                        self.execute_chain(actions.clone())?
                    }
                    (ChainBehavior::Complete, LoggingBehavior::Deferred) => {
                        self.execute_chain_deferred(actions.clone())?
                    }
                    (ChainBehavior::Complete, LoggingBehavior::Disabled) => {
                        let previous_state = self.set_action_logging_enabled(false);
                        let result = self.execute_chain(actions.clone());
                        self.set_action_logging_enabled(previous_state);
                        result?
                    }
                    (ChainBehavior::FinalOnly, _) => {
                        let results = match config.logging_behavior {
                            LoggingBehavior::Deferred => self.execute_chain_deferred(actions.clone())?,
                            LoggingBehavior::Disabled => {
                                let previous_state = self.set_action_logging_enabled(false);
                                let result = self.execute_chain(actions.clone());
                                self.set_action_logging_enabled(previous_state);
                                result?
                            }
                            _ => self.execute_chain(actions.clone())?,
                        };
                        vec![results.into_iter().last().unwrap_or(ActionResult::None)]
                    }
                    (ChainBehavior::Partial, _) => {
                        match self.execute_chain_partial(actions.clone()) {
                            Ok(results) => results,
                            Err((partial_results, error)) => {
                                return Ok(ExecutionResult::Partial(partial_results, error));
                            }
                        }
                    }
                };

                Ok(ExecutionResult::Chain(results))
            }
        }
    }

    // ==================== Always-Buffer TCP Data Collection Methods ====================

    /// Get recent TCP signal data (always available if buffering enabled)
    ///
    /// # Arguments
    /// * `duration` - How far back to collect data from current time
    ///
    /// # Returns
    /// Vector of recent timestamped signal frames, empty if buffering not active
    ///
    /// # Usage
    /// Perfect for real-time monitoring and checking recent signal trends without
    /// needing to plan data collection in advance
    pub fn get_recent_tcp_data(
        &self,
        duration: Duration,
    ) -> Vec<crate::types::TimestampedSignalFrame> {
        self.tcp_reader
            .as_ref()
            .map(|reader| reader.get_recent_data(duration))
            .unwrap_or_default()
    }

    /// Execute action with time-windowed data collection
    ///
    /// This is the core method for synchronized data collection during SPM operations.
    /// It captures data before, during, and after action execution using the always-buffer.
    ///
    /// # Arguments
    /// * `action` - The SPM action to execute
    /// * `pre_duration` - How much data to collect before action starts
    /// * `post_duration` - How much data to collect after action ends
    ///
    /// # Returns
    /// ExperimentData containing both action result and time-windowed signal data
    ///
    /// # Errors
    /// Returns error if buffering is not active or action execution fails
    pub fn execute_with_data_collection(
        &mut self,
        action: Action,
        pre_duration: Duration,
        post_duration: Duration,
    ) -> Result<crate::types::ExperimentData, NanonisError> {
        if self.tcp_reader.is_none() {
            return Err(NanonisError::InvalidCommand(
                "TCP buffering not active".to_string(),
            ));
        }

        let action_start = Instant::now();
        let action_result = self.execute(action.clone())?;
        let action_end = Instant::now();

        std::thread::sleep(post_duration);

        let window_start = action_start - pre_duration;
        let window_end = action_end + post_duration;

        let signal_frames = self
            .tcp_reader
            .as_ref()
            .unwrap()
            .get_data_between(window_start, window_end);
        let tcp_config = self.tcp_logger_config.as_ref().unwrap().clone();

        let experiment_data = crate::types::ExperimentData {
            action_result,
            signal_frames,
            tcp_config,
            action_start,
            action_end,
            total_duration: action_end.duration_since(action_start),
        };

        // Log the complete experiment data if logging is enabled
        if self.action_logging_enabled && self.action_logger.is_some() {
            let log_entry = ActionLogEntry {
                action: format!("Data Collection: {}", action.description()),
                result: ActionLogResult::from_experiment_data(&experiment_data),
                start_time: chrono::Utc::now(),
                duration_ms: experiment_data.total_duration.as_millis() as u64,
                metadata: Some([
                    ("type".to_string(), "experiment_data_collection".to_string()),
                    ("pre_duration_ms".to_string(), pre_duration.as_millis().to_string()),
                    ("post_duration_ms".to_string(), post_duration.as_millis().to_string()),
                    ("signal_frame_count".to_string(), experiment_data.signal_frames.len().to_string()),
                ].into_iter().collect()),
            };
            
            if let Err(log_error) = self.action_logger.as_mut().unwrap().add(log_entry) {
                log::warn!("Failed to log experiment data: {}", log_error);
            }
        }

        Ok(experiment_data)
    }

    /// Convenience method for bias pulse with data collection
    ///
    /// # Arguments
    /// * `pulse_voltage` - Bias voltage for the pulse (V)
    /// * `pulse_duration` - Duration of the pulse
    /// * `pre_duration` - Data collection before pulse
    /// * `post_duration` - Data collection after pulse
    ///
    /// # Returns
    /// ExperimentData with pulse results and synchronized signal data
    pub fn pulse_with_data_collection(
        &mut self,
        pulse_voltage: f32,
        pulse_duration: Duration,
        pre_duration: Duration,
        post_duration: Duration,
    ) -> Result<crate::types::ExperimentData, NanonisError> {
        self.execute_with_data_collection(
            Action::BiasPulse {
                wait_until_done: true,
                bias_value_v: pulse_voltage,
                pulse_width: pulse_duration,
                z_controller_hold: crate::types::ZControllerHold::Hold as u16,
                pulse_mode: crate::types::PulseMode::Absolute as u16,
            },
            pre_duration,
            post_duration,
        )
    }

    /// Get current buffer statistics if buffering is active
    ///
    /// # Returns
    /// Optional tuple of (current_count, max_capacity, time_span) or None if no buffering
    ///
    /// # Usage
    /// Monitor buffer health, detect overruns, check data collection status
    pub fn tcp_buffer_stats(&self) -> Option<(usize, usize, Duration)> {
        self.tcp_reader.as_ref().map(|reader| reader.buffer_stats())
    }

    /// Stop TCP buffering and return final buffer state
    ///
    /// # Returns
    /// Vector containing all buffered data, or empty if buffering wasn't active
    ///
    /// # Usage
    /// Optional manual cleanup - this happens automatically via Drop trait.
    /// Call this only if you need to access the final buffered data before ActionDriver is dropped.
    pub fn stop_tcp_buffering(
        &mut self,
    ) -> Result<Vec<crate::types::TimestampedSignalFrame>, NanonisError> {
        if let Some(mut reader) = self.tcp_reader.take() {
            let final_data = reader.get_all_data();
            reader.stop()?;
            log::info!(
                "Manually stopped TCP buffering, collected {} frames",
                final_data.len()
            );
            Ok(final_data)
        } else {
            Ok(Vec::new())
        }
    }

    /// Execute action chain with time-windowed data collection
    ///
    /// This executes a sequence of actions while continuously collecting signal data,
    /// providing precise timing information for each action in the chain.
    ///
    /// # Arguments
    /// * `actions` - Vector of actions to execute in sequence
    /// * `pre_duration` - How much data to collect before chain starts
    /// * `post_duration` - How much data to collect after chain ends
    ///
    /// # Returns
    /// ChainExperimentData containing results and timing for each action plus synchronized signal data
    ///
    /// # Errors
    /// Returns error if buffering is not active or any action execution fails
    pub fn execute_chain_with_data_collection(
        &mut self,
        actions: Vec<Action>,
        pre_duration: Duration,
        post_duration: Duration,
    ) -> Result<crate::types::ChainExperimentData, NanonisError> {
        if self.tcp_reader.is_none() {
            return Err(NanonisError::InvalidCommand(
                "TCP buffering not active".to_string(),
            ));
        }

        let chain_start = Instant::now();
        let mut action_results = Vec::with_capacity(actions.len());
        let mut action_timings = Vec::with_capacity(actions.len());

        // Execute each action and track timing
        for action in actions {
            let action_start = Instant::now();
            let action_result = self.execute(action)?;
            let action_end = Instant::now();

            action_results.push(action_result);
            action_timings.push((action_start, action_end));
        }

        let chain_end = Instant::now();

        // Wait for post-chain data to be collected
        std::thread::sleep(post_duration);

        // Query buffered data for the entire time window
        let window_start = chain_start - pre_duration;
        let window_end = chain_end + post_duration;

        let signal_frames = self
            .tcp_reader
            .as_ref()
            .unwrap()
            .get_data_between(window_start, window_end);
        let tcp_config = self.tcp_logger_config.as_ref().unwrap().clone();

        let chain_experiment_data = crate::types::ChainExperimentData {
            action_results,
            signal_frames,
            tcp_config,
            action_timings,
            chain_start,
            chain_end,
            total_duration: chain_end.duration_since(chain_start),
        };

        // Log the complete chain experiment data if logging is enabled
        if self.action_logging_enabled && self.action_logger.is_some() {
            let log_entry = ActionLogEntry {
                action: format!("Chain Data Collection: {} actions", chain_experiment_data.action_results.len()),
                result: ActionLogResult::from_chain_experiment_data(&chain_experiment_data),
                start_time: chrono::Utc::now(),
                duration_ms: chain_experiment_data.total_duration.as_millis() as u64,
                metadata: Some([
                    ("type".to_string(), "chain_experiment_data_collection".to_string()),
                    ("pre_duration_ms".to_string(), pre_duration.as_millis().to_string()),
                    ("post_duration_ms".to_string(), post_duration.as_millis().to_string()),
                    ("action_count".to_string(), chain_experiment_data.action_results.len().to_string()),
                    ("signal_frame_count".to_string(), chain_experiment_data.signal_frames.len().to_string()),
                ].into_iter().collect()),
            };
            
            if let Err(log_error) = self.action_logger.as_mut().unwrap().add(log_entry) {
                log::warn!("Failed to log chain experiment data: {}", log_error);
            }
        }

        Ok(chain_experiment_data)
    }

    /// Start TCP logger
    pub fn start_tcp_logger(&mut self) -> Result<(), NanonisError> {
        self.client.tcplog_start()
    }

    /// Stop TCP logger
    pub fn stop_tcp_logger(&mut self) -> Result<(), NanonisError> {
        self.client.tcplog_stop()
    }

    /// Configure TCP logger channels
    pub fn set_tcp_logger_channels(&mut self, channels: Vec<i32>) -> Result<(), NanonisError> {
        self.client.tcplog_chs_set(channels)
    }

    /// Set TCP logger oversampling
    pub fn set_tcp_logger_oversampling(&mut self, oversampling: i32) -> Result<(), NanonisError> {
        self.client.tcplog_oversampl_set(oversampling)
    }

    /// Get TCP logger status
    pub fn get_tcp_logger_status(&mut self) -> Result<crate::types::TCPLogStatus, NanonisError> {
        self.client.tcplog_status_get()
    }

    /// Execute a single action with direct 1:1 mapping to client methods
    pub fn execute(&mut self, action: Action) -> Result<ActionResult, NanonisError> {
        let start_time = chrono::Utc::now();
        let start_instant = std::time::Instant::now();
        
        let result = self.execute_internal(action.clone());
        
        let duration = start_instant.elapsed();
        
        // Log the action execution if logging is enabled
        if self.action_logging_enabled && self.action_logger.is_some() {
            let log_entry = match &result {
                Ok(action_result) => ActionLogEntry::new(&action, action_result, start_time, duration),
                Err(error) => ActionLogEntry::new_error(&action, error, start_time, duration),
            };
            
            if let Err(log_error) = self.action_logger.as_mut().unwrap().add(log_entry) {
                log::warn!("Failed to log action: {}", log_error);
            }
        }
        
        result
    }

    /// Execute action with optional data collection (unified interface)
    ///
    /// This provides a single interface for both normal execution and data collection.
    /// When data_collection is true, this method collects TCP signal data alongside action execution.
    ///
    /// # Arguments
    /// * `action` - The action to execute
    /// * `data_collection` - If true, collect TCP signal data (requires TCP reader to be active)
    /// * `pre_duration` - How much data to collect before action (only used if data_collection=true)
    /// * `post_duration` - How much data to collect after action (only used if data_collection=true)
    ///
    /// # Returns
    /// ActionResult for normal execution, or ActionResult::ExperimentData for data collection
    ///
    /// # Usage
    /// ```rust,ignore
    /// // Normal execution
    /// let result = driver.execute_with_options(action, false, Duration::ZERO, Duration::ZERO)?;
    /// 
    /// // With data collection
    /// let result = driver.execute_with_options(action, true, Duration::from_millis(100), Duration::from_millis(200))?;
    /// ```
    pub fn execute_with_options(
        &mut self,
        action: Action,
        data_collection: bool,
        pre_duration: Duration,
        post_duration: Duration,
    ) -> Result<ActionResult, NanonisError> {
        if data_collection && self.tcp_reader.is_some() {
            // Use data collection execution
            let _experiment_data = self.execute_with_data_collection(action, pre_duration, post_duration)?;
            // Convert ExperimentData to ActionResult for unified return type
            Ok(ActionResult::Success) // For now, return Success - could extend ActionResult to include ExperimentData
        } else {
            // Use normal execution
            self.execute(action)
        }
    }

    /// Execute chain with optional data collection (unified interface)
    ///
    /// # Arguments
    /// * `chain` - The action chain to execute
    /// * `data_collection` - If true, collect TCP signal data for the entire chain
    /// * `pre_duration` - How much data to collect before chain starts
    /// * `post_duration` - How much data to collect after chain ends
    ///
    /// # Returns
    /// Vector of ActionResults
    pub fn execute_chain_with_options(
        &mut self,
        chain: impl Into<ActionChain>,
        data_collection: bool,
        pre_duration: Duration,
        post_duration: Duration,
    ) -> Result<Vec<ActionResult>, NanonisError> {
        if data_collection && self.tcp_reader.is_some() {
            // Use data collection execution
            let chain_experiment_data = self.execute_chain_with_data_collection(
                chain.into().into_iter().collect(), 
                pre_duration, 
                post_duration
            )?;
            // Return the action results from the chain
            Ok(chain_experiment_data.action_results)
        } else {
            // Use normal execution
            self.execute_chain(chain)
        }
    }

    /// Internal execute method without logging (for performance-critical chains)
    fn execute_internal(&mut self, action: Action) -> Result<ActionResult, NanonisError> {
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

            // === TCP Logger Operations ===
            Action::StartTCPLogger => {
                self.start_tcp_logger()?;
                Ok(ActionResult::Success)
            }

            Action::StopTCPLogger => {
                self.stop_tcp_logger()?;
                Ok(ActionResult::Success)
            }

            Action::GetTCPLoggerStatus => {
                let status = self.get_tcp_logger_status()?;
                let config = self.tcp_logger_config();

                Ok(ActionResult::TCPLoggerStatus {
                    status,
                    channels: config.map(|c| c.channels.clone()).unwrap_or_default(),
                    oversampling: config.map(|c| c.oversampling).unwrap_or(0),
                })
            }

            Action::ConfigureTCPLogger {
                channels,
                oversampling,
            } => {
                self.set_tcp_logger_channels(channels)?;
                self.set_tcp_logger_oversampling(oversampling)?;
                Ok(ActionResult::Success)
            }

            Action::CheckTipState { method } => {
                use crate::actions::TipCheckMethod;
                use crate::tip_prep::TipState;
                
                let tip_state = match method {
                    TipCheckMethod::SignalBounds { signal, bounds } => {
                        let value = self.client.signals_vals_get(vec![signal.into()], true)?[0];
                        if value >= bounds.0 && value <= bounds.1 {
                            TipState::Good
                        } else {
                            TipState::Bad
                        }
                    }
                    
                    TipCheckMethod::MultiSignalBounds { signals } => {
                        let indices: Vec<i32> = signals.iter().map(|(signal, _)| (*signal).into()).collect();
                        let values = self.client.signals_vals_get(indices, true)?;
                        
                        let all_good = signals.iter().zip(values.iter()).all(|((_signal, bounds), value)| {
                            *value >= bounds.0 && *value <= bounds.1
                        });
                        
                        if all_good {
                            TipState::Good
                        } else {
                            TipState::Bad
                        }
                    }
                    
                    TipCheckMethod::SignalStability { signal, threshold, history_size } => {
                        // Read multiple samples for stability analysis
                        let mut samples = Vec::with_capacity(history_size);
                        for _ in 0..history_size {
                            let value = self.client.signals_vals_get(vec![signal.into()], true)?[0];
                            samples.push(value);
                            std::thread::sleep(std::time::Duration::from_millis(10)); // Small delay between readings
                        }
                        
                        // Calculate standard deviation
                        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
                        let variance = samples.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / samples.len() as f32;
                        let std_dev = variance.sqrt();
                        
                        if std_dev <= threshold {
                            TipState::Stable
                        } else {
                            TipState::Good // Assume signal is in bounds but not stable
                        }
                    }
                    
                    TipCheckMethod::Custom { method_name } => {
                        // For custom methods, return a default state and log the method name
                        log::warn!("Custom tip check method '{}' not implemented, returning Good", method_name);
                        TipState::Good
                    }
                };
                
                Ok(ActionResult::TipState(tip_state))
            }
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

    /// Execute chain with deferred logging for timing-critical operations
    ///
    /// This method executes all actions using execute_internal() (no per-action logging)
    /// and then logs the entire chain as a single entry with total timing.
    /// Use this when you need precise timing without logging overhead between actions.
    ///
    /// # Arguments
    /// * `chain` - The action chain to execute
    ///
    /// # Returns
    /// Vector of all action results
    ///
    /// # Logging Behavior
    /// - Individual actions are NOT logged during execution
    /// - Single log entry created for the entire chain with total duration
    /// - Log entry includes chain summary and final result
    pub fn execute_chain_deferred(
        &mut self,
        chain: impl Into<ActionChain>,
    ) -> Result<Vec<ActionResult>, NanonisError> {
        let chain = chain.into();
        let start_time = chrono::Utc::now();
        let start_instant = std::time::Instant::now();
        
        let mut results = Vec::with_capacity(chain.len());
        
        // Execute all actions without per-action logging
        for action in chain.iter() {
            let result = self.execute_internal(action.clone())?;
            results.push(result);
        }
        
        let duration = start_instant.elapsed();
        
        // Log the entire chain as a single entry if logging is enabled
        if self.action_logging_enabled && self.action_logger.is_some() {
            let chain_summary = format!("Chain: {}", chain.summary());
            let final_result = results.last().unwrap_or(&ActionResult::None);
            
            let log_entry = ActionLogEntry::new(
                &crate::actions::Action::Wait { duration: Duration::from_millis(0) }, // Placeholder action
                final_result,
                start_time,
                duration,
            )
            .with_metadata("type", "chain_execution")
            .with_metadata("chain_summary", chain_summary)
            .with_metadata("action_count", results.len().to_string());
            
            if let Err(log_error) = self.action_logger.as_mut().unwrap().add(log_entry) {
                log::warn!("Failed to log chain execution: {}", log_error);
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

    // ==================== Action Logging Control Methods ====================

    /// Enable or disable action logging at runtime
    ///
    /// # Arguments
    /// * `enabled` - true to enable logging, false to disable
    ///
    /// # Returns
    /// Previous logging state
    ///
    /// # Usage
    /// ```rust,ignore
    /// let previous_state = driver.set_action_logging_enabled(false);
    /// // Execute timing-critical operations without logging overhead
    /// driver.execute(critical_action)?;
    /// driver.set_action_logging_enabled(previous_state); // Restore
    /// ```
    pub fn set_action_logging_enabled(&mut self, enabled: bool) -> bool {
        let previous = self.action_logging_enabled;
        self.action_logging_enabled = enabled;
        previous
    }

    /// Check if action logging is currently enabled
    pub fn is_action_logging_enabled(&self) -> bool {
        self.action_logging_enabled && self.action_logger.is_some()
    }

    /// Manually flush the action log buffer to file
    ///
    /// # Returns
    /// Result indicating if flush was successful
    ///
    /// # Usage
    /// Force immediate write of buffered actions to file, useful before
    /// critical operations or at experiment checkpoints
    pub fn flush_action_log(&mut self) -> Result<(), NanonisError> {
        if let Some(ref mut logger) = self.action_logger {
            logger.flush()?;
        }
        Ok(())
    }

    /// Get action log buffer statistics
    ///
    /// # Returns
    /// Optional tuple of (current_buffer_count, is_logging_enabled) or None if no logger
    ///
    /// # Usage
    /// Monitor buffer utilization to understand logging overhead and frequency
    pub fn action_log_stats(&self) -> Option<(usize, bool)> {
        self.action_logger.as_ref().map(|logger| {
            (logger.len(), self.action_logging_enabled)
        })
    }

    /// Finalize action log as JSON array (if configured for JSON output)
    ///
    /// # Returns
    /// Result indicating if finalization was successful
    ///
    /// # Usage
    /// Call this at the end of your experiment to convert JSONL to JSON format
    /// for easier post-experiment analysis. This happens automatically on drop,
    /// but you can call it manually for explicit control.
    pub fn finalize_action_log(&mut self) -> Result<(), NanonisError> {
        if let Some(ref mut logger) = self.action_logger {
            logger.finalize_as_json()?;
        }
        Ok(())
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

// ==================== Type-Safe Extraction Implementations ====================

impl ExpectFromExecution<ActionResult> for ExecutionResult {
    fn expect_from_execution(self) -> Result<ActionResult, NanonisError> {
        self.into_single()
    }
}

impl ExpectFromExecution<Vec<ActionResult>> for ExecutionResult {
    fn expect_from_execution(self) -> Result<Vec<ActionResult>, NanonisError> {
        self.into_chain()
    }
}

impl ExpectFromExecution<crate::types::ExperimentData> for ExecutionResult {
    fn expect_from_execution(self) -> Result<crate::types::ExperimentData, NanonisError> {
        self.into_experiment_data()
    }
}

impl ExpectFromExecution<crate::types::ChainExperimentData> for ExecutionResult {
    fn expect_from_execution(self) -> Result<crate::types::ChainExperimentData, NanonisError> {
        self.into_chain_experiment_data()
    }
}

impl ExpectFromExecution<f64> for ExecutionResult {
    fn expect_from_execution(self) -> Result<f64, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::Value(v)) => Ok(v),
            ExecutionResult::Single(ActionResult::Values(mut vs)) if vs.len() == 1 => {
                Ok(vs.pop().unwrap())
            }
            _ => Err(NanonisError::InvalidCommand(
                "Expected single numeric value".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<Vec<f64>> for ExecutionResult {
    fn expect_from_execution(self) -> Result<Vec<f64>, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::Values(vs)) => Ok(vs),
            ExecutionResult::Single(ActionResult::Value(v)) => Ok(vec![v]),
            _ => Err(NanonisError::InvalidCommand(
                "Expected numeric values".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<bool> for ExecutionResult {
    fn expect_from_execution(self) -> Result<bool, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::Status(b)) => Ok(b),
            _ => Err(NanonisError::InvalidCommand(
                "Expected boolean status".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<Position> for ExecutionResult {
    fn expect_from_execution(self) -> Result<Position, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::Position(pos)) => Ok(pos),
            _ => Err(NanonisError::InvalidCommand(
                "Expected position data".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<OsciData> for ExecutionResult {
    fn expect_from_execution(self) -> Result<OsciData, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::OsciData(data)) => Ok(data),
            _ => Err(NanonisError::InvalidCommand(
                "Expected oscilloscope data".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<crate::tip_prep::TipState> for ExecutionResult {
    fn expect_from_execution(self) -> Result<crate::tip_prep::TipState, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::TipState(state)) => Ok(state),
            _ => Err(NanonisError::InvalidCommand(
                "Expected tip state".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<Vec<String>> for ExecutionResult {
    fn expect_from_execution(self) -> Result<Vec<String>, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::Text(text)) => Ok(text),
            _ => Err(NanonisError::InvalidCommand(
                "Expected text data".to_string(),
            )),
        }
    }
}

impl Drop for ActionDriver {
    fn drop(&mut self) {
        // Clean up TCP buffering first
        if let Some(mut reader) = self.tcp_reader.take() {
            let final_data = reader.get_all_data();
            let _ = reader.stop(); // Ignore errors during cleanup
            log::info!(
                "ActionDriver cleanup: Stopped TCP buffering, collected {} frames",
                final_data.len()
            );
        }

        // Perform safe shutdown sequence
        let _ = self.execute_chain(vec![
            Action::Withdraw {
                wait_until_finished: false,
                timeout: Duration::from_secs(1),
            },
            Action::MoveMotorAxis {
                direction: crate::MotorDirection::ZMinus,
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
