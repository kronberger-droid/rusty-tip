use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use log::{debug, info, warn};
use nanonis_rs::signals::SignalIndex;
use ndarray::Array1;

use crate::{
    actions::{
        Action, ActionChain, ActionLogEntry, ActionLogResult, ActionResult,
        ExpectFromAction,
    },
    buffered_tcp_reader::BufferedTCPReader,
    signal_registry::SignalRegistry,
    types::{DataToGet, OsciData, SignalStats, TriggerConfig},
    utils::{poll_until, poll_with_timeout, PollError},
    MotorGroup, NanonisClient, NanonisError, Position, PulseMode, ScanAction,
    ScanDirection, Signal, TipShaperConfig, ZControllerHold,
};

// ========================================================================
// TIP STATE CHECKING CONSTANTS
// ========================================================================

/// Maximum standard deviation for stable signal (Hz)
/// This checks the noise level in the signal
/// Typical values: 0.3-0.5 for frequency shift signals with moderate noise
/// Increased to 1.0 for noisy signals - adjust based on your actual noise level
const TIP_STATE_MAX_STD_DEV: f32 = 1.0;

/// Maximum slope for stable signal (Hz per sample)
/// This checks for drift/trend in the signal
/// Slope is calculated via linear regression over the data window
/// Typical values: 0.001-0.01 depending on your signal drift rate
/// Increased to 0.01 for drifting signals - adjust based on your actual drift
const TIP_STATE_MAX_SLOPE: f32 = 0.01;

/// Duration of data collection for tip state checking (milliseconds)
const TIP_STATE_DATA_COLLECTION_DURATION_MS: u64 = 500;

/// Timeout for stable signal reading during tip state check (seconds)
const TIP_STATE_READ_TIMEOUT_SECS: u64 = 15;

/// Number of retries for stable signal reading during tip state check
const TIP_STATE_READ_RETRY_COUNT: u32 = 3;

/// Configuration for TCP Logger integration with always-buffer support
#[derive(Debug, Clone)]
pub struct TCPReaderConfig {
    /// TCP data stream port (typically 6590)
    pub stream_port: u16,
    /// Signal channel indices to record (0-23)
    pub channels: Vec<i32>,
    /// Oversampling rate multiplier (0-1000)
    pub oversampling: i32,
    /// Whether to start logging automatically on connection
    pub auto_start: bool,
    /// Buffer size for always-buffer mode (None = no buffering)
    /// When Some(size), BufferedTCPReader starts automatically
    pub buffer_size: Option<usize>,
}

impl Default for TCPReaderConfig {
    fn default() -> Self {
        Self {
            stream_port: 6590,
            channels: (0..=23).collect(),
            oversampling: 20,
            auto_start: true,
            buffer_size: Some(10_000),
        }
    }
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
    pub fn with_data_collection(
        mut self,
        pre_duration: Duration,
        post_duration: Duration,
    ) -> Self {
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
            ExecutionResult::Chain(mut results) if results.len() == 1 => {
                Ok(results.pop().unwrap())
            }
            _ => Err(NanonisError::Protocol(
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
            _ => Err(NanonisError::Protocol(
                "Expected chain results".to_string(),
            )),
        }
    }

    /// Extract experiment data or error if not experiment
    pub fn into_experiment_data(
        self,
    ) -> Result<crate::types::ExperimentData, NanonisError> {
        match self {
            ExecutionResult::ExperimentData(data) => Ok(data),
            _ => Err(NanonisError::Protocol(
                "Expected experiment data".to_string(),
            )),
        }
    }

    /// Extract chain experiment data or error if not chain experiment
    pub fn into_chain_experiment_data(
        self,
    ) -> Result<crate::types::ChainExperimentData, NanonisError> {
        match self {
            ExecutionResult::ChainExperimentData(data) => Ok(data),
            _ => Err(NanonisError::Protocol(
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
    pub fn with_data_collection(
        mut self,
        pre_duration: Duration,
        post_duration: Duration,
    ) -> Self {
        self.config = self
            .config
            .with_data_collection(pre_duration, post_duration);
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
                let result =
                    self.driver.run_with_config(self.request, self.config)?;
                result.into_single()
            }
            ActionRequest::Chain(_) => Err(NanonisError::Protocol(
                "Use .execute() for chains, .go() is only for single actions"
                    .to_string(),
            )),
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
    tcp_reader_config: Option<TCPReaderConfig>,
    action_logger_config: Option<(std::path::PathBuf, usize, bool)>, // (file_path, buffer_size, final_format_json)
    custom_tcp_mapping: Option<Vec<(u8, u8)>>, // Custom Nanonis to TCP channel mapping
    shutdown_flag: Option<Arc<AtomicBool>>,    // Graceful shutdown support
}

impl ActionDriverBuilder {
    /// Create a new builder with required connection parameters
    pub fn new(addr: &str, port: u16) -> Self {
        Self {
            addr: addr.to_string(),
            port,
            connection_timeout: None,
            initial_storage: HashMap::new(),
            tcp_reader_config: None,
            action_logger_config: None,
            custom_tcp_mapping: None,
            shutdown_flag: None,
        }
    }

    /// Set connection timeout for the underlying NanonisClient
    pub fn with_connection_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = Some(timeout);
        self
    }

    /// Initialize with pre-stored values
    pub fn with_initial_storage(
        mut self,
        storage: HashMap<String, ActionResult>,
    ) -> Self {
        self.initial_storage = storage;
        self
    }

    /// Add a single pre-stored value
    pub fn with_stored_value(
        mut self,
        key: String,
        value: ActionResult,
    ) -> Self {
        self.initial_storage.insert(key, value);
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
    ///     .with_tcp_reader(TCPReaderConfig {
    ///         stream_port: 6590,
    ///         channels: vec![0, 8],
    ///         oversampling: 100,
    ///         auto_start: true,
    ///         buffer_size: Some(10_000),
    ///     })
    ///     .build()?;
    /// // Buffering is now active and ready for immediate data queries
    /// ```
    pub fn with_tcp_reader(mut self, config: TCPReaderConfig) -> Self {
        if config.buffer_size.is_none() {
            log::warn!(
                "TCPLoggerConfig buffer_size is None - buffering disabled"
            );
        }
        self.tcp_reader_config = Some(config);
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
    pub fn with_action_logging(
        mut self,
        file_path: impl Into<std::path::PathBuf>,
        buffer_size: usize,
        final_format_json: bool,
    ) -> Self {
        self.action_logger_config =
            Some((file_path.into(), buffer_size, final_format_json));
        self
    }

    /// Provide custom Nanonis to TCP channel mapping
    ///
    /// Override the default hardcoded mappings with your own. This is useful when
    /// your Nanonis configuration has different signal indices.
    ///
    /// # Arguments
    /// * `mapping` - Array of (nanonis_index, tcp_channel) tuples
    ///
    /// # Example
    /// ```rust,ignore
    /// let custom_map = [
    ///     (76, 18),  // Frequency shift
    ///     (0, 0),    // Current
    ///     (24, 8),   // Bias
    /// ];
    ///
    /// let driver = ActionDriver::builder("127.0.0.1", 6501)
    ///     .with_custom_tcp_mapping(&custom_map)
    ///     .build()?;
    /// ```
    pub fn with_custom_tcp_mapping(mut self, mapping: &[(u8, u8)]) -> Self {
        self.custom_tcp_mapping = Some(mapping.to_vec());
        self
    }

    /// Set shutdown flag for graceful termination of long-running operations
    ///
    /// When set, operations like stability checks will periodically check this flag
    /// and return early with `NanonisError::Protocol("Shutdown requested".to_string())` if it becomes true.
    pub fn with_shutdown_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.shutdown_flag = Some(flag);
        self
    }

    /// Build the ActionDriver with configured parameters and optional automatic buffering
    pub fn build(self) -> Result<ActionDriver, NanonisError> {
        let mut client = {
            let mut builder =
                NanonisClient::builder().address(&self.addr).port(self.port);

            if let Some(timeout) = self.connection_timeout {
                builder = builder.connect_timeout(timeout);
            }

            builder.build()?
        };

        let tcp_reader = if let Some(ref config) = self.tcp_reader_config {
            if let Some(buffer_size) = config.buffer_size {
                // 1. Configure TCP logger settings first
                client.tcplog_chs_set(config.channels.clone())?;
                client.tcplog_oversampl_set(config.oversampling)?;

                // 2. Connect TCP stream BEFORE starting logger (critical sequence!)
                let reader =
                    crate::buffered_tcp_reader::BufferedTCPReader::new(
                        "127.0.0.1",
                        config.stream_port,
                        buffer_size,
                        config.channels.len() as u32,
                        config.oversampling as f32,
                    )?;
                log::debug!(
                    "TCP stream connected, buffer capacity: {} frames",
                    buffer_size
                );

                // 3. NOW start TCP logger (data flows to connected reader)
                if config.auto_start {
                    // Reset TCP logger state first to ensure clean start
                    log::debug!("Stopping TCP logger to ensure clean state");
                    let _ = client.tcplog_stop(); // Ignore errors - might not be running
                    std::thread::sleep(std::time::Duration::from_millis(200)); // Give it time to stop

                    // Now start TCP logger
                    client.tcplog_start()?;
                    log::debug!("TCP logger started, data collection active");
                }

                Some(reader)
            } else {
                None
            }
        } else {
            None
        };

        // Create action logger if configured
        let action_logger =
            if let Some((file_path, buffer_size, final_format_json)) =
                self.action_logger_config
            {
                Some(crate::logger::Logger::new(
                    file_path,
                    buffer_size,
                    final_format_json,
                ))
            } else {
                None
            };

        // Auto-initialize signal registry with custom or hardcoded mapping
        let signal_names = client.signal_names_get()?;
        let signal_registry =
            if let Some(ref custom_map) = self.custom_tcp_mapping {
                log::debug!(
                    "Using custom TCP channel mapping with {} entries",
                    custom_map.len()
                );
                SignalRegistry::builder()
                    .with_standard_map()
                    .add_tcp_map(custom_map)
                    .from_signal_names(&signal_names)
                    .create_aliases()
                    .build()
            } else {
                SignalRegistry::with_hardcoded_tcp_mapping(&signal_names)
            };

        Ok(ActionDriver {
            client,
            stored_values: self.initial_storage,
            tcp_reader_config: self.tcp_reader_config,
            tcp_reader,
            action_logger,
            action_logging_enabled: true, // Default to enabled if logger is configured
            signal_registry,
            recent_stable_signals: std::collections::VecDeque::new(),
            shutdown_flag: self.shutdown_flag,
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
    tcp_reader_config: Option<TCPReaderConfig>,
    /// Buffered TCP reader for always-buffer mode (automatically started if configured)
    tcp_reader: Option<crate::buffered_tcp_reader::BufferedTCPReader>,
    /// Action logger for execution tracking
    action_logger:
        Option<crate::logger::Logger<crate::actions::ActionLogEntry>>,
    /// Enable/disable action logging at runtime
    action_logging_enabled: bool,
    /// Signal registry for name-based lookup and TCP mapping
    signal_registry: SignalRegistry,
    /// Recent ReadStableSignal results for correlation with CheckTipState
    recent_stable_signals: std::collections::VecDeque<(
        crate::actions::StableSignal,
        std::time::Instant,
    )>,
    /// Shutdown flag for graceful termination of long-running operations
    shutdown_flag: Option<Arc<AtomicBool>>,
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
    pub fn with_nanonis_client(mut client: NanonisClient) -> Self {
        // Initialize signal registry even for this convenience method
        let signal_names = client.signal_names_get().unwrap_or_default();
        let signal_registry =
            SignalRegistry::with_hardcoded_tcp_mapping(&signal_names);

        Self {
            client,
            stored_values: HashMap::new(),
            tcp_reader_config: None,
            tcp_reader: None,
            action_logger: None,
            action_logging_enabled: false,
            signal_registry,
            recent_stable_signals: std::collections::VecDeque::new(),
            shutdown_flag: None,
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

    /// Set shutdown flag for graceful termination of long-running operations
    pub fn set_shutdown_flag(&mut self, flag: Arc<AtomicBool>) {
        self.shutdown_flag = Some(flag);
    }

    /// Check if shutdown has been requested
    fn is_shutdown_requested(&self) -> bool {
        self.shutdown_flag
            .as_ref()
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Execute an auto-approach operation.
    ///
    /// If `wait_until_finished` is true, blocks until approach completes or timeout.
    /// If false, starts the approach and returns immediately.
    pub fn auto_approach(
        &mut self,
        wait_until_finished: bool,
        timeout: Duration,
    ) -> Result<(), NanonisError> {
        // Check if already running
        match self.client.auto_approach_on_off_get() {
            Ok(true) => {
                log::warn!("Auto-approach already running");
                return Ok(());
            }
            Ok(false) => {
                log::debug!("Auto-approach is idle, proceeding to start");
            }
            Err(_) => {
                log::warn!(
                    "Auto-approach status unknown, attempting to proceed"
                );
            }
        }

        // Open auto-approach module
        match self.client.auto_approach_open() {
            Ok(_) => log::debug!("Opened the auto-approach module"),
            Err(_) => {
                log::debug!("Failed to open auto-approach module, already open")
            }
        }

        // Wait for module initialization
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Start auto-approach
        if let Err(e) = self.client.auto_approach_on_off_set(true) {
            log::error!("Failed to start auto-approach: {}", e);
            return Err(NanonisError::Protocol(format!(
                "Failed to start auto-approach: {}",
                e
            )));
        }

        if !wait_until_finished {
            log::debug!("Auto-approach started, not waiting for completion");
            return Ok(());
        }

        // Wait for completion with timeout
        log::debug!("Waiting for auto-approach to complete...");
        let poll_interval = std::time::Duration::from_millis(100);

        match poll_until(
            || {
                self.client
                    .auto_approach_on_off_get()
                    .map(|running| !running)
            },
            timeout,
            poll_interval,
        ) {
            Ok(()) => {
                log::debug!("Auto-approach completed successfully");
                Ok(())
            }
            Err(PollError::Timeout) => {
                log::warn!("Auto-approach timed out after {:?}", timeout);
                let _ = self.client.auto_approach_on_off_set(false);
                Err(NanonisError::Protocol(
                    "Auto-approach timed out".to_string(),
                ))
            }
            Err(PollError::ConditionError(e)) => {
                log::error!("Error checking auto-approach status: {}", e);
                Err(NanonisError::Protocol(format!(
                    "Status check error: {}",
                    e
                )))
            }
        }
    }

    /// Center the frequency shift using the PLL auto-center function.
    pub fn center_freq_shift(&mut self) -> Result<(), NanonisError> {
        let modulator_index = 1;
        log::debug!("Centering frequency shift");
        self.client.pll_freq_shift_auto_center(modulator_index)
    }

    /// Get TCP Logger configuration if set
    pub fn tcp_reader_config(&self) -> Option<&TCPReaderConfig> {
        self.tcp_reader_config.as_ref()
    }

    /// Check if TCP logger is configured and available
    pub fn has_tcp_reader(&self) -> bool {
        self.tcp_reader.is_some()
    }

    pub fn tcp_reader_mut(&mut self) -> Option<&mut BufferedTCPReader> {
        self.tcp_reader.as_mut()
    }

    /// Clear the TCP reader buffer
    ///
    /// This removes all buffered data, which is useful to discard stale values
    /// before starting a new measurement or tip preparation sequence.
    pub fn clear_tcp_buffer(&self) {
        if let Some(ref tcp_reader) = self.tcp_reader {
            tcp_reader.clear_buffer();
            debug!("TCP reader buffer cleared");
        } else {
            warn!("No TCP reader available to clear");
        }
    }

    /// Get reference to the signal registry
    pub fn signal_registry(&self) -> &SignalRegistry {
        &self.signal_registry
    }

    /// Calculate number of data points needed for a target duration
    ///
    /// Based on the TCP reader configuration (oversampling), calculates how many
    /// samples are needed to cover the specified duration.
    ///
    /// # Arguments
    /// * `target_duration` - Desired time window for data collection
    ///
    /// # Returns
    /// Number of samples, or None if TCP reader is not configured
    ///
    /// # Example
    /// For 500ms at 2000 Hz effective rate: returns 1000 samples
    fn calculate_samples_for_duration(
        &self,
        target_duration: Duration,
    ) -> Option<usize> {
        if let Some(ref config) = self.tcp_reader_config {
            // Effective sample rate = base_rate / oversampling
            // For oversampling=1 at 2kHz base: 2000 samples/sec
            // For 500ms: 2000 * 0.5 = 1000 samples
            let base_rate = 2000.0; // Typical Nanonis base rate in Hz
            let effective_rate = base_rate / config.oversampling as f64;
            let samples = (effective_rate * target_duration.as_secs_f64())
                .ceil() as usize;
            log::debug!(
                "Calculated {} samples for {:.0}ms (base: {}Hz, oversampling: {}, effective: {:.1}Hz)",
                samples,
                target_duration.as_millis(),
                base_rate,
                config.oversampling,
                effective_rate
            );
            Some(samples.max(50)) // Minimum 50 samples
        } else {
            None
        }
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
    pub fn run_with_config(
        &mut self,
        request: ActionRequest,
        config: ExecutionConfig,
    ) -> Result<ExecutionResult, NanonisError> {
        match (&request, &config.data_collection) {
            // Single action with data collection
            (
                ActionRequest::Single(action),
                Some((pre_duration, post_duration)),
            ) => {
                let experiment_data = self.execute_with_data_collection(
                    action.clone(),
                    *pre_duration,
                    *post_duration,
                )?;
                Ok(ExecutionResult::ExperimentData(experiment_data))
            }

            // Chain with data collection
            (
                ActionRequest::Chain(actions),
                Some((pre_duration, post_duration)),
            ) => {
                let chain_experiment_data = self
                    .execute_chain_with_data_collection(
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
                        let previous_state =
                            self.set_action_logging_enabled(false);
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
                let results =
                    match (&config.chain_behavior, &config.logging_behavior) {
                        (ChainBehavior::Complete, LoggingBehavior::Normal) => {
                            self.execute_chain(actions.clone())?
                        }
                        (
                            ChainBehavior::Complete,
                            LoggingBehavior::Deferred,
                        ) => self.execute_chain_deferred(actions.clone())?,
                        (
                            ChainBehavior::Complete,
                            LoggingBehavior::Disabled,
                        ) => {
                            let previous_state =
                                self.set_action_logging_enabled(false);
                            let result = self.execute_chain(actions.clone());
                            self.set_action_logging_enabled(previous_state);
                            result?
                        }
                        (ChainBehavior::FinalOnly, _) => {
                            let results = match config.logging_behavior {
                                LoggingBehavior::Deferred => self
                                    .execute_chain_deferred(actions.clone())?,
                                LoggingBehavior::Disabled => {
                                    let previous_state =
                                        self.set_action_logging_enabled(false);
                                    let result =
                                        self.execute_chain(actions.clone());
                                    self.set_action_logging_enabled(
                                        previous_state,
                                    );
                                    result?
                                }
                                _ => self.execute_chain(actions.clone())?,
                            };
                            vec![results
                                .into_iter()
                                .last()
                                .unwrap_or(ActionResult::None)]
                        }
                        (ChainBehavior::Partial, _) => {
                            match self.execute_chain_partial(actions.clone()) {
                                Ok(results) => results,
                                Err((partial_results, error)) => {
                                    return Ok(ExecutionResult::Partial(
                                        partial_results,
                                        error,
                                    ));
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
            return Err(NanonisError::Protocol(
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
        let tcp_config = self.tcp_reader_config.as_ref().unwrap().clone();

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
                metadata: Some(
                    [
                        (
                            "type".to_string(),
                            "experiment_data_collection".to_string(),
                        ),
                        (
                            "pre_duration_ms".to_string(),
                            pre_duration.as_millis().to_string(),
                        ),
                        (
                            "post_duration_ms".to_string(),
                            post_duration.as_millis().to_string(),
                        ),
                        (
                            "signal_frame_count".to_string(),
                            experiment_data.signal_frames.len().to_string(),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            };

            if let Err(log_error) =
                self.action_logger.as_mut().unwrap().add(log_entry)
            {
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
            return Err(NanonisError::Protocol(
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
        let tcp_config = self.tcp_reader_config.as_ref().unwrap().clone();

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
                action: format!(
                    "Chain Data Collection: {} actions",
                    chain_experiment_data.action_results.len()
                ),
                result: ActionLogResult::from_chain_experiment_data(
                    &chain_experiment_data,
                ),
                start_time: chrono::Utc::now(),
                duration_ms: chain_experiment_data.total_duration.as_millis()
                    as u64,
                metadata: Some(
                    [
                        (
                            "type".to_string(),
                            "chain_experiment_data_collection".to_string(),
                        ),
                        (
                            "pre_duration_ms".to_string(),
                            pre_duration.as_millis().to_string(),
                        ),
                        (
                            "post_duration_ms".to_string(),
                            post_duration.as_millis().to_string(),
                        ),
                        (
                            "action_count".to_string(),
                            chain_experiment_data
                                .action_results
                                .len()
                                .to_string(),
                        ),
                        (
                            "signal_frame_count".to_string(),
                            chain_experiment_data
                                .signal_frames
                                .len()
                                .to_string(),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            };

            if let Err(log_error) =
                self.action_logger.as_mut().unwrap().add(log_entry)
            {
                log::warn!(
                    "Failed to log chain experiment data: {}",
                    log_error
                );
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
    pub fn set_tcp_logger_channels(
        &mut self,
        channels: Vec<i32>,
    ) -> Result<(), NanonisError> {
        self.client.tcplog_chs_set(channels)
    }

    /// Set TCP logger oversampling
    pub fn set_tcp_logger_oversampling(
        &mut self,
        oversampling: i32,
    ) -> Result<(), NanonisError> {
        self.client.tcplog_oversampl_set(oversampling)
    }

    /// Get TCP logger status
    pub fn get_tcp_logger_status(
        &mut self,
    ) -> Result<crate::types::TCPLogStatus, NanonisError> {
        self.client.tcplog_status_get()
    }

    /// Execute a single action
    pub fn execute(
        &mut self,
        action: Action,
    ) -> Result<ActionResult, NanonisError> {
        let start_time = chrono::Utc::now();
        let start_instant = std::time::Instant::now();

        let result = self.execute_internal(action.clone());

        let duration = start_instant.elapsed();

        // Log the action execution if logging is enabled
        if self.action_logging_enabled && self.action_logger.is_some() {
            let log_entry = match &result {
                Ok(action_result) => ActionLogEntry::new(
                    &action,
                    action_result,
                    start_time,
                    duration,
                ),
                Err(error) => ActionLogEntry::new_error(
                    &action, error, start_time, duration,
                ),
            };

            if let Err(log_error) =
                self.action_logger.as_mut().unwrap().add(log_entry)
            {
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
            let _experiment_data = self.execute_with_data_collection(
                action,
                pre_duration,
                post_duration,
            )?;
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
            let chain_experiment_data = self
                .execute_chain_with_data_collection(
                    chain.into().into_iter().collect(),
                    pre_duration,
                    post_duration,
                )?;
            // Return the action results from the chain
            Ok(chain_experiment_data.action_results)
        } else {
            // Use normal execution
            self.execute_chain(chain)
        }
    }

    /// Internal execute method without logging (for performance-critical chains)
    fn execute_internal(
        &mut self,
        action: Action,
    ) -> Result<ActionResult, NanonisError> {
        match action {
            // === Signal Operations ===
            Action::ReadSignal {
                signal,
                wait_for_newest,
            } => {
                let value = self.client.signals_vals_get(
                    vec![SignalIndex::new(signal.index).into()],
                    wait_for_newest,
                )?;
                Ok(ActionResult::Value(value[0] as f64))
            }

            Action::ReadSignals {
                signals,
                wait_for_newest,
            } => {
                let indices: Vec<i32> = signals
                    .iter()
                    .map(|s| SignalIndex::new(s.index).into())
                    .collect();
                let values =
                    self.client.signals_vals_get(indices, wait_for_newest)?;
                Ok(ActionResult::Values(
                    values.into_iter().map(|v| v as f64).collect(),
                ))
            }

            Action::ReadSignalNames => {
                let names = self.client.signal_names_get()?;
                Ok(ActionResult::Text(names))
            }

            // === Bias Operations ===
            Action::ReadBias => {
                let bias = self.client.bias_get()?;
                Ok(ActionResult::Value(bias as f64))
            }

            Action::SetBias { voltage } => {
                self.client.bias_set(voltage)?;
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

                self.client.osci1t_ch_set(signal.index as i32)?;

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
                        let osci_data = self
                            .find_stable_oscilloscope_data_with_fallback(
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
                        let (t0, dt, size, data) =
                            self.client.osci1t_data_get(data_mode)?;
                        let osci_data =
                            OsciData::new_stable(t0, dt, size, data);
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
                self.client.motor_start_move(
                    direction,
                    steps,
                    MotorGroup::Group1,
                    blocking,
                )?;
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
                    self.client.motor_start_move(
                        direction,
                        steps,
                        MotorGroup::Group1,
                        blocking,
                    )?;
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
                center_freq_shift,
            } => {
                log::debug!(
                    "Starting auto-approach (wait: {}, timeout: {:?}, center_freq: {})",
                    wait_until_finished,
                    timeout,
                    center_freq_shift
                );

                // Center frequency shift if requested
                if center_freq_shift {
                    // Approach to the surface
                    self.auto_approach(true, timeout)?;

                    // Sleep for 0.2 secs
                    std::thread::sleep(Duration::from_millis(200));

                    // Toggle on the safe tip
                    if let Ok(safetip_state) =
                        self.client_mut().safe_tip_on_off_get()
                    {
                        if !safetip_state {
                            self.client_mut().safe_tip_on_off_set(true)?;
                        }
                    } else {
                        log::warn!(
                            "Failed to read safe tip state, setting true"
                        );
                        self.client_mut().safe_tip_on_off_set(true)?;
                    }

                    self.check_safetip_status("after enabling safe tip")?;

                    // Home 50nm away from the surface
                    self.client_mut().z_ctrl_home()?;

                    self.check_safetip_status("after z_ctrl_home")?;

                    // Sleep for 0.5 secs
                    std::thread::sleep(Duration::from_millis(500));

                    self.check_safetip_status("after 500ms settle")?;

                    // Center the freq shift
                    if let Err(e) = self.center_freq_shift() {
                        log::warn!("Failed to center frequency shift: {}", e);
                        // Continue anyway, this is not critical
                    }

                    self.check_safetip_status("after center_freq_shift")?;

                    // Approach again
                    self.auto_approach(wait_until_finished, timeout)?;

                    self.check_safetip_status("after final auto_approach")?;

                    // Toggle of the safe tip
                    if let Ok(safetip_state) =
                        self.client_mut().safe_tip_on_off_get()
                    {
                        if safetip_state {
                            self.client_mut().safe_tip_on_off_set(false)?;
                        }
                    } else {
                        log::warn!(
                            "Failed to read safe tip state, setting false"
                        );
                        self.client_mut().safe_tip_on_off_set(false)?;
                    }
                } else {
                    self.auto_approach(wait_until_finished, timeout)?;
                }

                Ok(ActionResult::Success)
            }

            Action::Withdraw {
                wait_until_finished,
                timeout,
            } => {
                self.client.z_ctrl_withdraw(wait_until_finished, timeout)?;
                Ok(ActionResult::Success)
            }

            Action::SafeReposition { x_steps, y_steps } => {
                // Safe repositioning with hardcoded defaults
                let displacement =
                    crate::types::MotorDisplacement::new(x_steps, y_steps, -3);
                let withdraw_timeout = Duration::from_secs(5);
                let approach_timeout = Duration::from_secs(10);
                let stabilization_wait = Duration::from_millis(500);

                // Execute the safe repositioning sequence
                // 1. Withdraw
                self.client.z_ctrl_withdraw(true, withdraw_timeout)?;

                // 2. Move motor 3D (using the same logic as MoveMotor3D)
                let movements = displacement.to_motor_movements();
                for (direction, steps) in movements {
                    self.client.motor_start_move(
                        direction,
                        steps,
                        MotorGroup::Group1,
                        true,
                    )?;
                }

                thread::sleep(Duration::from_millis(500));

                // 3. Center frequency and auto approach
                self.run(Action::AutoApproach {
                    wait_until_finished: true,
                    timeout: approach_timeout,
                    center_freq_shift: true,
                })
                .go()?;

                // 4. Wait for stabilization
                thread::sleep(stabilization_wait);

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
                let current_bias =
                    self.client_mut().bias_get().unwrap_or(500e-3);

                let config = TipShaperConfig {
                    switch_off_delay: std::time::Duration::from_millis(10),
                    change_bias: true,
                    bias_v: pulse_height_v,
                    tip_lift_m: 0.0,
                    lift_time_1: pulse_width,
                    bias_lift_v: current_bias,
                    bias_settling_time: std::time::Duration::from_millis(50),
                    lift_height_m: 100e-9,
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
                None => Err(NanonisError::Protocol(format!(
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
                use crate::actions::TCPReaderStatus;
                let status = self.get_tcp_logger_status()?;
                let config = self.tcp_reader_config();

                Ok(ActionResult::TCPReaderStatus(TCPReaderStatus {
                    status,
                    channels: config
                        .map(|c| c.channels.clone())
                        .unwrap_or_default(),
                    oversampling: config.map(|c| c.oversampling).unwrap_or(0),
                }))
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
                use std::collections::HashMap;

                use crate::{
                    actions::{TipCheckMethod, TipState},
                    types::TipShape,
                };

                let (tip_shape, measured_signals, mut metadata) = match method {
                    TipCheckMethod::SignalBounds { signal, bounds } => {
                        // Debug TCP logger status before calling ReadStableSignal
                        if let Some(ref tcp_reader) = self.tcp_reader {
                            let (frame_count, _max_capacity, time_span) =
                                tcp_reader.buffer_stats();
                            log::debug!("CheckTipState: TCP reader available with {} frames, timespan: {}ms", 
                                frame_count, time_span.as_millis());
                        } else {
                            log::warn!(
                                "CheckTipState: No TCP reader available for signal {}",
                                signal.index
                                );
                        }

                        // Use ReadStableSignal instead of single instantaneous read
                        log::debug!(
                            "CheckTipState: Calling ReadStableSignal for signal {}",
                            signal.index
                        );

                        // Calculate samples needed for configured data collection duration
                        let data_points = self
                            .calculate_samples_for_duration(
                                Duration::from_millis(
                                    TIP_STATE_DATA_COLLECTION_DURATION_MS,
                                ),
                            )
                            .unwrap_or(100); // Fallback to 100 if TCP not configured

                        let stable_result = self
                            .run(Action::ReadStableSignal {
                                signal: signal.clone(),
                                data_points: Some(data_points),
                                use_new_data: true, // Get fresh data for tip state checking
                                stability_method: crate::actions::SignalStabilityMethod::Combined {
                                    max_std_dev: TIP_STATE_MAX_STD_DEV,
                                    max_slope: TIP_STATE_MAX_SLOPE,
                                },
                                timeout: Duration::from_secs(TIP_STATE_READ_TIMEOUT_SECS),
                                retry_count: Some(TIP_STATE_READ_RETRY_COUNT),
                            })
                            .execute();

                        let (value, raw_data, read_method) = match stable_result
                        {
                            Ok(exec_result) => match exec_result {
                                ExecutionResult::Single(
                                    ActionResult::StableSignal(stable_signal),
                                ) => {
                                    // Use stable value from ReadStableSignal
                                    log::debug!("CheckTipState: ReadStableSignal succeeded with {} data points", stable_signal.raw_data.len());
                                    (
                                        stable_signal.stable_value,
                                        stable_signal.raw_data,
                                        "stable_signal",
                                    )
                                }
                                ExecutionResult::Single(
                                    ActionResult::Values(values),
                                ) => {
                                    // ReadStableSignal failed but returned raw data, use minimum as fallback
                                    log::warn!("CheckTipState: ReadStableSignal failed but returned {} raw values, using minimum as fallback", values.len());
                                    let raw_data: Vec<f32> = values
                                        .iter()
                                        .map(|&v| v as f32)
                                        .collect();
                                    let min_value = raw_data
                                        .iter()
                                        .cloned()
                                        .fold(f32::INFINITY, f32::min);
                                    (min_value, raw_data, "fallback_minimum")
                                }
                                _ => {
                                    // Unexpected result type, fallback to single read
                                    log::warn!("CheckTipState: ReadStableSignal returned unexpected result type, falling back to single read");
                                    let single_value = self
                                        .client
                                        .signal_val_get(signal.index, true)?;
                                    (
                                        single_value,
                                        vec![single_value],
                                        "single_read_fallback",
                                    )
                                }
                            },
                            Err(e) => {
                                // Complete fallback to single read
                                log::warn!("CheckTipState: ReadStableSignal failed with error: {}, falling back to single read", e);
                                let single_value = self
                                    .client
                                    .signal_val_get(signal.index, true)?;
                                (
                                    single_value,
                                    vec![single_value],
                                    "single_read_fallback",
                                )
                            }
                        };

                        let mut measured = HashMap::new();
                        measured.insert(SignalIndex::new(signal.index), value);

                        let shape = if value >= bounds.0 && value <= bounds.1 {
                            TipShape::Sharp
                        } else {
                            TipShape::Blunt
                        };

                        // Populate metadata with analysis context and dataset
                        let bounds_center = (bounds.0 + bounds.1) / 2.0;
                        let bounds_width = (bounds.1 - bounds.0).abs();
                        let distance_from_center =
                            (value - bounds_center).abs();
                        let relative_distance = if bounds_width > 0.0 {
                            distance_from_center / (bounds_width / 2.0)
                        } else {
                            0.0
                        };
                        let mut metadata = HashMap::new();
                        metadata.insert(
                            "method".to_string(),
                            "signal_bounds".to_string(),
                        );
                        metadata.insert(
                            "signal_index".to_string(),
                            signal.index.to_string(),
                        );
                        metadata.insert(
                            "measured_value".to_string(),
                            format!("{:.6e}", value),
                        );
                        metadata.insert(
                            "bounds_lower".to_string(),
                            format!("{:.6e}", bounds.0),
                        );
                        metadata.insert(
                            "bounds_upper".to_string(),
                            format!("{:.6e}", bounds.1),
                        );
                        metadata.insert(
                            "bounds_center".to_string(),
                            format!("{:.6e}", bounds_center),
                        );
                        metadata.insert(
                            "bounds_width".to_string(),
                            format!("{:.6e}", bounds_width),
                        );
                        metadata.insert(
                            "distance_from_center".to_string(),
                            format!("{:.6e}", distance_from_center),
                        );
                        metadata.insert(
                            "relative_distance".to_string(),
                            format!("{:.3}", relative_distance),
                        );
                        metadata.insert(
                            "within_bounds".to_string(),
                            (shape == TipShape::Sharp).to_string(),
                        );
                        metadata.insert(
                            "read_method".to_string(),
                            read_method.to_string(),
                        );
                        metadata.insert(
                            "dataset_size".to_string(),
                            raw_data.len().to_string(),
                        );

                        // Store raw dataset for debugging stability measures
                        let raw_data_summary = if raw_data.len() <= 10 {
                            raw_data
                                .iter()
                                .map(|x| format!("{:.3e}", x))
                                .collect::<Vec<_>>()
                                .join(",")
                        } else {
                            let first_5: String = raw_data
                                .iter()
                                .take(5)
                                .map(|x| format!("{:.3e}", x))
                                .collect::<Vec<_>>()
                                .join(",");
                            let last_5: String = raw_data
                                .iter()
                                .rev()
                                .take(5)
                                .rev()
                                .map(|x| format!("{:.3e}", x))
                                .collect::<Vec<_>>()
                                .join(",");
                            format!("{},...,{}", first_5, last_5)
                        };
                        metadata.insert(
                            "raw_dataset_summary".to_string(),
                            format!("[{}]", raw_data_summary),
                        );

                        if shape == TipShape::Blunt {
                            let margin_violation = if value < bounds.0 {
                                bounds.0 - value
                            } else {
                                value - bounds.1
                            };
                            metadata.insert(
                                "margin_violation".to_string(),
                                format!("{:.6e}", margin_violation),
                            );
                            metadata.insert(
                                "violation_direction".to_string(),
                                if value < bounds.0 {
                                    "below_lower_bound".to_string()
                                } else {
                                    "above_upper_bound".to_string()
                                },
                            );
                        }

                        (shape, measured, metadata)
                    }

                    TipCheckMethod::MultiSignalBounds { ref signals } => {
                        let mut measured = HashMap::new();
                        let mut violations = Vec::new();
                        let mut all_good = true;
                        let mut all_datasets = Vec::new();
                        let mut read_methods = Vec::new();

                        // Calculate samples needed for configured data collection duration
                        let data_points = self
                            .calculate_samples_for_duration(
                                Duration::from_millis(
                                    TIP_STATE_DATA_COLLECTION_DURATION_MS,
                                ),
                            )
                            .unwrap_or(100); // Fallback to 100 if TCP not configured

                        // Read each signal using ReadStableSignal
                        for (signal, bounds) in signals.iter() {
                            let stable_result = self
                                .run(Action::ReadStableSignal {
                                    signal: signal.clone(),
                                    data_points: Some(data_points),
                                    use_new_data: true, // Get fresh data for tip state checking
                                    stability_method:
                                        crate::actions::SignalStabilityMethod::Combined {
                                            max_std_dev: TIP_STATE_MAX_STD_DEV,
                                            max_slope: TIP_STATE_MAX_SLOPE,
                                        },
                                    timeout: Duration::from_secs(TIP_STATE_READ_TIMEOUT_SECS),
                                    retry_count: Some(TIP_STATE_READ_RETRY_COUNT),
                                })
                                .execute();

                            let (value, raw_data, read_method) =
                                match stable_result {
                                    Ok(exec_result) => match exec_result {
                                        ExecutionResult::Single(
                                            ActionResult::StableSignal(
                                                stable_signal,
                                            ),
                                        ) => (
                                            stable_signal.stable_value,
                                            stable_signal.raw_data,
                                            "stable_signal",
                                        ),
                                        ExecutionResult::Single(
                                            ActionResult::Values(values),
                                        ) => {
                                            let raw_data: Vec<f32> = values
                                                .iter()
                                                .map(|&v| v as f32)
                                                .collect();
                                            let min_value = raw_data
                                                .iter()
                                                .cloned()
                                                .fold(f32::INFINITY, f32::min);
                                            (
                                                min_value,
                                                raw_data,
                                                "fallback_minimum",
                                            )
                                        }
                                        _ => {
                                            let single_value =
                                                self.client.signal_val_get(
                                                    signal.index,
                                                    true,
                                                )?;
                                            (
                                                single_value,
                                                vec![single_value],
                                                "single_read_fallback",
                                            )
                                        }
                                    },
                                    Err(_) => {
                                        let single_value =
                                            self.client.signal_val_get(
                                                signal.index,
                                                true,
                                            )?;
                                        (
                                            single_value,
                                            vec![single_value],
                                            "single_read_fallback",
                                        )
                                    }
                                };

                            measured
                                .insert(SignalIndex::new(signal.index), value);
                            all_datasets.push(raw_data);
                            read_methods.push(read_method);

                            let in_bounds =
                                value >= bounds.0 && value <= bounds.1;
                            if !in_bounds {
                                violations.push((
                                    signal.clone(),
                                    value,
                                    *bounds,
                                ));
                                all_good = false;
                            }
                        }

                        let shape = if all_good {
                            TipShape::Sharp
                        } else {
                            TipShape::Blunt
                        };

                        // Populate metadata with multi-signal analysis and datasets
                        let mut metadata = HashMap::new();
                        metadata.insert(
                            "method".to_string(),
                            "multi_signal_bounds".to_string(),
                        );
                        metadata.insert(
                            "signal_count".to_string(),
                            signals.len().to_string(),
                        );
                        metadata.insert(
                            "signals_in_bounds".to_string(),
                            (signals.len() - violations.len()).to_string(),
                        );
                        metadata.insert(
                            "violation_count".to_string(),
                            violations.len().to_string(),
                        );
                        metadata.insert(
                            "overall_pass".to_string(),
                            all_good.to_string(),
                        );

                        // Add individual signal details with datasets
                        for (i, ((signal, bounds), dataset)) in
                            signals.iter().zip(all_datasets.iter()).enumerate()
                        {
                            let prefix = format!("signal_{}", i);
                            let value =
                                measured[&SignalIndex::new(signal.index)];

                            metadata.insert(
                                format!("{}_index", prefix),
                                signal.index.to_string(),
                            );
                            metadata.insert(
                                format!("{}_value", prefix),
                                format!("{:.6e}", value),
                            );
                            metadata.insert(
                                format!("{}_bounds", prefix),
                                format!("[{:.3e}, {:.3e}]", bounds.0, bounds.1),
                            );
                            metadata.insert(
                                format!("{}_in_bounds", prefix),
                                (value >= bounds.0 && value <= bounds.1)
                                    .to_string(),
                            );
                            metadata.insert(
                                format!("{}_read_method", prefix),
                                read_methods[i].to_string(),
                            );
                            metadata.insert(
                                format!("{}_dataset_size", prefix),
                                dataset.len().to_string(),
                            );

                            // Store dataset summary for debugging
                            let dataset_summary = if dataset.len() <= 10 {
                                dataset
                                    .iter()
                                    .map(|x| format!("{:.3e}", x))
                                    .collect::<Vec<_>>()
                                    .join(",")
                            } else {
                                let first_3: String = dataset
                                    .iter()
                                    .take(3)
                                    .map(|x| format!("{:.3e}", x))
                                    .collect::<Vec<_>>()
                                    .join(",");
                                let last_3: String = dataset
                                    .iter()
                                    .rev()
                                    .take(3)
                                    .rev()
                                    .map(|x| format!("{:.3e}", x))
                                    .collect::<Vec<_>>()
                                    .join(",");
                                format!("{},...,{}", first_3, last_3)
                            };
                            metadata.insert(
                                format!("{}_dataset_summary", prefix),
                                format!("[{}]", dataset_summary),
                            );
                        }

                        (shape, measured, metadata)
                    }
                };

                // Add TCP buffer context and recent signal trends if available
                if let Some(ref tcp_reader) = self.tcp_reader {
                    let (frame_count, _max_capacity, time_span) =
                        tcp_reader.buffer_stats();
                    metadata.insert(
                        "tcp_buffer_frames".to_string(),
                        frame_count.to_string(),
                    );
                    metadata.insert(
                        "tcp_buffer_utilization".to_string(),
                        format!("{:.2}", tcp_reader.buffer_utilization()),
                    );
                    metadata.insert(
                        "tcp_data_timespan_ms".to_string(),
                        time_span.as_millis().to_string(),
                    );
                    metadata.insert(
                        "tcp_uptime_ms".to_string(),
                        tcp_reader.uptime().as_millis().to_string(),
                    );

                    // Add recent signal trend analysis for correlation with stable signal data
                    for signal_idx in measured_signals.keys() {
                        if tcp_reader.frame_count() >= 20 {
                            // Need minimum data for trend analysis
                            let recent_frames =
                                tcp_reader.get_recent_frames(50); // Last 50 data points

                            // Extract signal values for this specific signal from recent TCP data
                            let signal_values: Vec<f32> = recent_frames
                                .iter()
                                .filter_map(|frame| {
                                    // Find the signal in the frame data (assuming signal index maps to data array position)
                                    let idx = signal_idx.get() as usize;
                                    if idx < frame.signal_frame.data.len() {
                                        Some(frame.signal_frame.data[idx])
                                    } else {
                                        None
                                    }
                                })
                                .collect();

                            if signal_values.len() >= 10 {
                                // Minimum for meaningful statistics
                                let mean = signal_values.iter().sum::<f32>()
                                    / signal_values.len() as f32;
                                let variance = signal_values
                                    .iter()
                                    .map(|x| (x - mean).powi(2))
                                    .sum::<f32>()
                                    / signal_values.len() as f32;
                                let std_dev = variance.sqrt();
                                let relative_std = if mean.abs() > 1e-15 {
                                    (std_dev / mean.abs()) * 100.0
                                } else {
                                    0.0
                                };

                                // Calculate trend (simple linear regression slope)
                                let x_values: Vec<f32> = (0..signal_values
                                    .len())
                                    .map(|i| i as f32)
                                    .collect();
                                let x_mean = x_values.iter().sum::<f32>()
                                    / x_values.len() as f32;
                                let y_mean = mean;

                                let numerator: f32 = x_values
                                    .iter()
                                    .zip(signal_values.iter())
                                    .map(|(x, y)| (x - x_mean) * (y - y_mean))
                                    .sum();
                                let denominator: f32 = x_values
                                    .iter()
                                    .map(|x| (x - x_mean).powi(2))
                                    .sum();

                                let trend_slope = if denominator.abs() > 1e-15 {
                                    numerator / denominator
                                } else {
                                    0.0
                                };

                                let signal_prefix =
                                    format!("tcp_signal_{}", signal_idx.get());
                                metadata.insert(
                                    format!("{}_recent_samples", signal_prefix),
                                    signal_values.len().to_string(),
                                );
                                metadata.insert(
                                    format!("{}_recent_mean", signal_prefix),
                                    format!("{:.6e}", mean),
                                );
                                metadata.insert(
                                    format!("{}_recent_std", signal_prefix),
                                    format!("{:.6e}", std_dev),
                                );
                                metadata.insert(
                                    format!(
                                        "{}_recent_relative_std_pct",
                                        signal_prefix
                                    ),
                                    format!("{:.3}", relative_std),
                                );
                                metadata.insert(
                                    format!("{}_trend_slope", signal_prefix),
                                    format!("{:.6e}", trend_slope),
                                );
                                metadata.insert(
                                    format!(
                                        "{}_current_vs_recent_mean",
                                        signal_prefix
                                    ),
                                    format!(
                                        "{:.6e}",
                                        measured_signals[signal_idx] - mean
                                    ),
                                );

                                // Classify signal stability based on recent data
                                let is_stable_signal = relative_std < 5.0
                                    && trend_slope.abs() < (std_dev * 0.1);
                                metadata.insert(
                                    format!("{}_appears_stable", signal_prefix),
                                    is_stable_signal.to_string(),
                                );

                                // Check if current measurement is within recent range
                                let min_recent = signal_values
                                    .iter()
                                    .cloned()
                                    .fold(f32::INFINITY, f32::min);
                                let max_recent = signal_values
                                    .iter()
                                    .cloned()
                                    .fold(f32::NEG_INFINITY, f32::max);
                                let current_in_recent_range =
                                    measured_signals[signal_idx] >= min_recent
                                        && measured_signals[signal_idx]
                                            <= max_recent;
                                metadata.insert(
                                    format!(
                                        "{}_current_in_recent_range",
                                        signal_prefix
                                    ),
                                    current_in_recent_range.to_string(),
                                );
                                metadata.insert(
                                    format!("{}_recent_range", signal_prefix),
                                    format!(
                                        "[{:.6e}, {:.6e}]",
                                        min_recent, max_recent
                                    ),
                                );
                            }
                        }
                    }
                }

                // Add recent ReadStableSignal data for correlation and debugging
                let now = std::time::Instant::now();
                let recent_signals: Vec<_> = self
                    .recent_stable_signals
                    .iter()
                    .filter(|(_, timestamp)| {
                        now.duration_since(*timestamp)
                            < std::time::Duration::from_secs(300)
                    }) // Last 5 minutes
                    .collect();

                if !recent_signals.is_empty() {
                    metadata.insert(
                        "recent_stable_signals_count".to_string(),
                        recent_signals.len().to_string(),
                    );

                    // Add details of the most recent ReadStableSignal for debugging
                    if let Some((most_recent_signal, timestamp)) =
                        recent_signals.last()
                    {
                        let age_ms = now.duration_since(*timestamp).as_millis();
                        metadata.insert(
                            "most_recent_stable_signal_age_ms".to_string(),
                            age_ms.to_string(),
                        );
                        metadata.insert(
                            "most_recent_stable_value".to_string(),
                            format!("{:.6e}", most_recent_signal.stable_value),
                        );
                        metadata.insert(
                            "most_recent_data_points".to_string(),
                            most_recent_signal.data_points_used.to_string(),
                        );
                        metadata.insert(
                            "most_recent_analysis_duration_ms".to_string(),
                            most_recent_signal
                                .analysis_duration
                                .as_millis()
                                .to_string(),
                        );

                        // Include raw data summary for debugging (first 5, last 5 values to avoid huge logs)
                        let raw_data = &most_recent_signal.raw_data;
                        let raw_data_summary = if raw_data.len() <= 10 {
                            // Small dataset, include all
                            raw_data
                                .iter()
                                .map(|x| format!("{:.3e}", x))
                                .collect::<Vec<_>>()
                                .join(",")
                        } else {
                            // Large dataset, show first 5 and last 5
                            let first_5: String = raw_data
                                .iter()
                                .take(5)
                                .map(|x| format!("{:.3e}", x))
                                .collect::<Vec<_>>()
                                .join(",");
                            let last_5: String = raw_data
                                .iter()
                                .rev()
                                .take(5)
                                .rev()
                                .map(|x| format!("{:.3e}", x))
                                .collect::<Vec<_>>()
                                .join(",");
                            format!("{},...,{}", first_5, last_5)
                        };
                        metadata.insert(
                            "most_recent_raw_data_summary".to_string(),
                            format!("[{}]", raw_data_summary),
                        );
                        metadata.insert(
                            "most_recent_raw_data_full_count".to_string(),
                            raw_data.len().to_string(),
                        );

                        // Include stability metrics
                        for (metric_name, metric_value) in
                            &most_recent_signal.stability_metrics
                        {
                            metadata.insert(
                                format!("most_recent_metric_{}", metric_name),
                                format!("{:.6e}", metric_value),
                            );
                        }
                    }
                }

                // Add execution timestamp
                metadata.insert(
                    "execution_timestamp".to_string(),
                    chrono::Utc::now().to_rfc3339(),
                );

                // Log a concise summary; full details at debug level
                let signal_values_str = measured_signals
                    .iter()
                    .map(|(signal_idx, value)| {
                        format!("signal_{}={:.3}", signal_idx.get(), value)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                log::info!(
                    "CheckTipState: shape={:?}, signals=[{}]",
                    tip_shape,
                    signal_values_str
                );

                log::debug!("CheckTipState detail: read_method={}, dataset_size={}, recent_stable_count={}",
                    metadata.get("read_method").map(|s| s.as_str()).unwrap_or("unknown"),
                    metadata.get("dataset_size").map(|s| s.as_str()).unwrap_or("unknown"),
                    recent_signals.len());

                Ok(ActionResult::TipState(TipState {
                    shape: tip_shape,
                    measured_signals,
                    metadata,
                }))
            }

            Action::CheckTipStability {
                method,
                max_duration: _,
            } => {
                use std::collections::HashMap;

                use crate::actions::{StabilityResult, TipStabilityMethod};

                let start_time = std::time::Instant::now();
                let mut metrics = HashMap::new();
                let mut recommendations = Vec::new();

                let (is_stable, measured_values) = match method {
                    TipStabilityMethod::ExtendedMonitoring {
                        signal: _,
                        duration: _,
                        sampling_interval: _,
                        stability_threshold: _,
                    } => {
                        todo!("ExtendedMonitoring not yet implemented");
                    }

                    TipStabilityMethod::BiasSweepResponse {
                        ref signal,
                        bias_range,
                        bias_steps,
                        step_duration,
                        allowed_signal_change,
                    } => {
                        log::info!(
                            "Performing simple bias sweep stability test: {:.2}V to {:.2}V",
                            bias_range.0,
                            bias_range.1
                        );

                        // 1. Get signal channel index for TCP reader
                        let tcp_channel = signal.tcp_channel.ok_or_else(|| {
                            NanonisError::Protocol(format!(
                                "Signal {} (Nanonis index) has no TCP channel mapping",
                                signal.index
                            ))
                        })?;

                        // 2. Save and configure scan properties
                        log::info!("Reading current scan properties...");
                        let original_props = self.client.scan_props_get()?;
                        log::info!(
                            "Original scan props: continuous={}, bouncy={}",
                            original_props.continuous_scan,
                            original_props.bouncy_scan
                        );

                        // Configure scan for stability check
                        log::info!(
                            "Configuring scan: continuous=true, bouncy=true"
                        );
                        let scan_props =
                            nanonis_rs::scan::ScanPropsBuilder::new()
                                .continuous_scan(true)
                                .bouncy_scan(true);
                        self.client.scan_props_set(scan_props)?;
                        log::info!("Scan properties configured");

                        // 3. Get initial bias for restoration
                        let initial_bias = self.client.bias_get()?;
                        log::info!(
                            "Initial bias: {:.3} V (will restore after sweep)",
                            initial_bias
                        );

                        // 4. Read baseline signal value once before starting
                        let baseline_value = {
                            let tcp_reader =
                                self.tcp_reader_mut().ok_or_else(|| {
                                    NanonisError::Protocol(
                                        "TCP reader not available".to_string(),
                                    )
                                })?;

                            let recent_frames = tcp_reader.get_recent_frames(1);
                            if recent_frames.is_empty() {
                                return Err(NanonisError::Protocol(
                                    "No frames available from TCP reader"
                                        .to_string(),
                                ));
                            }

                            recent_frames[0].signal_frame.data
                                [tcp_channel as usize]
                        };

                        log::info!(
                            "Baseline signal: {:.3}, threshold: {:.3}",
                            baseline_value,
                            allowed_signal_change
                        );

                        // 4. Start scan
                        self.client.scan_action(
                            ScanAction::Start,
                            ScanDirection::Down,
                        )?;
                        log::info!("Scan started");

                        // Wait for scan to actually start (max 5 seconds)
                        let mut scan_started = false;
                        for _ in 0..50 {
                            // Check for shutdown request
                            if self.is_shutdown_requested() {
                                log::info!("Shutdown requested while waiting for scan to start");
                                let _ = self.client.scan_action(
                                    ScanAction::Stop,
                                    ScanDirection::Up,
                                );
                                let _ = self.client.bias_set(initial_bias);
                                return Err(NanonisError::Protocol(
                                    "Shutdown requested".to_string(),
                                ));
                            }
                            std::thread::sleep(Duration::from_millis(100));
                            let is_scanning = self.client.scan_status_get()?;
                            if is_scanning {
                                scan_started = true;
                                log::info!("Scan started successfully");
                                break;
                            }
                        }

                        if !scan_started {
                            return Err(NanonisError::Protocol(
                                "Scan failed to start within 5 seconds"
                                    .to_string(),
                            ));
                        }

                        // 5. Sweep bias from upper to lower
                        let bias_step_size =
                            (bias_range.1 - bias_range.0) / (bias_steps as f32);
                        let mut current_bias = bias_range.0;

                        for step_num in 0..bias_steps {
                            // Check for shutdown request
                            if self.is_shutdown_requested() {
                                log::info!("Shutdown requested during bias sweep at step {}/{}", step_num + 1, bias_steps);
                                let _ = self.client.scan_action(
                                    ScanAction::Stop,
                                    ScanDirection::Up,
                                );
                                let _ = self.client.bias_set(initial_bias);
                                return Err(NanonisError::Protocol(
                                    "Shutdown requested".to_string(),
                                ));
                            }
                            self.client.bias_set(current_bias)?;
                            log::debug!(
                                "Step {}/{}: bias={:.2}V",
                                step_num + 1,
                                bias_steps,
                                current_bias
                            );
                            // Interruptible sleep: split into 10ms chunks for responsive shutdown
                            let sleep_chunks =
                                (step_duration.as_millis() / 10).max(1) as u32;
                            let chunk_duration = step_duration / sleep_chunks;
                            for _ in 0..sleep_chunks {
                                if self.is_shutdown_requested() {
                                    log::info!("Shutdown requested during bias sweep step sleep");
                                    let _ = self.client.scan_action(
                                        ScanAction::Stop,
                                        ScanDirection::Up,
                                    );
                                    let _ = self.client.bias_set(initial_bias);
                                    return Err(NanonisError::Protocol(
                                        "Shutdown requested".to_string(),
                                    ));
                                }
                                std::thread::sleep(chunk_duration);
                            }
                            current_bias += bias_step_size;
                        }

                        log::info!("Bias sweep completed");

                        // 6. Read final signal value once after finishing
                        let final_value = {
                            let tcp_reader =
                                self.tcp_reader_mut().ok_or_else(|| {
                                    NanonisError::Protocol(
                                        "TCP reader not available".to_string(),
                                    )
                                })?;

                            let recent_frames = tcp_reader.get_recent_frames(1);
                            if recent_frames.is_empty() {
                                return Err(NanonisError::Protocol(
                                    "No frames available from TCP reader"
                                        .to_string(),
                                ));
                            }

                            recent_frames[0].signal_frame.data
                                [tcp_channel as usize]
                        };

                        // 7. Stop scan, withdraw, then restore bias
                        let _ = self
                            .client
                            .scan_action(ScanAction::Stop, ScanDirection::Up);

                        // Withdraw before changing bias
                        if let Err(e) = self
                            .client
                            .z_ctrl_withdraw(true, Duration::from_secs(5))
                        {
                            log::error!(
                                "Failed to withdraw before restoring bias: {}",
                                e
                            );
                        }

                        // Delay before changing bias
                        std::thread::sleep(Duration::from_millis(200));

                        if let Err(e) = self.client.bias_set(initial_bias) {
                            log::error!(
                                "Failed to restore initial bias: {}",
                                e
                            );
                        } else {
                            log::info!(
                                "Bias restored to {:.3} V",
                                initial_bias
                            );
                        }

                        // 8. Check if change exceeded threshold
                        let signal_change =
                            (final_value - baseline_value).abs();
                        let is_stable = signal_change <= allowed_signal_change;

                        log::info!(
                            "Bias sweep result: baseline={:.3}, final={:.3}, change={:.3}, threshold={:.3}, stable={}",
                            baseline_value,
                            final_value,
                            signal_change,
                            allowed_signal_change,
                            is_stable
                        );

                        // 9. Populate metrics
                        metrics.insert(
                            "baseline_value".to_string(),
                            baseline_value,
                        );
                        metrics.insert("final_value".to_string(), final_value);
                        metrics
                            .insert("signal_change".to_string(), signal_change);
                        metrics.insert(
                            "threshold".to_string(),
                            allowed_signal_change,
                        );

                        // 10. Add recommendations
                        if is_stable {
                            recommendations.push(format!(
                                "Tip is stable - signal change {:.3} within threshold {:.3}",
                                signal_change, allowed_signal_change
                            ));
                        } else {
                            recommendations.push(format!(
                                "Tip is blunt - signal change {:.3} exceeded threshold {:.3}. Tip shaping recommended.",
                                signal_change, allowed_signal_change
                            ));
                        }

                        // Create measured values map
                        let mut measured_values = HashMap::new();
                        measured_values.insert(
                            signal.clone(),
                            vec![baseline_value, final_value],
                        );

                        (is_stable, measured_values)
                    }
                };

                let analysis_duration = start_time.elapsed();
                let result = StabilityResult {
                    is_stable,
                    method_used: format!("{:?}", method.clone()),
                    measured_values,
                    analysis_duration,
                    metrics,
                    potential_damage_detected: !is_stable
                        && matches!(
                            method,
                            TipStabilityMethod::BiasSweepResponse { .. }
                        ),
                    recommendations,
                };

                Ok(ActionResult::StabilityResult(result))
            }

            Action::ReadStableSignal {
                signal,
                data_points,
                use_new_data,
                stability_method,
                timeout,
                retry_count,
            } => {
                use std::time::Instant;

                let start_time = Instant::now();
                let data_points = data_points.unwrap_or(50);
                let max_retries = retry_count.unwrap_or(0);

                // Validate TCP logger is configured and active
                let tcp_config =
                    self.tcp_reader_config.as_ref().ok_or_else(|| {
                        NanonisError::Protocol(
                            "TCP logger not configured".to_string(),
                        )
                    })?;

                // Convert Nanonis signal index to TCP channel using registry
                log::debug!(
                    "ReadStableSignal: Looking up signal {} in signal registry",
                    signal.index
                );

                // Look up the signal from registry to get TCP channel
                let registry_signal = self
                    .signal_registry
                    .get_by_index(signal.index)
                    .ok_or_else(|| {
                        NanonisError::Protocol(format!(
                            "Signal {} not found in registry",
                            signal.index
                        ))
                    })?;

                let tcp_channel = registry_signal.tcp_channel.ok_or_else(|| {
                    log::error!(
                        "ReadStableSignal: Signal {} (Nanonis index) has no TCP channel mapping",
                        signal.index
                    );
                    NanonisError::Protocol(format!(
                        "Signal {} (Nanonis index) has no TCP channel mapping",
                        signal.index
                    ))
                })?;

                log::debug!(
                    "ReadStableSignal: Signal {} mapped to TCP channel {}",
                    signal.index,
                    tcp_channel
                );

                // Find TCP channel in TCP config channels
                log::debug!(
                    "ReadStableSignal: Signal {} (Nanonis) maps to TCP channel {}",
                    signal.index,
                    tcp_channel
                );
                log::debug!(
                    "ReadStableSignal: Available TCP channels: {:?}",
                    tcp_config.channels
                );
                let signal_channel_idx = tcp_config
                    .channels
                    .iter()
                    .position(|&ch| ch == tcp_channel as i32)
                    .ok_or_else(|| {
                        log::error!("ReadStableSignal: TCP channel {} for signal {} (Nanonis) not found in TCP logger configuration. Available channels: {:?}",
                            tcp_channel, signal.index, tcp_config.channels);
                        NanonisError::Protocol(format!(
                            "TCP channel {} for signal {} (Nanonis) not found in TCP logger configuration. Available: {:?}",
                            tcp_channel, signal.index, tcp_config.channels
                        ))
                    })?;

                log::debug!(
                    "ReadStableSignal: Signal {} (Nanonis) -> TCP channel {} -> Array position {}",
                    signal.index,
                    tcp_channel,
                    signal_channel_idx
                );
                log::debug!(
                    "ReadStableSignal: Full TCP channel list: {:?}",
                    tcp_config.channels
                );

                // Retry loop for data collection and stability analysis
                let mut attempt = 0;

                loop {
                    match self.attempt_stable_signal_read(
                        signal_channel_idx,
                        data_points,
                        use_new_data,
                        timeout,
                        &stability_method,
                    ) {
                        Ok((signal_data, is_stable, metrics)) => {
                            let analysis_duration = start_time.elapsed();

                            if is_stable {
                                // Calculate stable value (mean of the data)
                                let stable_value =
                                    signal_data.iter().sum::<f32>()
                                        / signal_data.len() as f32;

                                use crate::actions::StableSignal;
                                log::info!(
                                    "Stable signal acquired on attempt {} (retries: {})",
                                    attempt + 1,
                                    attempt
                                );

                                let stable_signal = StableSignal {
                                    stable_value,
                                    data_points_used: signal_data.len(),
                                    analysis_duration,
                                    stability_metrics: metrics,
                                    // Only include full buffer when not stable (for debugging)
                                    // When stable, only keep the mean value to reduce log file size
                                    raw_data: if is_stable {
                                        vec![stable_value]
                                    } else {
                                        signal_data
                                    },
                                };

                                // Store for correlation with future CheckTipState calls
                                self.recent_stable_signals.push_back((
                                    stable_signal.clone(),
                                    std::time::Instant::now(),
                                ));
                                // Keep only last 10 stable signal results
                                while self.recent_stable_signals.len() > 10 {
                                    self.recent_stable_signals.pop_front();
                                }

                                return Ok(ActionResult::StableSignal(
                                    stable_signal,
                                ));
                            } else if attempt >= max_retries {
                                // No more retries, return raw data as fallback
                                log::warn!(
                                    "Signal not stable after {} attempts, returning raw data",
                                    attempt + 1
                                );
                                let values: Vec<f64> = signal_data
                                    .iter()
                                    .map(|&x| x as f64)
                                    .collect();
                                return Ok(ActionResult::Values(values));
                            } else {
                                // Signal not stable, but we can retry
                                log::debug!(
                                    "Signal not stable on attempt {}, retrying...",
                                    attempt + 1
                                );
                            }
                        }
                        Err(e) => {
                            log::warn!(
                                "Data collection failed on attempt {}: {}",
                                attempt + 1,
                                e
                            );

                            if attempt >= max_retries {
                                return Err(e);
                            }
                        }
                    }

                    attempt += 1;

                    // Add delay between retries (exponential backoff)
                    if attempt <= max_retries {
                        let delay_ms = 100 * (1 << (attempt - 1).min(4)); // Cap at 1.6s delay
                        log::debug!(
                            "Waiting {}ms before retry attempt {}",
                            delay_ms,
                            attempt + 1
                        );
                        std::thread::sleep(Duration::from_millis(delay_ms));
                    }
                }
            }
            Action::ReachedTargedAmplitude => {
                let ampl_setpoint =
                    self.client_mut().pll_amp_ctrl_setpnt_get(1)?;

                let ampl_current = match self
                    .run(Action::ReadStableSignal {
                        signal: Signal::new("Amplitude".to_string(), 75, None).unwrap(),
                        data_points: Some(50),
                        use_new_data: false,
                        stability_method:
                            crate::actions::SignalStabilityMethod::RelativeStandardDeviation {
                                threshold_percent: 0.2,
                            },
                        timeout: Duration::from_millis(10),
                        retry_count: Some(3), // 3 retries for amplitude check
                    })
                    .go()? {
                        ActionResult::Values(values) => values.iter().map(|v| *v as f32).sum::<f32>() / values.len() as f32,
                        ActionResult::StableSignal(value) => value.stable_value,
                        other => {
                            return Err(NanonisError::Protocol(format!(
                                "CheckAmplitudeStability returned unexpected result type. Expected Values or StableSignal, got {:?}",
                                std::mem::discriminant(&other)
                            )))
                        }
                    };

                let status = (ampl_setpoint - 5e-12..ampl_setpoint + 5e-12)
                    .contains(&ampl_current);

                Ok(ActionResult::Status(status))
            }
        }
    }

    fn check_safetip_status(&mut self, context: &str) -> Result<(), NanonisError> {
        if let Ok(status) = self.client_mut().z_ctrl_status_get() {
            if matches!(status, nanonis_rs::z_ctrl::ZControllerStatus::SafeTip)
            {
                return Err(NanonisError::Protocol(
                    format!("SafeTip triggered ({}), abort!", context),
                ));
            }
        }

        Ok(())
    }

    /// Attempt a single stable signal read (used by retry logic)
    fn attempt_stable_signal_read(
        &self,
        signal_channel_idx: usize,
        data_points: usize,
        use_new_data: bool,
        timeout: Duration,
        stability_method: &crate::actions::SignalStabilityMethod,
    ) -> Result<
        (Vec<f32>, bool, std::collections::HashMap<String, f32>),
        NanonisError,
    > {
        // Collect signal data based on use_new_data flag
        let signal_data: Vec<f32> = if use_new_data {
            // Wait for new data with timeout
            self.collect_new_signal_data(
                signal_channel_idx,
                data_points,
                timeout,
            )?
        } else {
            // Use buffered data
            self.extract_buffered_signal_data(signal_channel_idx, data_points)?
        };

        if signal_data.is_empty() {
            return Err(NanonisError::Protocol(
                "No signal data available".to_string(),
            ));
        }

        // Analyze stability using the specified method
        let (is_stable, metrics) =
            Self::analyze_signal_stability(&signal_data, stability_method);

        Ok((signal_data, is_stable, metrics))
    }

    /// Collect new signal data from TCP logger with timeout
    fn collect_new_signal_data(
        &self,
        signal_channel_idx: usize,
        data_points: usize,
        timeout: Duration,
    ) -> Result<Vec<f32>, NanonisError> {
        use std::time::Instant;

        let tcp_reader = self.tcp_reader.as_ref().ok_or_else(|| {
            NanonisError::Protocol("TCP reader not available".to_string())
        })?;

        let start_time = Instant::now();
        let mut collected_data = Vec::with_capacity(data_points);

        log::debug!(
            "Collecting {} new data points for signal channel {} with timeout {:.1}s",
            data_points,
            signal_channel_idx,
            timeout.as_secs_f32()
        );

        while collected_data.len() < data_points
            && start_time.elapsed() < timeout
        {
            // Get recent data in small chunks to avoid blocking too long
            let recent_frames =
                tcp_reader.get_recent_data(Duration::from_millis(100));

            for frame in recent_frames {
                if collected_data.len() >= data_points {
                    break;
                }

                if let Some(&value) =
                    frame.signal_frame.data.get(signal_channel_idx)
                {
                    collected_data.push(value);
                }
            }

            if collected_data.len() < data_points {
                std::thread::sleep(Duration::from_millis(50)); // Small delay before next check
            }
        }

        if collected_data.is_empty() {
            log::warn!("No data collected within timeout");
        } else {
            log::debug!("Collected {} data points", collected_data.len());
        }

        Ok(collected_data)
    }

    /// Extract buffered signal data from TCP logger
    fn extract_buffered_signal_data(
        &self,
        signal_channel_idx: usize,
        data_points: usize,
    ) -> Result<Vec<f32>, NanonisError> {
        let tcp_reader = self.tcp_reader.as_ref().ok_or_else(|| {
            NanonisError::Protocol("TCP reader not available".to_string())
        })?;

        // Get recent data based on how many points we need
        let recent_frames = tcp_reader.get_recent_frames(data_points);

        let mut signal_data = Vec::new();
        for frame in recent_frames.iter().rev().take(data_points) {
            // Take most recent data points
            if let Some(&value) =
                frame.signal_frame.data.get(signal_channel_idx)
            {
                signal_data.push(value);
            }
        }

        signal_data.reverse(); // Return in chronological order

        log::info!("Extracted {} buffered data points", signal_data.len());
        Ok(signal_data)
    }

    /// Analyze signal stability using the specified method
    fn analyze_signal_stability(
        data: &[f32],
        method: &crate::actions::SignalStabilityMethod,
    ) -> (bool, std::collections::HashMap<String, f32>) {
        use crate::actions::SignalStabilityMethod;

        if data.len() < 2 {
            return (false, std::collections::HashMap::new());
        }

        let mut metrics = std::collections::HashMap::new();
        let mean = data.iter().sum::<f32>() / data.len() as f32;
        let variance = data.iter().map(|v| (v - mean).powi(2)).sum::<f32>()
            / data.len() as f32;
        let std_dev = variance.sqrt();

        metrics.insert("mean".to_string(), mean);
        metrics.insert("std_dev".to_string(), std_dev);
        metrics.insert("variance".to_string(), variance);

        let is_stable = match method {
            SignalStabilityMethod::StandardDeviation { threshold } => {
                metrics.insert("threshold".to_string(), *threshold);
                std_dev <= *threshold
            }

            SignalStabilityMethod::RelativeStandardDeviation {
                threshold_percent,
            } => {
                let relative_std = if mean.abs() > 1e-12 {
                    (std_dev / mean.abs()) * 100.0
                } else {
                    f32::INFINITY
                };
                metrics
                    .insert("relative_std_percent".to_string(), relative_std);
                metrics.insert(
                    "threshold_percent".to_string(),
                    *threshold_percent,
                );
                relative_std <= *threshold_percent
            }

            SignalStabilityMethod::MovingWindow {
                window_size,
                max_variation,
            } => {
                if data.len() < *window_size {
                    return (false, metrics);
                }

                let mut max_window_variation = 0.0f32;
                for window in data.windows(*window_size) {
                    let window_min =
                        window.iter().fold(f32::INFINITY, |a, &b| a.min(b));
                    let window_max =
                        window.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
                    let variation = window_max - window_min;
                    max_window_variation = max_window_variation.max(variation);
                }

                metrics.insert(
                    "max_window_variation".to_string(),
                    max_window_variation,
                );
                metrics.insert("window_size".to_string(), *window_size as f32);
                metrics.insert(
                    "max_variation_threshold".to_string(),
                    *max_variation,
                );
                max_window_variation <= *max_variation
            }

            SignalStabilityMethod::TrendAnalysis { max_slope } => {
                // Simple linear regression to detect trend
                let n = data.len() as f32;
                let x_mean = (n - 1.0) / 2.0; // indices 0, 1, 2, ... n-1
                let y_mean = mean;

                let mut numerator = 0.0;
                let mut denominator = 0.0;
                for (i, &y) in data.iter().enumerate() {
                    let x = i as f32;
                    numerator += (x - x_mean) * (y - y_mean);
                    denominator += (x - x_mean).powi(2);
                }

                let slope = if denominator > 1e-12 {
                    numerator / denominator
                } else {
                    0.0
                };
                let abs_slope = slope.abs();

                metrics.insert("slope".to_string(), slope);
                metrics.insert("abs_slope".to_string(), abs_slope);
                metrics.insert("max_slope_threshold".to_string(), *max_slope);
                abs_slope <= *max_slope
            }

            SignalStabilityMethod::Combined {
                max_std_dev,
                max_slope,
            } => {
                // Calculate slope via linear regression
                let n = data.len() as f32;
                let x_mean = (n - 1.0) / 2.0;
                let y_mean = mean;

                let mut numerator = 0.0;
                let mut denominator = 0.0;
                for (i, &y) in data.iter().enumerate() {
                    let x = i as f32;
                    numerator += (x - x_mean) * (y - y_mean);
                    denominator += (x - x_mean).powi(2);
                }

                let slope = if denominator > 1e-12 {
                    numerator / denominator
                } else {
                    0.0
                };
                let abs_slope = slope.abs();

                // Check both conditions: noise AND drift
                let noise_ok = std_dev <= *max_std_dev;
                let drift_ok = abs_slope <= *max_slope;

                metrics.insert("slope".to_string(), slope);
                metrics.insert("abs_slope".to_string(), abs_slope);
                metrics.insert("max_slope_threshold".to_string(), *max_slope);
                metrics
                    .insert("max_std_dev_threshold".to_string(), *max_std_dev);
                metrics.insert(
                    "noise_ok".to_string(),
                    if noise_ok { 1.0 } else { 0.0 },
                );
                metrics.insert(
                    "drift_ok".to_string(),
                    if drift_ok { 1.0 } else { 0.0 },
                );
                noise_ok && drift_ok
            }
        };

        metrics.insert("data_points".to_string(), data.len() as f32);

        (is_stable, metrics)
    }

    /// Execute action and extract specific type with validation
    ///
    /// This is a convenience method that combines execute() with type extraction,
    /// providing better ergonomics while preserving type safety.
    ///
    /// # Example
    /// ```ignore
    /// use rusty_tip::{ActionDriver, Action, Signal};
    /// use rusty_tip::types::{DataToGet, OsciData};
    ///
    /// let mut driver = ActionDriver::new("127.0.0.1", 6501)?;
    /// let signal = Signal::new("Frequency Shift", 24, None).unwrap();
    /// let osci_data: OsciData = driver.execute_expecting(Action::ReadOsci {
    ///     signal,
    ///     trigger: None,
    ///     data_to_get: DataToGet::Current,
    ///     is_stable: None,
    /// })?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn execute_expecting<T>(
        &mut self,
        action: Action,
    ) -> Result<T, NanonisError>
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
                    let (t0, dt, size, data) =
                        self.client.osci1t_data_get(2)?; // Wait2Triggers = 2

                    if let Some(stable_osci_data) = self
                        .analyze_stability_window(
                            t0,
                            dt,
                            size,
                            data,
                            relative_threshold,
                            absolute_threshold,
                            min_window_percent,
                            stability_fn,
                        )?
                    {
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

                let mut osci_data = OsciData::new_with_stats(
                    t0,
                    dt,
                    stable_data.len() as i32,
                    stable_data,
                    stats,
                );
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
                &crate::actions::Action::Wait {
                    duration: Duration::from_millis(0),
                }, // Placeholder action
                final_result,
                start_time,
                duration,
            )
            .with_metadata("type", "chain_execution")
            .with_metadata("chain_summary", chain_summary)
            .with_metadata("action_count", results.len().to_string());

            if let Err(log_error) =
                self.action_logger.as_mut().unwrap().add(log_entry)
            {
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
        self.action_logger
            .as_ref()
            .map(|logger| (logger.len(), self.action_logging_enabled))
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
        signal: Signal,
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
            _ => {
                Err(NanonisError::Protocol("Expected oscilloscope data".into()))
            }
        }
    }

    /// Convenience method to read oscilloscope data with custom stability function
    pub fn read_oscilloscope_with_stability(
        &mut self,
        signal: Signal,
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
            _ => {
                Err(NanonisError::Protocol("Expected oscilloscope data".into()))
            }
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
    fn expect_from_execution(
        self,
    ) -> Result<crate::types::ExperimentData, NanonisError> {
        self.into_experiment_data()
    }
}

impl ExpectFromExecution<crate::types::ChainExperimentData>
    for ExecutionResult
{
    fn expect_from_execution(
        self,
    ) -> Result<crate::types::ChainExperimentData, NanonisError> {
        self.into_chain_experiment_data()
    }
}

impl ExpectFromExecution<f64> for ExecutionResult {
    fn expect_from_execution(self) -> Result<f64, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::Value(v)) => Ok(v),
            ExecutionResult::Single(ActionResult::Values(mut vs))
                if vs.len() == 1 =>
            {
                Ok(vs.pop().unwrap())
            }
            _ => Err(NanonisError::Protocol(
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
            _ => Err(NanonisError::Protocol(
                "Expected numeric values".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<bool> for ExecutionResult {
    fn expect_from_execution(self) -> Result<bool, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::Status(b)) => Ok(b),
            _ => Err(NanonisError::Protocol(
                "Expected boolean status".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<Position> for ExecutionResult {
    fn expect_from_execution(self) -> Result<Position, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::Position(pos)) => Ok(pos),
            _ => Err(NanonisError::Protocol(
                "Expected position data".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<OsciData> for ExecutionResult {
    fn expect_from_execution(self) -> Result<OsciData, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::OsciData(data)) => Ok(data),
            _ => Err(NanonisError::Protocol(
                "Expected oscilloscope data".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<crate::types::TipShape> for ExecutionResult {
    fn expect_from_execution(
        self,
    ) -> Result<crate::types::TipShape, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::TipState(tip_state)) => {
                Ok(tip_state.shape)
            }
            _ => Err(NanonisError::Protocol("Expected tip state".to_string())),
        }
    }
}

impl ExpectFromExecution<crate::actions::TipState> for ExecutionResult {
    fn expect_from_execution(
        self,
    ) -> Result<crate::actions::TipState, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::TipState(tip_state)) => {
                Ok(tip_state)
            }
            _ => Err(NanonisError::Protocol("Expected tip state".to_string())),
        }
    }
}

impl ExpectFromExecution<crate::actions::StableSignal> for ExecutionResult {
    fn expect_from_execution(
        self,
    ) -> Result<crate::actions::StableSignal, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::StableSignal(
                stable_signal,
            )) => Ok(stable_signal),
            _ => Err(NanonisError::Protocol(
                "Expected stable signal".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<crate::actions::TCPReaderStatus> for ExecutionResult {
    fn expect_from_execution(
        self,
    ) -> Result<crate::actions::TCPReaderStatus, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::TCPReaderStatus(
                tcp_status,
            )) => Ok(tcp_status),
            _ => Err(NanonisError::Protocol(
                "Expected TCP reader status".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<crate::actions::StabilityResult> for ExecutionResult {
    fn expect_from_execution(
        self,
    ) -> Result<crate::actions::StabilityResult, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::StabilityResult(
                stability_result,
            )) => Ok(stability_result),
            _ => Err(NanonisError::Protocol(
                "Expected stability result".to_string(),
            )),
        }
    }
}

impl ExpectFromExecution<Vec<String>> for ExecutionResult {
    fn expect_from_execution(self) -> Result<Vec<String>, NanonisError> {
        match self {
            ExecutionResult::Single(ActionResult::Text(text)) => Ok(text),
            _ => Err(NanonisError::Protocol("Expected text data".to_string())),
        }
    }
}

impl Drop for ActionDriver {
    fn drop(&mut self) {
        log::info!("ActionDriver cleanup starting...");

        // Clean up TCP buffering first
        if let Some(mut reader) = self.tcp_reader.take() {
            let final_data = reader.get_all_data();
            let _ = reader.stop(); // Ignore errors during cleanup
            log::info!(
                "Stopped TCP buffering, collected {} frames",
                final_data.len()
            );
        }

        // Disable safe tip protection before cleanup
        log::info!("Disabling safe tip protection...");
        if let Err(e) = self.client_mut().safe_tip_on_off_set(false) {
            log::warn!("Failed to disable safe tip: {}", e);
        }

        // Perform safe shutdown sequence with blocking operations
        log::info!("Performing safe withdrawal...");
        let withdraw_result = self.execute_chain(vec![
            Action::Withdraw {
                wait_until_finished: true, // Make it blocking
                timeout: Duration::from_secs(5),
            },
            Action::MoveMotorAxis {
                direction: crate::MotorDirection::ZMinus,
                steps: 10,
                blocking: false,
            },
        ]);

        if let Err(e) = withdraw_result {
            log::warn!("Cleanup withdrawal failed: {}", e);
        } else {
            log::info!("Safe withdrawal completed");
        }

        log::info!("ActionDriver cleanup complete");
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
