mod config;
mod tip_prep;

use chrono::Utc;
use clap::Parser;
use env_logger::Env;
use log::{error, info, LevelFilter};
use rusty_tip::Signal;
use rusty_tip::{ActionDriver, TCPReaderConfig};
use std::{
    fs,
    io,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::config::{load_config_or_default, AppConfig};
use crate::tip_prep::{PulseMethod, TipController, TipControllerConfig};

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

/// Rusty Tip Preparation Tool
#[derive(Parser, Debug)]
#[command(name = "tip-prep")]
#[command(about = "Automated tip preparation for STM/AFM", long_about = None)]
struct Args {
    /// Path to configuration file (required)
    #[arg(short, long, value_name = "FILE", required = true)]
    config: PathBuf,

    /// Override log level (trace, debug, info, warn, error)
    #[arg(short, long, value_name = "LEVEL")]
    log_level: Option<String>,
}

/// Simple tip preparation demo - minimal configuration and straightforward execution
///
/// Usage:
///   cargo run --example simple_tip_prep
///   cargo run --example simple_tip_prep -- --config path/to/config.toml
///   cargo run --example simple_tip_prep -- --config path/to/config.toml --log-level debug
///
/// Build as executable:
///   cargo build --example simple_tip_prep --release
///   # Windows: target/release/examples/simple_tip_prep.exe
///   # Linux: target/release/examples/simple_tip_prep
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Windows-specific: Allocate console if launched from GUI
    #[cfg(windows)]
    ensure_console_allocated();

    // Parse arguments and load configuration
    let args = Args::parse();
    let config = load_config_or_default(Some(&args.config));

    // Initialize logging
    let log_level = args.log_level.unwrap_or(config.console.verbosity.clone());
    initialize_logging(&log_level)?;
    log_startup_info(&config, &args.config);

    // Setup hardware and signals
    let driver = setup_driver(&config)?;
    let freq_shift = setup_frequency_shift_signal(&driver)?;

    // Create and log tip controller configuration
    let tip_config = create_tip_controller_config(&config, freq_shift);
    log_tip_config(&tip_config);

    // Wait for user confirmation (Windows only)
    wait_for_user_confirmation()?;

    // Setup controller with graceful shutdown support
    let shutdown_flag = setup_shutdown_handler();
    let mut controller = TipController::new(driver, tip_config);
    controller.set_shutdown_flag(shutdown_flag.clone());

    // Run tip preparation
    run_and_report(controller, shutdown_flag)
}

// Helper Functions

/// Log pulse method configuration details
fn log_pulse_method(method: &PulseMethod) {
    match method {
        PulseMethod::Fixed {
            voltage,
            polarity,
            random_polarity_switch,
        } => {
            info!("Pulse method: Fixed ({:.2}V, {:?})", voltage, polarity);
            if let Some(switch) = random_polarity_switch {
                if switch.enabled {
                    info!(
                        "Random polarity switching: every {} pulses",
                        switch.switch_every_n_pulses
                    );
                } else {
                    info!("Random polarity switching: disabled");
                }
            }
        }
        PulseMethod::Stepping {
            voltage_bounds,
            voltage_steps,
            threshold_value,
            polarity,
            random_polarity_switch,
            ..
        } => {
            info!(
                "Pulse method: Stepping ({:.2}V to {:.2}V, {} steps, {:?})",
                voltage_bounds.0, voltage_bounds.1, voltage_steps, polarity
            );
            info!("Threshold value: {:.3}", threshold_value);
            if let Some(switch) = random_polarity_switch {
                if switch.enabled {
                    info!(
                        "Random polarity switching: every {} pulses",
                        switch.switch_every_n_pulses
                    );
                } else {
                    info!("Random polarity switching: disabled");
                }
            }
        }
        PulseMethod::Linear {
            voltage_bounds,
            linear_clamp,
            polarity,
            random_polarity_switch,
        } => {
            info!(
                "Pulse method: Linear (voltage: {:.2}V to {:.2}V, freq_shift range: {:.2} to {:.2} Hz, {:?})",
                voltage_bounds.0, voltage_bounds.1, linear_clamp.0, linear_clamp.1, polarity
            );
            if let Some(switch) = random_polarity_switch {
                if switch.enabled {
                    info!(
                        "Random polarity switching: every {} pulses",
                        switch.switch_every_n_pulses
                    );
                } else {
                    info!("Random polarity switching: disabled");
                }
            }
        }
    }
}

/// Log startup information
fn log_startup_info(config: &AppConfig, config_path: &PathBuf) {
    info!("=== Rusty Tip Preparation Tool ===");
    info!("Configuration: {}", config_path.display());
    info!(
        "Nanonis: {}:{}",
        config.nanonis.host_ip, config.nanonis.control_ports[0]
    );

    // Log layout and settings files from config
    if let Some(ref layout) = config.nanonis.layout_file {
        info!("Layout file: {}", layout);
    }
    if let Some(ref settings) = config.nanonis.settings_file {
        info!("Settings file: {}", settings);
    }
}

/// Log tip controller configuration
fn log_tip_config(config: &TipControllerConfig) {
    info!(
        "Sharp tip bounds: {:.2} to {:.2}",
        config.sharp_tip_bounds.0, config.sharp_tip_bounds.1
    );
    info!(
        "Stable tip allowed change: {:.3}",
        config.allowed_change_for_stable
    );
    info!("Check stability: {}", config.check_stability);
    info!(
        "Max cycles: {}",
        config
            .max_cycles
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unlimited".to_string())
    );
    info!(
        "Max duration: {}",
        config
            .max_duration
            .map(|d| format!("{} seconds", d.as_secs()))
            .unwrap_or_else(|| "unlimited".to_string())
    );

    // Log layout and settings files if specified
    if let Some(ref layout_file) = config.layout_file {
        info!("Layout file: {}", layout_file);
    }
    if let Some(ref settings_file) = config.settings_file {
        info!("Settings file: {}", settings_file);
    }

    log_pulse_method(&config.pulse_method);
}

/// Setup ActionDriver from configuration
fn setup_driver(
    config: &AppConfig,
) -> Result<ActionDriver, Box<dyn std::error::Error>> {
    let mut builder = ActionDriver::builder(
        &config.nanonis.host_ip,
        config.nanonis.control_ports[0],
    )
    .with_tcp_reader(TCPReaderConfig {
        stream_port: config.data_acquisition.data_port,
        oversampling: (2000 / config.data_acquisition.sample_rate) as i32,
        ..Default::default()
    })
    .with_action_logging(
        create_log_file_path(&config.experiment_logging.output_path)?,
        1000,
        config.experiment_logging.enabled,
    );

    // Apply custom TCP channel mapping from config if provided
    if let Some(ref tcp_mapping) = config.tcp_channel_mapping {
        let mapping: Vec<(u8, u8)> = tcp_mapping
            .iter()
            .map(|m| (m.nanonis_index, m.tcp_channel))
            .collect();
        info!("Custom TCP channel mapping: {} entries", mapping.len());
        builder = builder.with_custom_tcp_mapping(&mapping);
    }

    let driver = builder.build()?;
    info!("Connected to Nanonis system");

    Ok(driver)
}

/// Setup and validate frequency shift signal
fn setup_frequency_shift_signal(
    driver: &ActionDriver,
) -> Result<Signal, Box<dyn std::error::Error>> {
    // Look up frequency shift signal in registry
    let signal = driver
        .signal_registry()
        .get_by_name("freq shift")
        .ok_or("Frequency shift signal not found in registry")?;

    info!("Frequency shift signal: index {}", signal.index);

    // TCP mapping validation
    if let Some(tcp_ch) = signal.tcp_channel {
        info!("Frequency shift maps to TCP channel: {}", tcp_ch);
    } else {
        error!("WARNING: Frequency shift has no TCP mapping");
    }

    Ok(signal.clone())
}

/// Create TipControllerConfig from AppConfig
fn create_tip_controller_config(
    config: &AppConfig,
    freq_shift: Signal,
) -> TipControllerConfig {
    TipControllerConfig {
        freq_shift_signal: freq_shift,
        sharp_tip_bounds: (
            config.tip_prep.sharp_tip_bounds[0],
            config.tip_prep.sharp_tip_bounds[1],
        ),
        pulse_method: config.pulse_method.clone(),
        allowed_change_for_stable: config.tip_prep.stability.stable_tip_allowed_change,
        check_stability: config.tip_prep.stability.check_stability,
        max_cycles: config.tip_prep.max_cycles,
        max_duration: config
            .tip_prep
            .max_duration_secs
            .map(Duration::from_secs),
        stability_config: config.tip_prep.stability.clone(),
        layout_file: config.nanonis.layout_file.clone(),
        settings_file: config.nanonis.settings_file.clone(),
        initial_bias_v: config.tip_prep.initial_bias_v,
        initial_z_setpoint_a: config.tip_prep.initial_z_setpoint_a,
        safe_tip_threshold: config.tip_prep.safe_tip_threshold,
    }
}

/// Setup Ctrl+C handler for graceful shutdown
fn setup_shutdown_handler() -> Arc<AtomicBool> {
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = shutdown_flag.clone();

    ctrlc::set_handler(move || {
        info!("Ctrl+C received - initiating graceful shutdown...");
        shutdown_flag_clone.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl+C handler");

    shutdown_flag
}

/// Run tip preparation and report results
fn run_and_report(
    mut controller: TipController,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting tip preparation process...");

    let result = match controller.run() {
        Ok(()) => {
            if shutdown_flag.load(Ordering::SeqCst) {
                info!("✓ Tip preparation stopped by user");
            } else {
                info!("✓ Tip preparation completed successfully!");
            }
            Ok(())
        }
        Err(e) => {
            error!("✗ Tip preparation failed: {}", e);
            Err(e.into())
        }
    };

    info!("Cleaning up and shutting down...");
    drop(controller);
    info!("Cleanup complete");

    result
}

/// Wait for user confirmation before proceeding
fn wait_for_user_confirmation() -> Result<(), Box<dyn std::error::Error>> {
    println!();
    println!("Press Enter to start tip preparation (or Ctrl+C to cancel)...");
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(())
}

/// Initialize logging with configurable level
fn initialize_logging(
    log_level: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let level = match log_level.to_lowercase().as_str() {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "info" => LevelFilter::Info,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        _ => {
            eprintln!(
                "Warning: Invalid log level '{}', using 'info'",
                log_level
            );
            LevelFilter::Info
        }
    };

    env_logger::Builder::from_env(Env::default())
        .filter_level(level)
        .format_timestamp_millis()
        .init();

    Ok(())
}

fn create_log_file_path(
    log_path: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let history_dir = PathBuf::from(log_path);

    // Ensure directory exists
    fs::create_dir_all(&history_dir)?;

    // Create timestamped filename
    let filename =
        format!("tip_prep_{}.jsonl", Utc::now().format("%Y%m%d_%H%M%S"));
    let file_path = history_dir.join(filename);

    Ok(file_path)
}

/// Windows-specific: Allocate console if running from GUI
#[cfg(windows)]
fn ensure_console_allocated() {
    unsafe {
        // Try to allocate a new console
        if winapi::um::consoleapi::AllocConsole() != 0 {
            // Successfully allocated new console
            println!("Console allocated for tip preparation tool");
        }

        // Set console title
        let title = "Rusty Tip Preparation Tool";
        let wide_title: Vec<u16> = OsString::from(title)
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        winapi::um::wincon::SetConsoleTitleW(wide_title.as_ptr());

        // Enable ANSI escape sequences for colored output (Windows 10+)
        let stdout_handle = winapi::um::processenv::GetStdHandle(
            winapi::um::winbase::STD_OUTPUT_HANDLE,
        );
        if stdout_handle != winapi::um::handleapi::INVALID_HANDLE_VALUE {
            let mut mode: u32 = 0;
            if winapi::um::consoleapi::GetConsoleMode(stdout_handle, &mut mode)
                != 0
            {
                mode |= winapi::um::wincon::ENABLE_VIRTUAL_TERMINAL_PROCESSING;
                winapi::um::consoleapi::SetConsoleMode(stdout_handle, mode);
            }
        }
    }
}
