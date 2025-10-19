use config::{Config, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppConfig {
    pub nanonis: NanonisConfig,
    pub data_logger: DataLoggerConfig,
    pub logging: LoggingConfig,
    pub tip_prep: TipPrepConfig,
    pub pulse_method: PulseMethodConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct NanonisConfig {
    pub host_ip: String,
    pub control_ports: Vec<u16>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DataLoggerConfig {
    pub data_port: u16,
    pub sample_rate: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
    pub action_logging: bool,
    pub log_path: String,
    pub log_level: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TipPrepConfig {
    pub sharp_tip_bounds: [f32; 2],
    pub stable_tip_allowed_change: f32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PulseMethodConfig {
    Fixed { pulse_voltage: Vec<f32> },
    Stepping {
        voltage_bounds: [f32; 2],
        voltage_steps: u16,
        cycles_before_step: u16,
        threshold_type: String,
        threshold_value: f32,
    },
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            nanonis: NanonisConfig::default(),
            data_logger: DataLoggerConfig::default(),
            logging: LoggingConfig::default(),
            tip_prep: TipPrepConfig::default(),
            pulse_method: PulseMethodConfig::default(),
        }
    }
}

impl Default for NanonisConfig {
    fn default() -> Self {
        Self {
            host_ip: "127.0.0.1".to_string(),
            control_ports: vec![6501, 6502, 6503, 6504],
        }
    }
}

impl Default for DataLoggerConfig {
    fn default() -> Self {
        Self {
            data_port: 6590,
            sample_rate: 2000,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            action_logging: true,
            log_path: "./logs".to_string(),
            log_level: "info".to_string(),
        }
    }
}

impl Default for TipPrepConfig {
    fn default() -> Self {
        Self {
            sharp_tip_bounds: [-2.0, 0.0],
            stable_tip_allowed_change: 0.2,
        }
    }
}

impl Default for PulseMethodConfig {
    fn default() -> Self {
        Self::Stepping {
            voltage_bounds: [2.0, 6.0],
            voltage_steps: 4,
            cycles_before_step: 2,
            threshold_type: "absolute".to_string(),
            threshold_value: 0.1,
        }
    }
}

/// Load configuration from file with layered fallbacks
pub fn load_config(config_path: Option<&Path>) -> Result<AppConfig, ConfigError> {
    let mut builder = Config::builder()
        .add_source(Config::try_from(&AppConfig::default())?);

    if let Some(path) = config_path {
        if path.exists() {
            builder = builder.add_source(File::from(path));
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
                break;
            }
        }
    }

    // Add environment variable overrides with prefix "RUSTY_TIP_"
    builder = builder.add_source(
        Environment::with_prefix("RUSTY_TIP")
            .separator("__")
            .try_parsing(true),
    );

    let config = builder.build()?;
    config.try_deserialize::<AppConfig>()
}

/// Load configuration with better error handling and defaults
pub fn load_config_or_default(config_path: Option<&Path>) -> AppConfig {
    match load_config(config_path) {
        Ok(config) => {
            log::info!("Configuration loaded successfully");
            config
        }
        Err(e) => {
            log::warn!("Failed to load config ({}), using defaults", e);
            AppConfig::default()
        }
    }
}