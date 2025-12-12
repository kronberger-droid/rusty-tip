use chrono::Utc;
use env_logger::Env;
use log::{error, info, LevelFilter};
use rusty_tip::{
    load_config_or_default,
    tip_prep::{PulseMethod, TipControllerConfig},
    ActionDriver, SignalIndex, TCPReaderConfig, TipController,
};
use std::{
    env, fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[cfg(windows)]
use std::io;

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

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

    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let (config_path, log_level_override) = parse_args(&args);

    // Load configuration (with fallback to defaults)
    let app_config = load_config_or_default(config_path.as_deref());

    // Initialize logging with configurable level
    let verbosity = log_level_override.unwrap_or(app_config.console.verbosity.clone());
    initialize_logging(&verbosity)?;

    info!("=== Rusty Tip Preparation Tool ===");
    info!(
        "Configuration loaded from: {:?}",
        config_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "defaults".to_string())
    );
    info!("Console verbosity: {}", verbosity);
    info!(
        "Nanonis host: {}:{}",
        app_config.nanonis.host_ip, app_config.nanonis.control_ports[0]
    );

    // Create driver from config
    let mut builder = ActionDriver::builder(
        &app_config.nanonis.host_ip,
        app_config.nanonis.control_ports[0],
    )
    .with_tcp_reader(TCPReaderConfig {
        stream_port: app_config.data_acquisition.data_port,
        oversampling: (2000 / app_config.data_acquisition.sample_rate) as i32,
        ..Default::default()
    })
    .with_action_logging(
        create_log_file_path(&app_config.experiment_logging.output_path)?,
        1000,
        app_config.experiment_logging.enabled,
    );

    // Apply custom TCP channel mapping from config if provided
    if let Some(ref tcp_mapping) = app_config.tcp_channel_mapping {
        let mapping: Vec<(u8, u8)> = tcp_mapping
            .iter()
            .map(|m| (m.nanonis_index, m.tcp_channel))
            .collect();
        info!(
            "Using custom TCP channel mapping from config with {} entries",
            mapping.len()
        );
        builder = builder.with_custom_tcp_mapping(&mapping);
    }

    let driver = builder.build()?;

    info!("Connected to Nanonis system");

    // Get frequency shift signal from Nanonis system
    let freq_shift_signal = SignalIndex::from_name("freq shift", &driver)?;
    info!(
        "Using signal index: {} (Nanonis index)",
        freq_shift_signal.0 .0
    );

    // Validate TCP mapping for frequency shift
    match driver.validate_tcp_signal(freq_shift_signal) {
        Ok(tcp_ch) => info!("Frequency shift signal maps to TCP channel: {}", tcp_ch),
        Err(e) => error!("WARNING: Frequency shift signal has no TCP mapping: {}", e),
    }

    // Create pulse method from config
    let pulse_method = match app_config.pulse_method {
        rusty_tip::config::PulseMethodConfig::Fixed {
            ref pulse_voltage,
            polarity,
            ref random_polarity_switch,
        } => {
            let voltage = pulse_voltage.first().copied().unwrap_or(4.0);
            let random_switch = random_polarity_switch.as_ref().and_then(|rps| {
                if rps.enabled {
                    Some(rusty_tip::tip_prep::RandomPolaritySwitch {
                        switch_every_n_pulses: rps.switch_every_n_pulses,
                    })
                } else {
                    None
                }
            });

            info!(
                "Using fixed pulse method with voltage: {:.2}V, polarity: {:?}",
                voltage, polarity
            );
            if let Some(ref switch) = random_switch {
                info!(
                    "Random polarity switching enabled: every {} pulses",
                    switch.switch_every_n_pulses
                );
            }

            PulseMethod::Fixed {
                voltage,
                polarity: match polarity {
                    rusty_tip::config::PolaritySign::Positive => {
                        rusty_tip::tip_prep::PolaritySign::Positive
                    }
                    rusty_tip::config::PolaritySign::Negative => {
                        rusty_tip::tip_prep::PolaritySign::Negative
                    }
                },
                random_switch,
            }
        }
        rusty_tip::config::PulseMethodConfig::Stepping {
            voltage_bounds,
            voltage_steps,
            cycles_before_step,
            threshold_value,
            polarity,
            ref random_polarity_switch,
            ..
        } => {
            let random_switch = random_polarity_switch.as_ref().and_then(|rps| {
                if rps.enabled {
                    Some(rusty_tip::tip_prep::RandomPolaritySwitch {
                        switch_every_n_pulses: rps.switch_every_n_pulses,
                    })
                } else {
                    None
                }
            });

            info!(
                "Using stepping pulse method: {:.2}V to {:.2}V in {} steps, polarity: {:?}",
                voltage_bounds[0], voltage_bounds[1], voltage_steps, polarity
            );
            if let Some(ref switch) = random_switch {
                info!(
                    "Random polarity switching enabled: every {} pulses",
                    switch.switch_every_n_pulses
                );
            }

            PulseMethod::stepping_fixed_threshold(
                (voltage_bounds[0], voltage_bounds[1]),
                voltage_steps,
                cycles_before_step,
                threshold_value,
                match polarity {
                    rusty_tip::config::PolaritySign::Positive => {
                        rusty_tip::tip_prep::PolaritySign::Positive
                    }
                    rusty_tip::config::PolaritySign::Negative => {
                        rusty_tip::tip_prep::PolaritySign::Negative
                    }
                },
                random_switch,
            )
        }
    };

    // Create tip controller configuration from app config
    let tip_config = TipControllerConfig {
        freq_shift_index: freq_shift_signal,
        sharp_tip_bounds: (
            app_config.tip_prep.sharp_tip_bounds[0],
            app_config.tip_prep.sharp_tip_bounds[1],
        ),
        pulse_method,
        allowed_change_for_stable: app_config.tip_prep.stable_tip_allowed_change,
        check_stability: app_config.tip_prep.check_stability,
        max_cycles: app_config.tip_prep.max_cycles,
        max_duration: app_config
            .tip_prep
            .max_duration_secs
            .map(Duration::from_secs),
    };

    info!(
        "Sharp tip bounds: {:.2} to {:.2}",
        tip_config.sharp_tip_bounds.0, tip_config.sharp_tip_bounds.1
    );
    info!(
        "Stable tip allowed change: {:.3}",
        tip_config.allowed_change_for_stable
    );
    info!("Check stability: {}", tip_config.check_stability);
    info!(
        "Max cycles: {}",
        tip_config
            .max_cycles
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unlimited".to_string())
    );
    info!(
        "Max duration: {}",
        tip_config
            .max_duration
            .map(|d| format!("{} seconds", d.as_secs()))
            .unwrap_or_else(|| "unlimited".to_string())
    );

    // Windows: Wait for user confirmation before proceeding
    #[cfg(windows)]
    {
        println!();
        println!("Press Enter to start tip preparation (or Ctrl+C to cancel)...");
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
    }

    // Set up Ctrl+C handler for graceful shutdown
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = shutdown_flag.clone();

    ctrlc::set_handler(move || {
        info!("Ctrl+C received - initiating graceful shutdown...");
        shutdown_flag_clone.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl+C handler");

    // Create and run controller
    let mut controller = TipController::new(driver, tip_config);
    controller.set_shutdown_flag(shutdown_flag.clone());

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

    // Explicitly clean up - ensures ActionDriver Drop is called properly
    info!("Cleaning up and shutting down...");
    drop(controller); // This will drop the TipController, which contains the ActionDriver
    info!("Cleanup complete");

    result
}

/// Parse command line arguments
fn parse_args(args: &[String]) -> (Option<PathBuf>, Option<String>) {
    let mut config_path = None;
    let mut log_level = None;
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                if i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    eprintln!("Error: --config requires a file path");
                    std::process::exit(1);
                }
            }
            "--log-level" => {
                if i + 1 < args.len() {
                    log_level = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!(
                        "Error: --log-level requires a level (trace, debug, info, warn, error)"
                    );
                    std::process::exit(1);
                }
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => {
                eprintln!("Error: Unknown argument '{}'", args[i]);
                print_help();
                std::process::exit(1);
            }
        }
    }

    (config_path, log_level)
}

/// Print help information
fn print_help() {
    println!("Rusty Tip Preparation Tool");
    println!();
    println!("USAGE:");
    println!("    simple_tip_prep [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    --config <FILE>       Use custom configuration file");
    println!("    --log-level <LEVEL>   Override log level (trace, debug, info, warn, error)");
    println!("    -h, --help           Print help information");
    println!();
    println!("EXAMPLES:");
    println!("    simple_tip_prep");
    println!("    simple_tip_prep --config my_config.toml");
    println!("    simple_tip_prep --config my_config.toml --log-level debug");
    println!();
    println!("ENVIRONMENT VARIABLES:");
    println!("    RUSTY_TIP__LOGGING__LOG_LEVEL     Override log level");
    println!("    RUSTY_TIP__NANONIS__HOST_IP       Override Nanonis host IP");
}

/// Initialize logging with configurable level
fn initialize_logging(log_level: &str) -> Result<(), Box<dyn std::error::Error>> {
    let level = match log_level.to_lowercase().as_str() {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "info" => LevelFilter::Info,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        _ => {
            eprintln!("Warning: Invalid log level '{}', using 'info'", log_level);
            LevelFilter::Info
        }
    };

    env_logger::Builder::from_env(Env::default())
        .filter_level(level)
        .format_timestamp_millis()
        .init();

    Ok(())
}

fn create_log_file_path(log_path: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let history_dir = PathBuf::from(log_path);

    // Ensure directory exists
    fs::create_dir_all(&history_dir)?;

    // Create timestamped filename
    let filename = format!("tip_prep_{}.jsonl", Utc::now().format("%Y%m%d_%H%M%S"));
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
        let stdout_handle =
            winapi::um::processenv::GetStdHandle(winapi::um::winbase::STD_OUTPUT_HANDLE);
        if stdout_handle != winapi::um::handleapi::INVALID_HANDLE_VALUE {
            let mut mode: u32 = 0;
            if winapi::um::consoleapi::GetConsoleMode(stdout_handle, &mut mode) != 0 {
                mode |= winapi::um::wincon::ENABLE_VIRTUAL_TERMINAL_PROCESSING;
                winapi::um::consoleapi::SetConsoleMode(stdout_handle, mode);
            }
        }
    }
}
