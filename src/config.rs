use config::{Config, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::controller_types::{PulseMethod, StabilityConfig};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TcpChannelMapping {
    pub nanonis_index: u8,
    pub tcp_channel: u8,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
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
        self.tip_prep
            .stability
            .validate()
            .map_err(ConfigError::Message)?;
        self.pulse_method
            .validate()
            .map_err(|e| ConfigError::Message(format!("Invalid pulse_method: {}", e)))?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct NanonisConfig {
    pub host_ip: String,
    pub control_ports: Vec<u16>,
    pub layout_file: Option<String>,
    pub settings_file: Option<String>,
}

fn default_stable_signal_samples() -> usize {
    100
}

fn default_max_std_dev() -> f64 {
    1.0
}

fn default_max_slope() -> f64 {
    0.01
}

fn default_stable_read_retries() -> usize {
    3
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DataAcquisitionConfig {
    pub data_port: u16,
    pub sample_rate: u32,
    /// Number of TCP stream samples to average for a stable signal read.
    #[serde(default = "default_stable_signal_samples")]
    pub stable_signal_samples: usize,
    /// Maximum standard deviation for a signal to be considered stable (Hz).
    #[serde(default = "default_max_std_dev")]
    pub max_std_dev: f64,
    /// Maximum linear regression slope for a signal to be considered stable (Hz/sample).
    #[serde(default = "default_max_slope")]
    pub max_slope: f64,
    /// Number of retries with exponential backoff when signal is not stable.
    #[serde(default = "default_stable_read_retries")]
    pub stable_read_retries: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ExperimentLoggingConfig {
    pub enabled: bool,
    pub output_path: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConsoleConfig {
    pub verbosity: String,
}

fn default_initial_bias_v() -> f32 {
    -500e-3
}
fn default_initial_z_setpoint_a() -> f32 {
    100e-12
}
fn default_safe_tip_threshold() -> f32 {
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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TimingConfig {
    #[serde(default = "default_pulse_width_ms")]
    pub pulse_width_ms: u64,
    #[serde(default = "default_post_approach_settle_ms")]
    pub post_approach_settle_ms: u64,
    #[serde(default = "default_post_reposition_settle_ms")]
    pub post_reposition_settle_ms: u64,
    #[serde(default = "default_buffer_clear_wait_ms")]
    pub buffer_clear_wait_ms: u64,
    #[serde(default = "default_post_pulse_settle_ms")]
    pub post_pulse_settle_ms: u64,
    #[serde(default = "default_reposition_steps")]
    pub reposition_steps: [i16; 2],
    #[serde(default = "default_status_interval")]
    pub status_interval: usize,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            pulse_width_ms: default_pulse_width_ms(),
            post_approach_settle_ms: default_post_approach_settle_ms(),
            post_reposition_settle_ms: default_post_reposition_settle_ms(),
            buffer_clear_wait_ms: default_buffer_clear_wait_ms(),
            post_pulse_settle_ms: default_post_pulse_settle_ms(),
            reposition_steps: default_reposition_steps(),
            status_interval: default_status_interval(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TipPrepConfig {
    pub sharp_tip_bounds: [f32; 2],
    pub max_cycles: Option<usize>,
    pub max_duration_secs: Option<u64>,
    #[serde(default)]
    pub stability: StabilityConfig,
    #[serde(default = "default_initial_bias_v")]
    pub initial_bias_v: f32,
    #[serde(default = "default_initial_z_setpoint_a")]
    pub initial_z_setpoint_a: f32,
    #[serde(default = "default_safe_tip_threshold")]
    pub safe_tip_threshold: f32,
    #[serde(default)]
    pub timing: TimingConfig,
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
            sample_rate: 2000,
            stable_signal_samples: default_stable_signal_samples(),
            max_std_dev: default_max_std_dev(),
            max_slope: default_max_slope(),
            stable_read_retries: default_stable_read_retries(),
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
