use config::{Config, ConfigError, Environment, File};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::controller_types::{PulseMethod, StabilityConfig};

/// One signal index to TCP logger channel assignment beyond the standard map.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, JsonSchema)]
pub struct TcpChannelMapping {
    pub nanonis_index: u8,
    pub tcp_channel: u8,
}

/// Everything a tip-prep run is configured with. Units are SI throughout;
/// the field docs say which.
#[derive(Debug, Deserialize, Serialize, Clone, Default, JsonSchema)]
pub struct AppConfig {
    pub nanonis: NanonisConfig,
    pub data_acquisition: DataAcquisitionConfig,
    pub experiment_logging: ExperimentLoggingConfig,
    pub console: ConsoleConfig,
    pub tip_prep: TipPrepConfig,
    pub pulse_method: PulseMethod,
    #[serde(default)]
    pub tcp_channel_mapping: Option<Vec<TcpChannelMapping>>,
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate stability config
        self.tip_prep
            .stability
            .validate()
            .map_err(ConfigError::Message)?;

        // Validate pulse method
        self.pulse_method
            .validate()
            .map_err(|e| ConfigError::Message(format!("Invalid pulse_method: {}", e)))?;

        // `cycle % status_interval` is evaluated every cycle.
        if self.tip_prep.timing.status_interval == 0 {
            return Err(ConfigError::Message(
                "tip_prep.timing.status_interval must be at least 1".into(),
            ));
        }

        Ok(())
    }
}

/// Where the controller is. The workbench connects through its own
/// Connection page and ignores this section.
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct NanonisConfig {
    pub host_ip: String,
    /// Command ports; the first one is used.
    pub control_ports: Vec<u16>,
    /// Nanonis layout file, loaded on connect.
    pub layout_file: Option<String>,
    /// Nanonis settings file, loaded on connect.
    pub settings_file: Option<String>,
}

fn default_stable_signal_samples() -> usize {
    100
}

/// How the signal stream is acquired. The thresholds a reading is *judged*
/// against live in [`SignalStabilityConfig`], not here.
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct DataAcquisitionConfig {
    /// TCP logger data port.
    pub data_port: u16,
    /// Stream rate to ask the TCP logger for, in Hz. The logger delivers its
    /// base rate divided by an integer, so the nearest such rate is what
    /// arrives; the controller measures it at startup, logs it, and the
    /// routine judges drift at the measured rate. 1000 Hz makes the default
    /// 100-sample stable read take a tenth of a second.
    #[schemars(extend("x-unit" = "Hz"))]
    pub sample_rate: u32,
    /// Number of TCP stream samples to average for a stable signal read.
    #[serde(default = "default_stable_signal_samples")]
    pub stable_signal_samples: usize,
}

/// The JSONL run log. The workbench writes logs where its Connection page
/// says and ignores this section.
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct ExperimentLoggingConfig {
    pub enabled: bool,
    pub output_path: String,
}

#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct ConsoleConfig {
    /// trace, debug, info, warn or error.
    pub verbosity: String,
}

fn default_initial_bias_v() -> f64 {
    -500e-3
}
fn default_initial_z_setpoint_a() -> f64 {
    100e-12
}
fn default_safe_tip_threshold() -> f64 {
    1e-9
}
fn default_pulse_width_ms() -> u64 {
    50
}
fn default_post_approach_settle_ms() -> u64 {
    2000
}
fn default_post_reposition_settle_ms() -> u64 {
    1000
}
fn default_post_move_settle_ms() -> u64 {
    500
}
fn default_buffer_clear_wait_ms() -> u64 {
    500
}
fn default_post_pulse_settle_ms() -> u64 {
    1000
}
fn default_reposition_steps() -> [i16; 2] {
    [3, 3]
}
fn default_status_interval() -> usize {
    10
}
/// 0.2.3 gave the approaches that start from a full withdraw ten minutes.
fn default_approach_timeout_ms() -> u64 {
    600_000
}
/// A reposition retracts three coarse steps, so its approach is short.
fn default_reposition_approach_timeout_ms() -> u64 {
    crate::action::z_controller::DEFAULT_APPROACH_TIMEOUT_MS
}
/// 0.2.3 backed the coarse motor off ten steps after the final withdraw; the
/// default here is a shorter two.
fn default_exit_retract_steps() -> u16 {
    2
}

/// Settle times and step counts, in milliseconds and coarse steps.
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct TimingConfig {
    /// Length of one bias pulse.
    #[serde(default = "default_pulse_width_ms")]
    #[schemars(extend("x-unit" = "ms"))]
    pub pulse_width_ms: u64,
    /// Settle after an approach.
    #[serde(default = "default_post_approach_settle_ms")]
    #[schemars(extend("x-unit" = "ms"))]
    pub post_approach_settle_ms: u64,
    /// Settle at the end of a reposition.
    #[serde(default = "default_post_reposition_settle_ms")]
    #[schemars(extend("x-unit" = "ms"))]
    pub post_reposition_settle_ms: u64,
    /// Settle time between the motor move and the approach during a
    /// reposition. Was hard-coded at 500 ms (V1 parity) before it became
    /// configurable.
    #[serde(default = "default_post_move_settle_ms")]
    #[schemars(extend("x-unit" = "ms"))]
    pub post_move_settle_ms: u64,
    /// Wait after clearing the stream buffer before the first read.
    #[serde(default = "default_buffer_clear_wait_ms")]
    #[schemars(extend("x-unit" = "ms"))]
    pub buffer_clear_wait_ms: u64,
    /// Settle after a pulse, before the reposition.
    #[serde(default = "default_post_pulse_settle_ms")]
    #[schemars(extend("x-unit" = "ms"))]
    pub post_pulse_settle_ms: u64,
    /// Coarse motor steps in x and y for each reposition.
    #[serde(default = "default_reposition_steps")]
    #[schemars(extend("x-unit" = "steps"))]
    pub reposition_steps: [i16; 2],
    /// Log a status line every this many cycles.
    #[serde(default = "default_status_interval")]
    pub status_interval: usize,
    /// Budget for an approach that starts from a full withdraw: the first
    /// approach of the run and the re-approach around each stability sweep.
    /// An approach that overruns it is stopped and the run ends in an error,
    /// with the tip withdrawn.
    #[serde(default = "default_approach_timeout_ms")]
    #[schemars(extend("x-unit" = "ms"))]
    pub approach_timeout_ms: u64,
    /// Budget for the approach inside a reposition, which starts only three
    /// coarse steps off the surface.
    #[serde(default = "default_reposition_approach_timeout_ms")]
    #[schemars(extend("x-unit" = "ms"))]
    pub reposition_approach_timeout_ms: u64,
    /// Coarse Z steps to back off after the final withdraw, however the run
    /// ends. A withdraw alone parks the tip at the top of the piezo range,
    /// still within reach of the surface; this puts real distance behind it.
    /// Zero disables the retract.
    #[serde(default = "default_exit_retract_steps")]
    #[schemars(extend("x-unit" = "steps"))]
    pub exit_retract_steps: u16,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            pulse_width_ms: default_pulse_width_ms(),
            post_approach_settle_ms: default_post_approach_settle_ms(),
            post_reposition_settle_ms: default_post_reposition_settle_ms(),
            post_move_settle_ms: default_post_move_settle_ms(),
            buffer_clear_wait_ms: default_buffer_clear_wait_ms(),
            post_pulse_settle_ms: default_post_pulse_settle_ms(),
            reposition_steps: default_reposition_steps(),
            status_interval: default_status_interval(),
            approach_timeout_ms: default_approach_timeout_ms(),
            reposition_approach_timeout_ms: default_reposition_approach_timeout_ms(),
            exit_retract_steps: default_exit_retract_steps(),
        }
    }
}

fn default_max_std_dev_hz() -> f64 {
    1.5
}

fn default_max_slope_hz_per_s() -> f64 {
    0.5
}

fn default_data_collection_duration_ms() -> u64 {
    500
}

fn default_read_timeout_secs() -> u64 {
    15
}

fn default_read_retry_count() -> u32 {
    3
}

/// Signal-read stability thresholds: how clean a frequency-shift reading must
/// be to be trusted as a measurement. Loosen these for noisier tips, tighten
/// for cleaner ones. Tunable at runtime via the config file.
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct SignalStabilityConfig {
    /// Maximum standard deviation (Hz) of the reading to count as stable.
    #[serde(default = "default_max_std_dev_hz")]
    #[schemars(extend("x-unit" = "Hz"))]
    pub max_std_dev_hz: f64,
    /// Maximum drift rate (Hz/s) of the reading to count as stable.
    #[serde(default = "default_max_slope_hz_per_s")]
    #[schemars(extend("x-unit" = "Hz/s"))]
    pub max_slope_hz_per_s: f64,
    /// Data-collection window (ms) for one stable read.
    ///
    /// Unused by the v2 read path, which sizes its batch from
    /// `data_acquisition.stable_signal_samples` instead. Kept so configs
    /// written for v1 still parse.
    #[serde(default = "default_data_collection_duration_ms")]
    #[schemars(extend("x-unit" = "ms"))]
    pub data_collection_duration_ms: u64,
    /// Timeout (s) for acquiring a stable read.
    ///
    /// Unused by the v2 read path, which bounds a read by `read_retry_count`
    /// and its exponential backoff instead. Kept so configs written for v1
    /// still parse.
    #[serde(default = "default_read_timeout_secs")]
    #[schemars(extend("x-unit" = "s"))]
    pub read_timeout_secs: u64,
    /// Number of retries when a stable read isn't found.
    #[serde(default = "default_read_retry_count")]
    pub read_retry_count: u32,
}

impl Default for SignalStabilityConfig {
    fn default() -> Self {
        Self {
            max_std_dev_hz: default_max_std_dev_hz(),
            max_slope_hz_per_s: default_max_slope_hz_per_s(),
            data_collection_duration_ms: default_data_collection_duration_ms(),
            read_timeout_secs: default_read_timeout_secs(),
            read_retry_count: default_read_retry_count(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct TipPrepConfig {
    /// Frequency shift window that counts as a sharp tip, lower then upper.
    #[schemars(extend("x-unit" = "Hz"))]
    pub sharp_tip_bounds: [f64; 2],
    /// Pulse cycles to spend before giving up. Unset means unlimited.
    pub max_cycles: Option<usize>,
    /// Wall-clock budget for the run. Unset means unlimited.
    #[schemars(extend("x-unit" = "s"))]
    pub max_duration_secs: Option<u64>,
    #[serde(default)]
    pub stability: StabilityConfig,
    /// Bias set before the first approach.
    #[serde(default = "default_initial_bias_v")]
    #[schemars(extend("x-unit" = "V", "x-display-unit" = "mV"))]
    pub initial_bias_v: f64,
    /// Z-controller setpoint, a current.
    #[serde(default = "default_initial_z_setpoint_a")]
    #[schemars(extend("x-unit" = "A", "x-display-unit" = "pA"))]
    pub initial_z_setpoint_a: f64,
    /// Safe-tip current threshold for the run.
    #[serde(default = "default_safe_tip_threshold")]
    #[schemars(extend("x-unit" = "A", "x-display-unit" = "pA"))]
    pub safe_tip_threshold: f64,
    #[serde(default)]
    pub timing: TimingConfig,
    /// Signal-read stability thresholds (frequency-shift noise/drift gates)
    #[serde(default)]
    pub signal_stability: SignalStabilityConfig,
}

impl Default for NanonisConfig {
    fn default() -> Self {
        Self {
            host_ip: "127.0.0.1".to_string(),
            control_ports: vec![6501, 6502, 6503, 6504],
            layout_file: None,
            settings_file: None,
        }
    }
}

impl Default for DataAcquisitionConfig {
    fn default() -> Self {
        Self {
            data_port: 6590,
            sample_rate: 1000,
            stable_signal_samples: default_stable_signal_samples(),
        }
    }
}

impl Default for ExperimentLoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            output_path: "./experiments".to_string(),
        }
    }
}

impl Default for ConsoleConfig {
    fn default() -> Self {
        Self {
            verbosity: "info".to_string(),
        }
    }
}

impl Default for TipPrepConfig {
    fn default() -> Self {
        Self {
            sharp_tip_bounds: [-2.0, 0.0],
            max_cycles: Some(10000),
            max_duration_secs: Some(12000),
            stability: StabilityConfig::default(),
            initial_bias_v: default_initial_bias_v(),
            initial_z_setpoint_a: default_initial_z_setpoint_a(),
            safe_tip_threshold: default_safe_tip_threshold(),
            timing: TimingConfig::default(),
            signal_stability: SignalStabilityConfig::default(),
        }
    }
}

/// Load configuration from a required config file path.
///
/// Returns an error if the file does not exist or is invalid.
pub fn load_config(config_path: &Path) -> Result<AppConfig, ConfigError> {
    let mut builder = Config::builder();

    if config_path.exists() {
        builder = builder.add_source(File::from(config_path));
    } else {
        return Err(ConfigError::Message(format!(
            "Config file not found: {}",
            config_path.display()
        )));
    }

    builder = builder.add_source(
        Environment::with_prefix("RUSTY_TIP")
            .separator("__")
            .try_parsing(true),
    );

    let config = builder.build()?;
    let app_config = config.try_deserialize::<AppConfig>()?;
    app_config.validate()?;
    Ok(app_config)
}

/// Load configuration with optional path and fallback behavior.
///
/// - If a path is provided and the file exists, loads from that file.
/// - If a path is provided but the file doesn't exist, returns an error.
/// - If no path is provided, tries common locations, then falls back to defaults.
pub fn load_config_with_fallback(config_path: Option<&Path>) -> Result<AppConfig, ConfigError> {
    let mut builder = Config::builder();
    let mut config_file_found = false;

    if let Some(path) = config_path {
        if path.exists() {
            builder = builder.add_source(File::from(path));
            config_file_found = true;
        } else {
            return Err(ConfigError::Message(format!(
                "Config file not found: {}",
                path.display()
            )));
        }
    } else {
        let possible_paths = [
            "config.toml",
            "base_config.toml",
            "examples/base_config.toml",
        ];

        for path in &possible_paths {
            if Path::new(path).exists() {
                builder = builder.add_source(File::with_name(path));
                config_file_found = true;
                break;
            }
        }
    }

    if !config_file_found {
        builder = builder.add_source(Config::try_from(&AppConfig::default())?);
    }

    builder = builder.add_source(
        Environment::with_prefix("RUSTY_TIP")
            .separator("__")
            .try_parsing(true),
    );

    let config = builder.build()?;
    let app_config = config.try_deserialize::<AppConfig>()?;
    app_config.validate()?;
    Ok(app_config)
}

/// Load configuration with error handling for CLI use.
///
/// If a config path is provided and loading fails, this function will panic.
/// If no config path is provided, falls back to defaults.
pub fn load_config_or_default(config_path: Option<&Path>) -> AppConfig {
    match load_config_with_fallback(config_path) {
        Ok(config) => {
            log::info!("Configuration loaded successfully");
            config
        }
        Err(e) => {
            if config_path.is_some() {
                panic!(
                    "Failed to load configuration: {}\n\
                    Please fix the configuration file or remove the --config argument to use defaults.",
                    e
                );
            } else {
                log::warn!("No configuration file found, using defaults");
                AppConfig::default()
            }
        }
    }
}
