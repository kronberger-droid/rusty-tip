use config::{Config, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};
use std::path::Path;

use rusty_tip::{PulseMethod, StabilityConfig};

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
    /// Validate all configuration values
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate stability config
        self.tip_prep.stability.validate()
            .map_err(ConfigError::Message)?;

        // Validate pulse method
        self.pulse_method.validate()
            .map_err(|e| ConfigError::Message(format!("Invalid pulse_method: {}", e)))?;

        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct NanonisConfig {
    pub host_ip: String,
    pub control_ports: Vec<u16>,
    /// Optional path to a Nanonis layout file (.lyt) to load during initialization
    /// If set, this layout will be loaded before tip preparation starts
    pub layout_file: Option<String>,
    /// Optional path to a Nanonis settings file (.ini) to load during initialization
    /// If set, these settings will be loaded before tip preparation starts
    pub settings_file: Option<String>,
}


#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DataAcquisitionConfig {
    pub data_port: u16,
    pub sample_rate: u32,
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
    /// Stability check configuration (includes check_stability flag)
    #[serde(default)]
    pub stability: StabilityConfig,
    /// Initial bias voltage (V) set before the first approach. Default: -0.5 V
    #[serde(default = "default_initial_bias_v")]
    pub initial_bias_v: f32,
    /// Initial Z-controller setpoint (A) set before the first approach. Default: 100 pA
    #[serde(default = "default_initial_z_setpoint_a")]
    pub initial_z_setpoint_a: f32,
    /// Safe tip threshold (A) for safe tip configuration. Default: 1 nA
    #[serde(default = "default_safe_tip_threshold")]
    pub safe_tip_threshold: f32,
    /// Timing and step configuration for the tip controller
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

/// Load configuration from file with layered fallbacks
pub fn load_config(config_path: Option<&Path>) -> Result<AppConfig, ConfigError> {
    // Start with an empty builder - defaults will be applied by serde #[serde(default)]
    let mut builder = Config::builder();

    // Add config file source
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
        // Try common config file locations
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

    // If no config file was found, use defaults
    if !config_file_found {
        builder = builder.add_source(Config::try_from(&AppConfig::default())?);
    }

    // Add environment variable overrides with prefix "RUSTY_TIP_"
    builder = builder.add_source(
        Environment::with_prefix("RUSTY_TIP")
            .separator("__")
            .try_parsing(true),
    );

    let config = builder.build()?;
    let app_config = config.try_deserialize::<AppConfig>()?;

    // Validate configuration before returning
    app_config.validate()?;

    Ok(app_config)
}

/// Load configuration with error handling
///
/// If a config path is provided and loading fails, this function will panic
/// rather than silently falling back to defaults, since that would likely
/// cause unexpected behavior.
///
/// If no config path is provided, it will try common locations and fall back
/// to defaults only if no config file exists.
pub fn load_config_or_default(config_path: Option<&Path>) -> AppConfig {
    match load_config(config_path) {
        Ok(config) => {
            log::info!("Configuration loaded successfully");
            config
        }
        Err(e) => {
            if config_path.is_some() {
                // User explicitly provided a config path - don't silently use defaults
                panic!(
                    "Failed to load configuration: {}\n\
                    Please fix the configuration file or remove the --config argument to use defaults.",
                    e
                );
            } else {
                // No explicit config path - falling back to defaults is acceptable
                log::warn!("No configuration file found, using defaults");
                AppConfig::default()
            }
        }
    }
}
