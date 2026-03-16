use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui;
use egui_plot::{HLine, Line, Plot, PlotPoints};
use log::{error, info, LevelFilter};
use std::collections::HashMap;
use std::path::Path;

use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rusty_tip::config::{
    load_config_with_fallback, AppConfig, ConsoleConfig, DataAcquisitionConfig,
    ExperimentLoggingConfig, NanonisConfig, TcpChannelMapping, TimingConfig,
    TipPrepConfig,
};
use rusty_tip::event::{
    ChannelForwarder, ConsoleLogger, Event, EventAccumulator, EventBus, FileLogger,
};
use rusty_tip::nanonis_controller::{NanonisController, NanonisSetupConfig};
use rusty_tip::signal_registry::SignalRegistry;
use rusty_tip::spm_controller::SpmController;
use rusty_tip::spm_error::SpmError;
use rusty_tip::tip_prep::{run_tip_prep, Outcome};
use rusty_tip::workflow::ShutdownFlag;
use rusty_tip::{
    BiasSweepPolarity, PolaritySign, PulseMethod, RandomPolaritySwitch, StabilityConfig,
};

// ============================================================================
// Tee Writer - sends env_logger output to both stderr and GUI channel
// ============================================================================

struct TeeWriter {
    sender: Sender<String>,
    stderr: std::io::Stderr,
}

impl std::io::Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stderr.write_all(buf)?;
        if let Ok(s) = std::str::from_utf8(buf) {
            let trimmed = s.trim_end_matches('\n');
            if !trimmed.is_empty() {
                let _ = self.sender.try_send(trimmed.to_string());
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stderr.flush()
    }
}

pub fn init_logging(level: LevelFilter) -> Receiver<String> {
    let (tx, rx) = unbounded();
    let writer = TeeWriter {
        sender: tx,
        stderr: std::io::stderr(),
    };

    env_logger::Builder::new()
        .filter_level(level)
        .filter_module("winit", LevelFilter::Off)
        .filter_module("eframe", LevelFilter::Off)
        .filter_module("egui_glow", LevelFilter::Off)
        .filter_module("wgpu", LevelFilter::Off)
        .filter_module("naga", LevelFilter::Off)
        .filter_module("zbus", LevelFilter::Off)
        .filter_module("tracing", LevelFilter::Off)
        .filter_module("accesskit", LevelFilter::Off)
        .format_timestamp_millis()
        .target(env_logger::Target::Pipe(Box::new(writer)))
        .init();

    rx
}

// ============================================================================
// Tab System
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Control,
    Configuration,
}

// ============================================================================
// Run Status for GUI
// ============================================================================

#[derive(Debug, Clone)]
pub enum RunStatus {
    Idle,
    Running,
    Completed,
    Error(String),
}

// ============================================================================
// Tip Prep State -- populated from Event stream
// ============================================================================

#[derive(Debug, Clone, Default)]
struct TipPrepState {
    cycle: usize,
    elapsed_secs: f64,
    freq_shift: Option<f64>,
    pulse_voltage: f64,
    phase: String,
    is_sharp: bool,
}

// ============================================================================
// Time-series data point for plots
// ============================================================================

#[derive(Debug, Clone)]
struct DataPoint {
    time_s: f64,
    value: f64,
}

// ============================================================================
// Editable Configuration
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PulseMethodType {
    Fixed,
    #[default]
    Stepping,
    Linear,
}

// ============================================================================
// Editable TCP Channel Mapping
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct EditableTcpMapping {
    pub nanonis_index: String,
    pub tcp_channel: String,
}

#[derive(Debug, Clone)]
pub struct EditableConfig {
    // Nanonis
    pub host_ip: String,
    pub control_port: String,
    pub data_port: String,
    pub layout_file: String,
    pub settings_file: String,

    // Data Acquisition
    pub sample_rate: String,
    pub stable_signal_samples: String,

    // Experiment Logging
    pub logging_enabled: bool,
    pub logging_output_path: String,

    // Console
    pub verbosity: String,

    // Tip prep
    pub sharp_tip_lower: String,
    pub sharp_tip_upper: String,
    pub max_cycles: String,
    pub max_duration_secs: String,
    pub initial_bias_v: String,
    pub initial_z_setpoint_pa: String,
    pub safe_tip_threshold_pa: String,

    // Stability
    pub check_stability: bool,
    pub stable_tip_allowed_change: String,
    pub bias_range_lower: String,
    pub bias_range_upper: String,
    pub bias_steps: String,
    pub step_period_ms: String,
    pub stability_max_duration: String,
    pub polarity_mode: BiasSweepPolarity,
    pub scan_speed_nm_s: String,

    // Pulse method
    pub pulse_method_type: PulseMethodType,
    pub pulse_voltage: String,
    pub pulse_voltage_min: String,
    pub pulse_voltage_max: String,
    pub pulse_polarity: PolaritySign,

    // Stepping-specific
    pub voltage_steps: String,
    pub cycles_before_step: String,
    pub threshold_value: String,

    // Linear-specific
    pub linear_clamp_min: String,
    pub linear_clamp_max: String,

    // Random polarity switch
    pub random_polarity_enabled: bool,
    pub random_polarity_switch_every: String,

    // TCP channel mapping
    pub tcp_channel_mappings: Vec<EditableTcpMapping>,
}

impl Default for EditableConfig {
    fn default() -> Self {
        Self {
            host_ip: "127.0.0.1".to_string(),
            control_port: "6501".to_string(),
            data_port: "6590".to_string(),
            layout_file: String::new(),
            settings_file: String::new(),
            sample_rate: "2000".to_string(),
            stable_signal_samples: "100".to_string(),
            logging_enabled: true,
            logging_output_path: "./experiments".to_string(),
            verbosity: "info".to_string(),
            sharp_tip_lower: "-2.0".to_string(),
            sharp_tip_upper: "0.0".to_string(),
            max_cycles: "10000".to_string(),
            max_duration_secs: "12000".to_string(),
            initial_bias_v: "-500".to_string(),
            initial_z_setpoint_pa: "100".to_string(),
            safe_tip_threshold_pa: "1000.0".to_string(),
            check_stability: true,
            stable_tip_allowed_change: "0.2".to_string(),
            bias_range_lower: "0.01".to_string(),
            bias_range_upper: "2.0".to_string(),
            bias_steps: "1000".to_string(),
            step_period_ms: "200".to_string(),
            stability_max_duration: "100".to_string(),
            polarity_mode: BiasSweepPolarity::Both,
            scan_speed_nm_s: "5.0".to_string(),
            pulse_method_type: PulseMethodType::Stepping,
            pulse_voltage: "6.0".to_string(),
            pulse_voltage_min: "2.0".to_string(),
            pulse_voltage_max: "6.0".to_string(),
            pulse_polarity: PolaritySign::Negative,
            voltage_steps: "4".to_string(),
            cycles_before_step: "2".to_string(),
            threshold_value: "0.1".to_string(),
            linear_clamp_min: "-20.0".to_string(),
            linear_clamp_max: "0.0".to_string(),
            random_polarity_enabled: false,
            random_polarity_switch_every: "10".to_string(),
            tcp_channel_mappings: Vec::new(),
        }
    }
}

impl EditableConfig {
    pub fn from_app_config(app_config: &AppConfig) -> Self {
        let (
            pulse_method_type,
            pulse_voltage,
            pulse_voltage_min,
            pulse_voltage_max,
            voltage_steps,
            cycles_before_step,
            threshold_value,
            linear_clamp_min,
            linear_clamp_max,
            pulse_polarity,
            random_polarity_enabled,
            random_polarity_switch_every,
        ) = match &app_config.pulse_method {
            PulseMethod::Fixed {
                voltage,
                polarity,
                random_polarity_switch,
            } => {
                let (rp_enabled, rp_every) = match random_polarity_switch {
                    Some(rps) => (rps.enabled, rps.switch_every_n_pulses.to_string()),
                    None => (false, "10".to_string()),
                };
                (
                    PulseMethodType::Fixed,
                    voltage.to_string(),
                    "2.0".to_string(),
                    "6.0".to_string(),
                    "4".to_string(),
                    "2".to_string(),
                    "0.1".to_string(),
                    "-20.0".to_string(),
                    "0.0".to_string(),
                    *polarity,
                    rp_enabled,
                    rp_every,
                )
            }
            PulseMethod::Stepping {
                voltage_bounds,
                voltage_steps,
                cycles_before_step,
                threshold_value,
                polarity,
                random_polarity_switch,
            } => {
                let (rp_enabled, rp_every) = match random_polarity_switch {
                    Some(rps) => (rps.enabled, rps.switch_every_n_pulses.to_string()),
                    None => (false, "10".to_string()),
                };
                (
                    PulseMethodType::Stepping,
                    "6.0".to_string(),
                    voltage_bounds.0.to_string(),
                    voltage_bounds.1.to_string(),
                    voltage_steps.to_string(),
                    cycles_before_step.to_string(),
                    threshold_value.to_string(),
                    "-20.0".to_string(),
                    "0.0".to_string(),
                    *polarity,
                    rp_enabled,
                    rp_every,
                )
            }
            PulseMethod::Linear {
                voltage_bounds,
                linear_clamp,
                polarity,
                random_polarity_switch,
            } => {
                let (rp_enabled, rp_every) = match random_polarity_switch {
                    Some(rps) => (rps.enabled, rps.switch_every_n_pulses.to_string()),
                    None => (false, "10".to_string()),
                };
                (
                    PulseMethodType::Linear,
                    "6.0".to_string(),
                    voltage_bounds.0.to_string(),
                    voltage_bounds.1.to_string(),
                    "4".to_string(),
                    "2".to_string(),
                    "0.1".to_string(),
                    linear_clamp.0.to_string(),
                    linear_clamp.1.to_string(),
                    *polarity,
                    rp_enabled,
                    rp_every,
                )
            }
        };

        Self {
            host_ip: app_config.nanonis.host_ip.clone(),
            control_port: app_config
                .nanonis
                .control_ports
                .first()
                .unwrap_or(&6501)
                .to_string(),
            data_port: app_config.data_acquisition.data_port.to_string(),
            layout_file: app_config.nanonis.layout_file.clone().unwrap_or_default(),
            settings_file: app_config.nanonis.settings_file.clone().unwrap_or_default(),
            sample_rate: app_config.data_acquisition.sample_rate.to_string(),
            stable_signal_samples: app_config
                .data_acquisition
                .stable_signal_samples
                .to_string(),
            logging_enabled: app_config.experiment_logging.enabled,
            logging_output_path: app_config.experiment_logging.output_path.clone(),
            verbosity: app_config.console.verbosity.clone(),
            sharp_tip_lower: app_config.tip_prep.sharp_tip_bounds[0].to_string(),
            sharp_tip_upper: app_config.tip_prep.sharp_tip_bounds[1].to_string(),
            max_cycles: app_config
                .tip_prep
                .max_cycles
                .map(|c| c.to_string())
                .unwrap_or_default(),
            max_duration_secs: app_config
                .tip_prep
                .max_duration_secs
                .map(|d| d.to_string())
                .unwrap_or_default(),
            initial_bias_v: (app_config.tip_prep.initial_bias_v * 1000.0).to_string(),
            initial_z_setpoint_pa: (app_config.tip_prep.initial_z_setpoint_a * 1e12).to_string(),
            safe_tip_threshold_pa: (app_config.tip_prep.safe_tip_threshold * 1e12).to_string(),
            check_stability: app_config.tip_prep.stability.check_stability,
            stable_tip_allowed_change: app_config
                .tip_prep
                .stability
                .stable_tip_allowed_change
                .to_string(),
            bias_range_lower: app_config.tip_prep.stability.bias_range.0.to_string(),
            bias_range_upper: app_config.tip_prep.stability.bias_range.1.to_string(),
            bias_steps: app_config.tip_prep.stability.bias_steps.to_string(),
            step_period_ms: app_config.tip_prep.stability.step_period_ms.to_string(),
            stability_max_duration: app_config
                .tip_prep
                .stability
                .max_duration_secs
                .to_string(),
            polarity_mode: app_config.tip_prep.stability.polarity_mode,
            scan_speed_nm_s: app_config
                .tip_prep
                .stability
                .scan_speed_m_s
                .map(|s| (s * 1e9).to_string())
                .unwrap_or_default(),
            pulse_method_type,
            pulse_voltage,
            pulse_voltage_min,
            pulse_voltage_max,
            pulse_polarity,
            voltage_steps,
            cycles_before_step,
            threshold_value,
            linear_clamp_min,
            linear_clamp_max,
            random_polarity_enabled,
            random_polarity_switch_every,
            tcp_channel_mappings: app_config
                .tcp_channel_mapping
                .as_ref()
                .map(|mappings| {
                    mappings
                        .iter()
                        .map(|m| EditableTcpMapping {
                            nanonis_index: m.nanonis_index.to_string(),
                            tcp_channel: m.tcp_channel.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub fn to_app_config(&self) -> Result<AppConfig, String> {
        let control_port: u16 = self
            .control_port
            .parse()
            .map_err(|_| "Invalid control port")?;
        let data_port: u16 = self.data_port.parse().map_err(|_| "Invalid data port")?;
        let sample_rate: u32 = self
            .sample_rate
            .parse()
            .map_err(|_| "Invalid sample rate")?;
        let stable_signal_samples: usize = self
            .stable_signal_samples
            .parse()
            .map_err(|_| "Invalid stable signal samples")?;
        let sharp_tip_lower: f32 = self
            .sharp_tip_lower
            .parse()
            .map_err(|_| "Invalid sharp tip lower bound")?;
        let sharp_tip_upper: f32 = self
            .sharp_tip_upper
            .parse()
            .map_err(|_| "Invalid sharp tip upper bound")?;
        let max_cycles: Option<usize> = if self.max_cycles.is_empty() {
            None
        } else {
            Some(
                self.max_cycles
                    .parse()
                    .map_err(|_| "Invalid max cycles")?,
            )
        };
        let max_duration_secs: Option<u64> = if self.max_duration_secs.is_empty() {
            None
        } else {
            Some(
                self.max_duration_secs
                    .parse()
                    .map_err(|_| "Invalid max duration")?,
            )
        };
        let initial_bias_mv: f32 = self
            .initial_bias_v
            .parse()
            .map_err(|_| "Invalid initial bias")?;
        let initial_z_setpoint_pa: f32 = self
            .initial_z_setpoint_pa
            .parse()
            .map_err(|_| "Invalid Z setpoint")?;
        let safe_tip_threshold_pa: f32 = self
            .safe_tip_threshold_pa
            .parse()
            .map_err(|_| "Invalid safe tip threshold")?;

        let scan_speed_m_s: Option<f32> = if self.scan_speed_nm_s.is_empty() {
            None
        } else {
            Some(
                self.scan_speed_nm_s
                    .parse::<f32>()
                    .map_err(|_| "Invalid scan speed")?
                    * 1e-9,
            )
        };

        let random_polarity_switch = if self.random_polarity_enabled {
            Some(RandomPolaritySwitch {
                enabled: true,
                switch_every_n_pulses: self
                    .random_polarity_switch_every
                    .parse()
                    .map_err(|_| "Invalid switch every N pulses")?,
            })
        } else {
            None
        };

        let pulse_method = match self.pulse_method_type {
            PulseMethodType::Fixed => PulseMethod::Fixed {
                voltage: self
                    .pulse_voltage
                    .parse()
                    .map_err(|_| "Invalid pulse voltage")?,
                polarity: self.pulse_polarity,
                random_polarity_switch,
            },
            PulseMethodType::Stepping => PulseMethod::Stepping {
                voltage_bounds: (
                    self.pulse_voltage_min
                        .parse()
                        .map_err(|_| "Invalid voltage min")?,
                    self.pulse_voltage_max
                        .parse()
                        .map_err(|_| "Invalid voltage max")?,
                ),
                voltage_steps: self
                    .voltage_steps
                    .parse()
                    .map_err(|_| "Invalid voltage steps")?,
                cycles_before_step: self
                    .cycles_before_step
                    .parse()
                    .map_err(|_| "Invalid cycles before step")?,
                threshold_value: self
                    .threshold_value
                    .parse()
                    .map_err(|_| "Invalid threshold value")?,
                polarity: self.pulse_polarity,
                random_polarity_switch,
            },
            PulseMethodType::Linear => PulseMethod::Linear {
                voltage_bounds: (
                    self.pulse_voltage_min
                        .parse()
                        .map_err(|_| "Invalid voltage min")?,
                    self.pulse_voltage_max
                        .parse()
                        .map_err(|_| "Invalid voltage max")?,
                ),
                linear_clamp: (
                    self.linear_clamp_min
                        .parse()
                        .map_err(|_| "Invalid linear clamp min")?,
                    self.linear_clamp_max
                        .parse()
                        .map_err(|_| "Invalid linear clamp max")?,
                ),
                polarity: self.pulse_polarity,
                random_polarity_switch,
            },
        };

        Ok(AppConfig {
            nanonis: NanonisConfig {
                host_ip: self.host_ip.clone(),
                control_ports: vec![control_port],
                layout_file: if self.layout_file.is_empty() {
                    None
                } else {
                    Some(self.layout_file.clone())
                },
                settings_file: if self.settings_file.is_empty() {
                    None
                } else {
                    Some(self.settings_file.clone())
                },
            },
            data_acquisition: DataAcquisitionConfig {
                data_port,
                sample_rate,
                stable_signal_samples,
                ..Default::default()
            },
            experiment_logging: ExperimentLoggingConfig {
                enabled: self.logging_enabled,
                output_path: self.logging_output_path.clone(),
            },
            console: ConsoleConfig {
                verbosity: self.verbosity.clone(),
            },
            tip_prep: TipPrepConfig {
                sharp_tip_bounds: [sharp_tip_lower, sharp_tip_upper],
                max_cycles,
                max_duration_secs,
                stability: StabilityConfig {
                    check_stability: self.check_stability,
                    stable_tip_allowed_change: self
                        .stable_tip_allowed_change
                        .parse()
                        .map_err(|_| "Invalid allowed change")?,
                    bias_range: (
                        self.bias_range_lower
                            .parse()
                            .map_err(|_| "Invalid bias range lower")?,
                        self.bias_range_upper
                            .parse()
                            .map_err(|_| "Invalid bias range upper")?,
                    ),
                    bias_steps: self
                        .bias_steps
                        .parse()
                        .map_err(|_| "Invalid bias steps")?,
                    step_period_ms: self
                        .step_period_ms
                        .parse()
                        .map_err(|_| "Invalid step period")?,
                    max_duration_secs: self
                        .stability_max_duration
                        .parse()
                        .map_err(|_| "Invalid stability max duration")?,
                    polarity_mode: self.polarity_mode,
                    scan_speed_m_s,
                },
                initial_bias_v: initial_bias_mv / 1000.0,
                initial_z_setpoint_a: initial_z_setpoint_pa * 1e-12,
                safe_tip_threshold: safe_tip_threshold_pa * 1e-12,
                timing: TimingConfig::default(),
            },
            pulse_method,
            tcp_channel_mapping: if self.tcp_channel_mappings.is_empty() {
                None
            } else {
                let mappings: Result<Vec<TcpChannelMapping>, String> = self
                    .tcp_channel_mappings
                    .iter()
                    .map(|m| {
                        Ok(TcpChannelMapping {
                            nanonis_index: m
                                .nanonis_index
                                .parse()
                                .map_err(|_| "Invalid nanonis index")?,
                            tcp_channel: m
                                .tcp_channel
                                .parse()
                                .map_err(|_| "Invalid TCP channel")?,
                        })
                    })
                    .collect();
                Some(mappings?)
            },
        })
    }
}

// ============================================================================
// Main Application
// ============================================================================

pub struct TipPrepApp {
    current_tab: Tab,
    config: EditableConfig,

    // File paths for load/save
    load_path: String,
    save_path: String,

    // Controller state
    controller_thread: Option<JoinHandle<()>>,
    shutdown_flag: Option<ShutdownFlag>,
    run_status: RunStatus,
    start_time: Option<Instant>,

    // Real-time state from V2 event stream
    event_receiver: Option<Receiver<Event>>,
    tip_state: TipPrepState,

    // Time-series data for plots
    freq_shift_history: Vec<DataPoint>,
    voltage_history: Vec<DataPoint>,

    // Sharp tip bounds for overlay (cached from config at start)
    sharp_bounds: Option<(f64, f64)>,

    // Last known action name from events
    current_action: String,

    // Messages
    message: Option<(String, bool)>,

    // Log messages for display
    log_messages: Vec<String>,
    log_receiver: Option<Receiver<String>>,
}

impl TipPrepApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            current_tab: Tab::default(),
            config: EditableConfig::default(),
            load_path: String::new(),
            save_path: String::new(),
            controller_thread: None,
            shutdown_flag: None,
            run_status: RunStatus::Idle,
            start_time: None,
            event_receiver: None,
            tip_state: TipPrepState::default(),
            freq_shift_history: Vec::new(),
            voltage_history: Vec::new(),
            sharp_bounds: None,
            current_action: String::new(),
            message: None,
            log_messages: Vec::new(),
            log_receiver: None,
        }
    }

    pub fn set_log_receiver(&mut self, receiver: Receiver<String>) {
        self.log_receiver = Some(receiver);
    }

    fn is_running(&self) -> bool {
        matches!(self.run_status, RunStatus::Running)
    }

    fn load_config_from_file(&mut self) {
        let config_path = Path::new(&self.load_path);
        if config_path.exists() {
            match load_config_with_fallback(Some(config_path)) {
                Ok(app_config) => {
                    self.config = EditableConfig::from_app_config(&app_config);
                    self.message = Some(("Config loaded".to_string(), false));
                }
                Err(e) => {
                    self.message = Some((format!("Failed to load: {}", e), true));
                }
            }
        } else {
            self.message = Some(("File not found".to_string(), true));
        }
    }

    fn save_config_to_file(&mut self) {
        let save_path = if self.save_path.to_lowercase().ends_with(".toml") {
            self.save_path.clone()
        } else {
            format!("{}.toml", self.save_path)
        };

        match self.config.to_app_config() {
            Ok(app_config) => match toml::to_string_pretty(&app_config) {
                Ok(toml_str) => {
                    if let Err(e) = std::fs::write(&save_path, toml_str) {
                        self.message = Some((format!("Write failed: {}", e), true));
                    } else {
                        self.save_path = save_path;
                        self.message = Some(("Config saved".to_string(), false));
                    }
                }
                Err(e) => {
                    self.message = Some((format!("Serialize failed: {}", e), true));
                }
            },
            Err(e) => {
                self.message = Some((format!("Invalid config: {}", e), true));
            }
        }
    }

    fn start_controller(&mut self) {
        let config = match self.config.to_app_config() {
            Ok(c) => c,
            Err(e) => {
                self.message = Some((format!("Invalid config: {}", e), true));
                return;
            }
        };

        // Cache sharp bounds for plot overlay
        self.sharp_bounds = Some((
            config.tip_prep.sharp_tip_bounds[0] as f64,
            config.tip_prep.sharp_tip_bounds[1] as f64,
        ));

        let shutdown = ShutdownFlag::new();
        self.shutdown_flag = Some(shutdown.clone());

        // Create channel for event forwarding
        let (event_tx, event_rx) = unbounded();
        self.event_receiver = Some(event_rx);

        let handle = thread::spawn(move || {
            if let Err(e) = run_controller(config, shutdown, event_tx) {
                error!("Controller error: {}", e);
            }
        });

        self.controller_thread = Some(handle);
        self.run_status = RunStatus::Running;
        self.start_time = Some(Instant::now());
        self.tip_state = TipPrepState::default();
        self.freq_shift_history.clear();
        self.voltage_history.clear();
        self.current_action.clear();
        self.log_messages.clear();
        info!("Controller started");
        self.message = Some(("Controller started".to_string(), false));
    }

    fn drain_events(&mut self) {
        // Poll log messages
        if let Some(rx) = &self.log_receiver {
            while let Ok(msg) = rx.try_recv() {
                self.log_messages.push(msg);
                if self.log_messages.len() > 1000 {
                    self.log_messages.drain(0..200);
                }
            }
        }

        // Poll events from the V2 event bus
        if let Some(rx) = &self.event_receiver {
            while let Ok(event) = rx.try_recv() {
                match &event {
                    Event::Custom { kind, data } if kind == "tip_prep_state" => {
                        // Parse TipPrepSnapshot from JSON
                        if let Some(cycle) = data.get("cycle").and_then(|v| v.as_u64()) {
                            self.tip_state.cycle = cycle as usize;
                        }
                        if let Some(elapsed) =
                            data.get("elapsed_secs").and_then(|v| v.as_f64())
                        {
                            self.tip_state.elapsed_secs = elapsed;
                        }
                        if let Some(fs) = data.get("freq_shift").and_then(|v| v.as_f64()) {
                            self.tip_state.freq_shift = Some(fs);
                            self.freq_shift_history.push(DataPoint {
                                time_s: self.tip_state.elapsed_secs,
                                value: fs,
                            });
                        }
                        if let Some(pv) = data.get("pulse_voltage").and_then(|v| v.as_f64()) {
                            self.tip_state.pulse_voltage = pv;
                            self.voltage_history.push(DataPoint {
                                time_s: self.tip_state.elapsed_secs,
                                value: pv,
                            });
                        }
                        if let Some(sharp) = data.get("is_sharp").and_then(|v| v.as_bool()) {
                            self.tip_state.is_sharp = sharp;
                        }
                        if let Some(phase) = data.get("phase").and_then(|v| v.as_str()) {
                            self.tip_state.phase = phase.to_string();
                        }
                    }
                    Event::ActionStarted { action, .. } => {
                        self.current_action = action.clone();
                    }
                    Event::Custom { kind, .. }
                        if kind == "workflow_completed"
                            || kind == "workflow_failed"
                            || kind == "workflow_stopped" =>
                    {
                        // These are handled by check_controller_status via thread join
                    }
                    _ => {}
                }
            }
        }
    }

    fn stop_controller(&mut self) {
        if let Some(ref flag) = self.shutdown_flag {
            flag.request();
        }
        self.message = Some(("Stop requested...".to_string(), false));
    }

    fn check_controller_status(&mut self) {
        self.drain_events();

        if let Some(handle) = &self.controller_thread {
            if handle.is_finished() {
                self.controller_thread = None;
                self.shutdown_flag = None;
                self.event_receiver = None;

                if matches!(self.run_status, RunStatus::Running) {
                    // Determine outcome from the last phase
                    match self.tip_state.phase.as_str() {
                        "stable" => {
                            self.run_status = RunStatus::Completed;
                            self.message = Some((
                                "Tip preparation completed successfully".to_string(),
                                false,
                            ));
                        }
                        _ => {
                            // Thread finished -- could be stopped, cycle limit, timeout, or error
                            // We check shutdown first
                            self.run_status = RunStatus::Idle;
                            if self.message.is_none()
                                || self
                                    .message
                                    .as_ref()
                                    .map(|(m, _)| m == "Stop requested...")
                                    .unwrap_or(false)
                            {
                                self.message =
                                    Some(("Controller finished".to_string(), false));
                            }
                        }
                    }
                }
            }
        }
    }

    fn tip_shape_text(&self) -> &str {
        if self.tip_state.phase == "stable" {
            "Stable"
        } else if self.tip_state.is_sharp {
            "Sharp"
        } else if self.tip_state.cycle > 0 {
            "Blunt"
        } else {
            "-"
        }
    }

    fn action_text(&self) -> &str {
        if self.current_action.is_empty() {
            match self.tip_state.phase.as_str() {
                "pulsing" => "Pulsing",
                "confirming" => "Confirming sharpness",
                "stability_check" => "Stability check",
                "stable" => "Completed",
                _ => "-",
            }
        } else {
            &self.current_action
        }
    }

    fn status_text(&self) -> &str {
        match &self.run_status {
            RunStatus::Idle => "Ready",
            RunStatus::Running => "Running",
            RunStatus::Completed => "Completed",
            RunStatus::Error(_) => "Error",
        }
    }

    fn elapsed_text(&self) -> String {
        match self.start_time {
            Some(start) if self.is_running() => {
                let elapsed = start.elapsed().as_secs();
                format!("{:.0}s", elapsed)
            }
            _ => "-".to_string(),
        }
    }

    fn render_control_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Left panel: Status and controls
            ui.vertical(|ui| {
                ui.set_min_width(280.0);

                // Status display
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    egui::Grid::new("status_grid")
                        .num_columns(2)
                        .spacing([20.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Status:");
                            let status_color = match &self.run_status {
                                RunStatus::Running => egui::Color32::YELLOW,
                                RunStatus::Completed => egui::Color32::GREEN,
                                RunStatus::Error(_) => egui::Color32::RED,
                                RunStatus::Idle => egui::Color32::GRAY,
                            };
                            ui.colored_label(status_color, self.status_text());
                            ui.end_row();

                            ui.label("Current Action:");
                            ui.label(self.action_text());
                            ui.end_row();

                            ui.label("Tip Shape:");
                            let shape_color = match self.tip_shape_text() {
                                "Blunt" => egui::Color32::RED,
                                "Sharp" => egui::Color32::YELLOW,
                                "Stable" => egui::Color32::GREEN,
                                _ => egui::Color32::GRAY,
                            };
                            ui.colored_label(shape_color, self.tip_shape_text());
                            ui.end_row();

                            ui.label("Phase:");
                            ui.label(if self.tip_state.phase.is_empty() {
                                "-"
                            } else {
                                &self.tip_state.phase
                            });
                            ui.end_row();

                            ui.label("Cycle Count:");
                            if self.tip_state.cycle > 0 {
                                ui.label(self.tip_state.cycle.to_string());
                            } else {
                                ui.label("-");
                            }
                            ui.end_row();

                            ui.label("Freq Shift:");
                            ui.label(
                                self.tip_state
                                    .freq_shift
                                    .map(|f| format!("{:.2} Hz", f))
                                    .unwrap_or_else(|| "-".to_string()),
                            );
                            ui.end_row();

                            ui.label("Pulse Voltage:");
                            if self.tip_state.cycle > 0 {
                                ui.label(format!("{:.2} V", self.tip_state.pulse_voltage));
                            } else {
                                ui.label("-");
                            }
                            ui.end_row();

                            ui.label("Elapsed:");
                            ui.label(self.elapsed_text());
                            ui.end_row();
                        });
                });

                ui.add_space(10.0);

                // Message display
                if let Some((ref msg, is_error)) = self.message {
                    if is_error {
                        ui.colored_label(egui::Color32::RED, msg);
                    } else {
                        ui.colored_label(egui::Color32::GREEN, msg);
                    }
                    ui.add_space(5.0);
                }

                // Control buttons
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.is_running(), egui::Button::new("Start"))
                        .clicked()
                    {
                        self.message = None;
                        self.start_controller();
                    }

                    if ui
                        .add_enabled(self.is_running(), egui::Button::new("Stop"))
                        .clicked()
                    {
                        self.stop_controller();
                    }
                });
            });

            ui.add_space(10.0);

            // Right panel: Log window
            ui.vertical(|ui| {
                ui.set_min_width(300.0);
                ui.label("Activity Log");
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(400.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            ui.set_min_width(280.0);
                            for msg in &self.log_messages {
                                ui.label(egui::RichText::new(msg).monospace().size(11.0));
                            }
                            if self.log_messages.is_empty() {
                                ui.colored_label(egui::Color32::GRAY, "No activity yet");
                            }
                        });
                });

                ui.add_space(5.0);
                if ui.button("Clear Log").clicked() {
                    self.log_messages.clear();
                }
            });
        });

        // Freq shift time-series plot
        ui.add_space(10.0);
        ui.label("Freq Shift History");

        let fs_points: PlotPoints = self
            .freq_shift_history
            .iter()
            .map(|dp| [dp.time_s, dp.value])
            .collect();
        let fs_line = Line::new("Freq Shift (Hz)", fs_points)
            .color(egui::Color32::LIGHT_BLUE);

        Plot::new("freq_shift_plot")
            .height(120.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .x_axis_label("Time (s)")
            .y_axis_label("Hz")
            .show(ui, |plot_ui| {
                plot_ui.line(fs_line);

                // Draw sharp bounds as horizontal lines
                if let Some((lower, upper)) = self.sharp_bounds {
                    plot_ui.hline(
                        HLine::new("Lower bound", lower)
                            .color(egui::Color32::from_rgba_unmultiplied(0, 255, 0, 80))
                            .style(egui_plot::LineStyle::Dashed { length: 5.0 }),
                    );
                    plot_ui.hline(
                        HLine::new("Upper bound", upper)
                            .color(egui::Color32::from_rgba_unmultiplied(0, 255, 0, 80))
                            .style(egui_plot::LineStyle::Dashed { length: 5.0 }),
                    );
                }
            });

        // Voltage time-series plot
        ui.add_space(5.0);
        ui.label("Pulse Voltage History");

        let v_points: PlotPoints = self
            .voltage_history
            .iter()
            .map(|dp| [dp.time_s, dp.value])
            .collect();
        let v_line = Line::new("Pulse Voltage (V)", v_points)
            .color(egui::Color32::from_rgb(255, 165, 0));

        Plot::new("voltage_plot")
            .height(100.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .x_axis_label("Time (s)")
            .y_axis_label("V")
            .show(ui, |plot_ui| {
                plot_ui.line(v_line);
            });
    }

    fn render_configuration_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Load/Save Section
            ui.heading("Load / Save Configuration");
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Load from:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.load_path).desired_width(300.0),
                    );
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("TOML", &["toml"])
                            .pick_file()
                        {
                            self.load_path = path.display().to_string();
                        }
                    }
                    if ui
                        .add_enabled(!self.load_path.is_empty(), egui::Button::new("Load"))
                        .clicked()
                    {
                        self.load_config_from_file();
                    }
                });

                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Save to:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.save_path).desired_width(300.0),
                    );
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("TOML", &["toml"])
                            .save_file()
                        {
                            self.save_path = path.display().to_string();
                        }
                    }
                    if ui
                        .add_enabled(!self.save_path.is_empty(), egui::Button::new("Save"))
                        .clicked()
                    {
                        self.save_config_to_file();
                    }
                });
            });

            ui.add_space(10.0);

            // Connection Settings
            ui.heading("Connection");
            egui::Frame::group(ui.style()).show(ui, |ui| {
                egui::Grid::new("connection_grid")
                    .num_columns(2)
                    .spacing([20.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Host IP:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.host_ip)
                                .desired_width(150.0),
                        );
                        ui.end_row();

                        ui.label("Control Port:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.control_port)
                                .desired_width(80.0),
                        );
                        ui.end_row();

                        ui.label("Data Port:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.data_port)
                                .desired_width(80.0),
                        );
                        ui.end_row();

                        ui.label("Sample Rate (Hz):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.sample_rate)
                                .desired_width(80.0),
                        );
                        ui.end_row();

                        ui.label("Stable Signal Samples:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.stable_signal_samples)
                                .desired_width(80.0),
                        );
                        ui.end_row();

                        ui.label("Layout File:");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.layout_file)
                                    .desired_width(200.0),
                            );
                            if ui.button("...").clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Layout", &["lyt"])
                                    .pick_file()
                                {
                                    self.config.layout_file = path.display().to_string();
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("Settings File:");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.settings_file)
                                    .desired_width(200.0),
                            );
                            if ui.button("...").clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Settings", &["ini"])
                                    .pick_file()
                                {
                                    self.config.settings_file = path.display().to_string();
                                }
                            }
                        });
                        ui.end_row();
                    });
            });

            ui.add_space(10.0);

            // Tip Prep Settings
            ui.heading("Tip Preparation");
            egui::Frame::group(ui.style()).show(ui, |ui| {
                egui::Grid::new("tip_prep_grid")
                    .num_columns(2)
                    .spacing([20.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Sharp Tip Bounds (Hz):");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.sharp_tip_lower)
                                    .desired_width(60.0),
                            );
                            ui.label("to");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.sharp_tip_upper)
                                    .desired_width(60.0),
                            );
                        });
                        ui.end_row();

                        ui.label("Max Cycles:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.max_cycles)
                                .desired_width(80.0),
                        );
                        ui.end_row();

                        ui.label("Max Duration (s):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.max_duration_secs)
                                .desired_width(80.0),
                        );
                        ui.end_row();

                        ui.label("Initial Bias (mV):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.initial_bias_v)
                                .desired_width(80.0),
                        );
                        ui.end_row();

                        ui.label("Initial Z Setpoint (pA):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.initial_z_setpoint_pa)
                                .desired_width(80.0),
                        );
                        ui.end_row();

                        ui.label("Safe Tip Threshold (pA):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.safe_tip_threshold_pa)
                                .desired_width(80.0),
                        );
                        ui.end_row();
                    });
            });

            ui.add_space(10.0);

            // Pulse Method Settings
            ui.heading("Pulse Method");
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Method:");
                    ui.selectable_value(
                        &mut self.config.pulse_method_type,
                        PulseMethodType::Fixed,
                        "Fixed",
                    );
                    ui.selectable_value(
                        &mut self.config.pulse_method_type,
                        PulseMethodType::Stepping,
                        "Stepping",
                    );
                    ui.selectable_value(
                        &mut self.config.pulse_method_type,
                        PulseMethodType::Linear,
                        "Linear",
                    );
                });

                ui.add_space(5.0);

                egui::Grid::new("pulse_grid")
                    .num_columns(2)
                    .spacing([20.0, 4.0])
                    .show(ui, |ui| {
                        match self.config.pulse_method_type {
                            PulseMethodType::Fixed => {
                                ui.label("Voltage (V):");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.config.pulse_voltage)
                                        .desired_width(60.0),
                                );
                                ui.end_row();
                            }
                            PulseMethodType::Stepping => {
                                ui.label("Voltage Range (V):");
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.config.pulse_voltage_min,
                                        )
                                        .desired_width(60.0),
                                    );
                                    ui.label("to");
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.config.pulse_voltage_max,
                                        )
                                        .desired_width(60.0),
                                    );
                                });
                                ui.end_row();

                                ui.label("Voltage Steps:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.config.voltage_steps)
                                        .desired_width(60.0),
                                );
                                ui.end_row();

                                ui.label("Cycles Before Step:");
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut self.config.cycles_before_step,
                                    )
                                    .desired_width(60.0),
                                );
                                ui.end_row();

                                ui.label("Threshold Value (Hz):");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.config.threshold_value)
                                        .desired_width(60.0),
                                );
                                ui.end_row();
                            }
                            PulseMethodType::Linear => {
                                ui.label("Voltage Range (V):");
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.config.pulse_voltage_min,
                                        )
                                        .desired_width(60.0),
                                    );
                                    ui.label("to");
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.config.pulse_voltage_max,
                                        )
                                        .desired_width(60.0),
                                    );
                                });
                                ui.end_row();

                                ui.label("Linear Clamp (Hz):");
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.config.linear_clamp_min,
                                        )
                                        .desired_width(60.0),
                                    );
                                    ui.label("to");
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.config.linear_clamp_max,
                                        )
                                        .desired_width(60.0),
                                    );
                                });
                                ui.end_row();
                            }
                        }

                        ui.label("Polarity:");
                        ui.horizontal(|ui| {
                            ui.selectable_value(
                                &mut self.config.pulse_polarity,
                                PolaritySign::Positive,
                                "Positive",
                            );
                            ui.selectable_value(
                                &mut self.config.pulse_polarity,
                                PolaritySign::Negative,
                                "Negative",
                            );
                        });
                        ui.end_row();

                        ui.label("Random Polarity Switch:");
                        ui.checkbox(&mut self.config.random_polarity_enabled, "Enabled");
                        ui.end_row();

                        if self.config.random_polarity_enabled {
                            ui.label("Switch Every N Pulses:");
                            ui.add(
                                egui::TextEdit::singleline(
                                    &mut self.config.random_polarity_switch_every,
                                )
                                .desired_width(60.0),
                            );
                            ui.end_row();
                        }
                    });
            });

            ui.add_space(10.0);

            // Stability Settings
            ui.heading("Stability Check");
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.checkbox(&mut self.config.check_stability, "Enable Stability Check");

                if self.config.check_stability {
                    ui.add_space(5.0);

                    egui::Grid::new("stability_grid")
                        .num_columns(2)
                        .spacing([20.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Allowed Change (Hz):");
                            ui.add(
                                egui::TextEdit::singleline(
                                    &mut self.config.stable_tip_allowed_change,
                                )
                                .desired_width(60.0),
                            );
                            ui.end_row();

                            ui.label("Bias Range (V):");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut self.config.bias_range_lower,
                                    )
                                    .desired_width(60.0),
                                );
                                ui.label("to");
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut self.config.bias_range_upper,
                                    )
                                    .desired_width(60.0),
                                );
                            });
                            ui.end_row();

                            ui.label("Bias Steps:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.bias_steps)
                                    .desired_width(80.0),
                            );
                            ui.end_row();

                            ui.label("Step Period (ms):");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.step_period_ms)
                                    .desired_width(80.0),
                            );
                            ui.end_row();

                            ui.label("Max Duration (s):");
                            ui.add(
                                egui::TextEdit::singleline(
                                    &mut self.config.stability_max_duration,
                                )
                                .desired_width(80.0),
                            );
                            ui.end_row();

                            ui.label("Polarity Mode:");
                            ui.horizontal(|ui| {
                                ui.selectable_value(
                                    &mut self.config.polarity_mode,
                                    BiasSweepPolarity::Both,
                                    "Both",
                                );
                                ui.selectable_value(
                                    &mut self.config.polarity_mode,
                                    BiasSweepPolarity::Positive,
                                    "Positive",
                                );
                                ui.selectable_value(
                                    &mut self.config.polarity_mode,
                                    BiasSweepPolarity::Negative,
                                    "Negative",
                                );
                            });
                            ui.end_row();

                            ui.label("Scan Speed (nm/s):");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.scan_speed_nm_s)
                                    .desired_width(80.0),
                            );
                            ui.end_row();
                        });
                }
            });

            ui.add_space(10.0);

            // Logging Settings
            ui.heading("Logging");
            egui::Frame::group(ui.style()).show(ui, |ui| {
                egui::Grid::new("logging_grid")
                    .num_columns(2)
                    .spacing([20.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Enable Logging:");
                        ui.checkbox(&mut self.config.logging_enabled, "");
                        ui.end_row();

                        ui.label("Output Path:");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(
                                    &mut self.config.logging_output_path,
                                )
                                .desired_width(200.0),
                            );
                            if ui.button("...").clicked() {
                                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                    self.config.logging_output_path =
                                        path.display().to_string();
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("Verbosity:");
                        ui.horizontal(|ui| {
                            let verbosity = &mut self.config.verbosity;
                            ui.selectable_value(verbosity, "error".to_string(), "Error");
                            ui.selectable_value(verbosity, "warn".to_string(), "Warn");
                            ui.selectable_value(verbosity, "info".to_string(), "Info");
                            ui.selectable_value(verbosity, "debug".to_string(), "Debug");
                        });
                        ui.end_row();
                    });
            });

            ui.add_space(10.0);

            // TCP Channel Mapping
            ui.heading("TCP Channel Mapping");
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label("Map Nanonis signal indices to TCP channel indices:");
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Nanonis Index");
                    ui.add_space(20.0);
                    ui.label("TCP Channel");
                    ui.add_space(20.0);
                    ui.label("");
                });

                let mut to_remove: Option<usize> = None;
                for (idx, mapping) in
                    self.config.tcp_channel_mappings.iter_mut().enumerate()
                {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut mapping.nanonis_index)
                                .desired_width(80.0),
                        );
                        ui.add_space(20.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut mapping.tcp_channel)
                                .desired_width(80.0),
                        );
                        ui.add_space(20.0);
                        if ui.button("Remove").clicked() {
                            to_remove = Some(idx);
                        }
                    });
                }

                if let Some(idx) = to_remove {
                    self.config.tcp_channel_mappings.remove(idx);
                }

                ui.add_space(5.0);

                if ui.button("Add Mapping").clicked() {
                    self.config
                        .tcp_channel_mappings
                        .push(EditableTcpMapping::default());
                }
            });

            ui.add_space(10.0);

            // Message display
            if let Some((ref msg, is_error)) = self.message {
                if is_error {
                    ui.colored_label(egui::Color32::RED, msg);
                } else {
                    ui.colored_label(egui::Color32::GREEN, msg);
                }
                ui.add_space(5.0);
            }

            // Reset button
            if ui.button("Reset to Defaults").clicked() {
                self.config = EditableConfig::default();
                self.message = Some(("Reset to defaults".to_string(), false));
            }
        });
    }
}

// ============================================================================
// V2 Controller Runner (background thread)
// ============================================================================

fn run_controller(
    config: AppConfig,
    shutdown: ShutdownFlag,
    event_tx: Sender<Event>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Setting up V2 controller...");

    // Connect to Nanonis
    let client = rusty_tip::NanonisClient::builder()
        .address(&config.nanonis.host_ip)
        .port(config.nanonis.control_ports[0])
        .build()?;
    let setup = NanonisSetupConfig {
        layout_file: config.nanonis.layout_file.clone(),
        settings_file: config.nanonis.settings_file.clone(),
        safe_tip_threshold_a: config.tip_prep.safe_tip_threshold as f64,
        ..Default::default()
    };
    let mut controller = NanonisController::new(client, setup);
    info!("Connected to Nanonis system");

    // Build signal registry
    let signal_names = controller.signal_names()?;
    let registry = build_signal_registry(&signal_names, &config);
    let freq_shift_signal = registry
        .get_by_name("freq shift")
        .ok_or("Frequency shift signal not found in registry")?;
    let freq_shift_index = freq_shift_signal.index as u32;

    // Setup TCP data stream
    setup_tcp_stream(&mut controller, &registry, &config)?;

    // Setup event bus with ChannelForwarder for GUI
    let mut events = EventBus::new();
    events.add_observer(Box::new(ConsoleLogger));
    events.add_observer(Box::new(ChannelForwarder::new(event_tx)));

    if config.experiment_logging.enabled {
        let log_dir = std::path::PathBuf::from(&config.experiment_logging.output_path);
        std::fs::create_dir_all(&log_dir)?;
        let filename = format!(
            "tip_prep_{}.jsonl",
            chrono::Utc::now().format("%Y%m%d_%H%M%S")
        );
        let file = std::fs::File::create(log_dir.join(filename))?;
        events.add_observer(Box::new(FileLogger::new(file)));
    }

    events.add_observer(Box::new(EventAccumulator::new(500)));

    // Run tip preparation
    let result = run_tip_prep(
        Box::new(controller),
        &events,
        &shutdown,
        &config,
        freq_shift_index,
    );

    // Convert ShutdownRequested to StoppedByUser
    let result = match result {
        Err(e)
            if e.downcast_ref::<SpmError>()
                .is_some_and(|e| matches!(e, SpmError::ShutdownRequested)) =>
        {
            Ok(Outcome::StoppedByUser)
        }
        other => other,
    };

    match &result {
        Ok(Outcome::Completed) => info!("Tip preparation completed successfully!"),
        Ok(Outcome::StoppedByUser) => info!("Tip preparation stopped by user"),
        Ok(Outcome::CycleLimit(n)) => error!("Max cycles ({}) exceeded", n),
        Ok(Outcome::TimedOut(d)) => {
            error!("Max duration ({:.0}s) exceeded", d.as_secs_f64())
        }
        Err(e) => error!("Tip preparation failed: {}", e),
    }

    Ok(())
}

fn build_signal_registry(signal_names: &[String], config: &AppConfig) -> SignalRegistry {
    let mut builder = SignalRegistry::builder().with_standard_map();

    if let Some(ref mappings) = config.tcp_channel_mapping {
        let tcp_map: Vec<(u8, u8)> = mappings
            .iter()
            .map(|m| (m.nanonis_index, m.tcp_channel))
            .collect();
        builder = builder.add_tcp_map(&tcp_map);
    }

    builder
        .from_signal_names(signal_names)
        .create_aliases()
        .build()
}

fn setup_tcp_stream(
    controller: &mut NanonisController,
    registry: &SignalRegistry,
    config: &AppConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tcp_signals = registry.tcp_signals();
    if tcp_signals.is_empty() {
        log::warn!("No TCP channel mappings found - falling back to polling");
        return Ok(());
    }

    let mut tcp_channels: Vec<i32> = tcp_signals
        .iter()
        .filter_map(|s| s.tcp_channel.map(|ch| ch as i32))
        .collect();
    tcp_channels.sort();
    tcp_channels.dedup();

    let tcp_to_position: HashMap<u8, usize> = tcp_channels
        .iter()
        .enumerate()
        .map(|(pos, &ch)| (ch as u8, pos))
        .collect();

    let mut signal_mapping: HashMap<u32, usize> = HashMap::new();
    for signal in &tcp_signals {
        if let Some(tcp_ch) = signal.tcp_channel {
            if let Some(&position) = tcp_to_position.get(&tcp_ch) {
                signal_mapping.insert(signal.index as u32, position);
            }
        }
    }

    let oversampling = config.data_acquisition.sample_rate as i32;
    controller.data_stream_configure(&tcp_channels, oversampling)?;
    controller.set_channel_mapping(signal_mapping);

    let buffer_size = 10_000;
    controller.start_tcp_reader(
        &config.nanonis.host_ip,
        config.data_acquisition.data_port,
        buffer_size,
    )?;

    let _ = controller.data_stream_stop();
    std::thread::sleep(Duration::from_millis(200));
    controller.data_stream_start()?;
    info!("TCP data stream started");

    Ok(())
}

impl eframe::App for TipPrepApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.check_controller_status();

        ctx.request_repaint_after(Duration::from_millis(100));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Rusty Tip Preparation");

            ui.add_space(5.0);

            // Tab bar
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.current_tab == Tab::Control, "Control")
                    .clicked()
                {
                    self.current_tab = Tab::Control;
                }
                if ui
                    .selectable_label(self.current_tab == Tab::Configuration, "Configuration")
                    .clicked()
                {
                    self.current_tab = Tab::Configuration;
                }
            });

            ui.separator();
            ui.add_space(5.0);

            match self.current_tab {
                Tab::Control => self.render_control_tab(ui),
                Tab::Configuration => self.render_configuration_tab(ui),
            }
        });
    }
}
