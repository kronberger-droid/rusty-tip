use chrono::Utc;
use env_logger::Env;
use log::{error, info, LevelFilter};
use rusty_tip::{
    load_config_or_default,
    tip_prep::{PulseMethod, TipControllerConfig},
    ActionDriver, SignalIndex, TCPReaderConfig, TipController,
};
use std::{env, fs, path::PathBuf, time::Duration};

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
    let driver = ActionDriver::builder(
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
    )
    .build()?;

    info!("Connected to Nanonis system");

    // Try different variations of the name
    let freq_shift_signal = SignalIndex::from_name("freq shift", &driver)?;
    let pulse_method = PulseMethod::stepping_fixed_threshold((2.0, 6.0), 4, 2, 1.0);
    // Create tip controller configuration with registry-based signal
    let config = TipControllerConfig {
        freq_shift_index: freq_shift_signal,
        sharp_tip_bounds: (-2.0, 0.0),
        pulse_method,
        ..Default::default()
    };
		
    info!("Using signal index: {}", freq_shift_signal.0);

    // Create pulse method from config
    let pulse_method = match app_config.pulse_method {
        rusty_tip::config::PulseMethodConfig::Fixed { ref pulse_voltage } => {
            let voltage = pulse_voltage.get(0).copied().unwrap_or(4.0);
            info!("Using fixed pulse method with voltage: {:.2}V", voltage);
            PulseMethod::Fixed(voltage)
        }
        rusty_tip::config::PulseMethodConfig::Stepping {
            voltage_bounds,
            voltage_steps,
            cycles_before_step,
            threshold_value,
            ..
        } => {
            info!(
                "Using stepping pulse method: {:.2}V to {:.2}V in {} steps",
                voltage_bounds[0], voltage_bounds[1], voltage_steps
            );
            PulseMethod::stepping_fixed_threshold(
                (voltage_bounds[0], voltage_bounds[1]),
                voltage_steps,
                cycles_before_step,
                threshold_value,
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
        check_stability: true,
        max_cycles: Some(10000),
        max_duration: Some(Duration::from_secs(12000)),
    };

    info!(
        "Sharp tip bounds: {:.2} to {:.2}",
        tip_config.sharp_tip_bounds.0, tip_config.sharp_tip_bounds.1
    );
    info!(
        "Stable tip allowed change: {:.3}",
        tip_config.allowed_change_for_stable
    );

    // Windows: Wait for user confirmation before proceeding
    #[cfg(windows)]
    {
        println!();
        println!("Press Enter to start tip preparation (or Ctrl+C to cancel)...");
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
    }

    // Create and run controller
    let mut controller = TipController::new(driver, tip_config);

    info!("Starting tip preparation process...");
    match controller.run() {
        Ok(()) => {
            info!("✓ Tip preparation completed successfully!");

            #[cfg(windows)]
            {
                info!("Press Enter to exit...");
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
            }
        }
        Err(e) => {
            error!("✗ Tip preparation failed: {}", e);

            #[cfg(windows)]
            {
                error!("Press Enter to exit...");
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
            }

            return Err(e.into());
        }
    }

    Ok(())
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
