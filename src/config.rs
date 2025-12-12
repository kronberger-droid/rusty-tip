use config::{Config, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TcpChannelMapping {
    pub nanonis_index: u8,
    pub tcp_channel: u8,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppConfig {
    pub nanonis: NanonisConfig,
    pub data_acquisition: DataAcquisitionConfig,
    pub experiment_logging: ExperimentLoggingConfig,
    pub console: ConsoleConfig,
    pub tip_prep: TipPrepConfig,
    pub pulse_method: PulseMethodConfig,
    #[serde(default)]
    pub tcp_channel_mapping: Option<Vec<TcpChannelMapping>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct NanonisConfig {
    pub host_ip: String,
    pub control_ports: Vec<u16>,
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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TipPrepConfig {
    pub sharp_tip_bounds: [f32; 2],
    pub stable_tip_allowed_change: f32,
    pub check_stability: bool,
    pub max_cycles: Option<usize>,
    pub max_duration_secs: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PolaritySign {
    Positive,
    Negative,
}

impl Default for PolaritySign {
    fn default() -> Self {
        PolaritySign::Positive
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RandomPolaritySwitchConfig {
    pub enabled: bool,
    pub switch_every_n_pulses: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PulseMethodConfig {
    Fixed {
        pulse_voltage: Vec<f32>,
        #[serde(default)]
        polarity: PolaritySign,
        #[serde(default)]
        random_polarity_switch: Option<RandomPolaritySwitchConfig>,
    },
    Stepping {
        voltage_bounds: [f32; 2],
        voltage_steps: u16,
        cycles_before_step: u16,
        threshold_type: String,
        threshold_value: f32,
        #[serde(default)]
        polarity: PolaritySign,
        #[serde(default)]
        random_polarity_switch: Option<RandomPolaritySwitchConfig>,
    },
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            nanonis: NanonisConfig::default(),
            data_acquisition: DataAcquisitionConfig::default(),
            experiment_logging: ExperimentLoggingConfig::default(),
            console: ConsoleConfig::default(),
            tip_prep: TipPrepConfig::default(),
            pulse_method: PulseMethodConfig::default(),
            tcp_channel_mapping: None,
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
            stable_tip_allowed_change: 0.2,
            check_stability: true,
            max_cycles: Some(10000),
            max_duration_secs: Some(12000),
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
            polarity: PolaritySign::Positive,
            random_polarity_switch: None,
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
