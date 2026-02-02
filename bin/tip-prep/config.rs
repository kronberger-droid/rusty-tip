use config::{Config, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::tip_prep::PulseMethod;

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
        self.tip_prep.stability.validate()?;

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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BiasSweepPolarity {
    /// Sweep from upper_bound toward lower_bound (toward zero)
    Positive,
    /// Sweep from -upper_bound toward -lower_bound (toward zero)
    Negative,
    /// Two sweeps: positive first (toward zero), then negative (toward zero)
    #[default]
    Both,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StabilityConfig {
    /// Whether to perform stability checking
    /// When true, performs a scan with bias sweep to verify tip stability
    /// When false, only checks if tip is sharp based on bounds
    pub check_stability: bool,
    /// Maximum allowed change in signal for tip to be considered stable (in Hz)
    /// During the bias sweep, if the signal changes more than this threshold,
    /// the tip is considered unstable
    pub stable_tip_allowed_change: f32,
    /// Bias voltage range for stability sweep (lower, upper) in V
    /// Must be positive magnitude-only; polarity_mode determines sign
    pub bias_range: (f32, f32),
    /// Number of steps in the bias sweep
    pub bias_steps: u16,
    /// Time to wait at each step in ms
    pub step_period_ms: u64,
    /// Maximum duration for stability check in seconds
    pub max_duration_secs: u64,
    /// Polarity mode for bias sweep
    #[serde(default)]
    pub polarity_mode: BiasSweepPolarity,
    /// Scan speed for stability check in m/s (None = use current scan speed)
    pub scan_speed_m_s: Option<f32>,
}

impl Default for StabilityConfig {
    fn default() -> Self {
        Self {
            check_stability: true,
            stable_tip_allowed_change: 0.2,
            bias_range: (0.01, 2.0), // Strictly positive range
            bias_steps: 1000,
            step_period_ms: 200,
            max_duration_secs: 100,
            polarity_mode: BiasSweepPolarity::Both,
            scan_speed_m_s: Some(5e-9), // 5 nm/s default
        }
    }
}

impl StabilityConfig {
    /// Validate configuration values
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate bias_range: lower bound must be >= 0, upper bound must be > 0
        if self.bias_range.0 <= 0.0 || self.bias_range.1 <= 0.0 {
            return Err(ConfigError::Message(format!(
                "bias_range must be strictly positive (got [{}, {}]). Use polarity_mode to control sign.",
                self.bias_range.0, self.bias_range.1
            )));
        }
        if self.bias_range.0 >= self.bias_range.1 {
            return Err(ConfigError::Message(format!(
                "bias_range: lower bound ({}) must be less than upper bound ({})",
                self.bias_range.0, self.bias_range.1
            )));
        }

        // Validate stable_tip_allowed_change is positive
        if self.stable_tip_allowed_change <= 0.0 {
            return Err(ConfigError::Message(format!(
                "stable_tip_allowed_change must be positive, got: {}",
                self.stable_tip_allowed_change
            )));
        }

        // Validate bias_steps is non-zero
        if self.bias_steps == 0 {
            return Err(ConfigError::Message(
                "bias_steps must be greater than zero".to_string()
            ));
        }

        Ok(())
    }
}

fn default_initial_bias_v() -> f32 {
    -500e-3
}

fn default_initial_z_setpoint_a() -> f32 {
    100e-12
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
