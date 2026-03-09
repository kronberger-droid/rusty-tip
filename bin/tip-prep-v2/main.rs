mod config;

use chrono::Utc;
use clap::Parser;
use env_logger::Env;
use log::{error, info, LevelFilter};
use std::{
    fs,
    io,
    path::PathBuf,
    time::Duration,
};

use rusty_tip::action::{ActionContext, ActionOutput};
use rusty_tip::action::bias::{BiasPulse, SetBias};
use rusty_tip::action::signals::ReadSignal;
use rusty_tip::action::z_controller::{AutoApproach, SetZSetpoint, Withdraw};
use rusty_tip::action::util::Wait;
use rusty_tip::action::Action;
use rusty_tip::action::DataStore;
use rusty_tip::event::{ConsoleLogger, EventAccumulator, EventBus, FileLogger};
use rusty_tip::nanonis_controller::NanonisController;
use rusty_tip::workflow::ShutdownFlag;

use crate::config::{load_config, AppConfig};

/// Rusty Tip Preparation Tool (v2)
#[derive(Parser, Debug)]
#[command(name = "tip-prep-v2")]
#[command(about = "Automated tip preparation for STM/AFM (v2)", long_about = None)]
struct Args {
    /// Path to configuration file (required)
    #[arg(short, long, value_name = "FILE", required = true)]
    config: PathBuf,

    /// Override log level (trace, debug, info, warn, error)
    #[arg(short, long, value_name = "LEVEL")]
    log_level: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    ensure_console_allocated();

    let args = Args::parse();
    let config = load_config(&args.config)?;

    let log_level = args
        .log_level
        .unwrap_or(config.console.verbosity.clone());
    initialize_logging(&log_level)?;

    info!("=== Rusty Tip Preparation Tool (v2) ===");
    info!("Configuration: {}", args.config.display());
    info!(
        "Nanonis: {}:{}",
        config.nanonis.host_ip, config.nanonis.control_ports[0]
    );

    // Connect to hardware
    let client = rusty_tip::NanonisClient::builder()
        .address(&config.nanonis.host_ip)
        .port(config.nanonis.control_ports[0])
        .build()?;
    let controller = NanonisController::new(client);
    info!("Connected to Nanonis system");

    // Setup event bus
    let events = setup_event_bus(&config)?;

    // Setup shutdown handler
    let shutdown = setup_shutdown_handler();

    // Wait for user confirmation
    wait_for_user_confirmation()?;

    // Run tip preparation
    let result = run_tip_prep(controller, &events, &shutdown, &config);

    match &result {
        Ok(Outcome::Completed) => info!("Tip preparation completed successfully!"),
        Ok(Outcome::StoppedByUser) => info!("Tip preparation stopped by user"),
        Ok(Outcome::CycleLimit(n)) => error!("Max cycles ({}) exceeded", n),
        Ok(Outcome::TimedOut(d)) => {
            error!("Max duration ({:.0}s) exceeded", d.as_secs_f64())
        }
        Err(e) => error!("Tip preparation failed: {}", e),
    }

    info!("Cleanup complete");
    result.map(|_| ())
}

// ============================================================================
// Tip preparation logic
// ============================================================================

enum Outcome {
    Completed,
    StoppedByUser,
    CycleLimit(usize),
    TimedOut(Duration),
}

fn run_tip_prep(
    mut controller: NanonisController,
    events: &EventBus,
    shutdown: &ShutdownFlag,
    config: &AppConfig,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    let mut store = DataStore::new();
    let mut ctx = ActionContext {
        controller: &mut controller,
        store: &mut store,
        events,
    };

    // Pre-loop initialization
    info!("Initializing...");
    SetBias {
        voltage: config.tip_prep.initial_bias_v as f64,
    }
    .execute(&mut ctx)?;

    SetZSetpoint {
        setpoint: config.tip_prep.initial_z_setpoint_a as f64,
    }
    .execute(&mut ctx)?;

    AutoApproach::default().execute(&mut ctx)?;

    Wait {
        duration_ms: config.tip_prep.timing.post_approach_settle_ms,
    }
    .execute(&mut ctx)?;

    // Find the frequency shift signal index
    // TODO: look up by name from signal registry once integrated
    let freq_shift_index = 0_u32;
    let bounds = (
        config.tip_prep.sharp_tip_bounds[0] as f64,
        config.tip_prep.sharp_tip_bounds[1] as f64,
    );

    let max_cycles = config.tip_prep.max_cycles.unwrap_or(usize::MAX);
    let max_duration = config
        .tip_prep
        .max_duration_secs
        .map(Duration::from_secs);
    let start_time = std::time::Instant::now();

    // Main loop: Blunt -> Sharp
    // (Stability checking will be added as a composite action later)
    for cycle in 1..=max_cycles {
        // Check shutdown
        if shutdown.is_requested() {
            return Ok(Outcome::StoppedByUser);
        }

        // Check timeout
        if let Some(max_dur) = max_duration {
            if start_time.elapsed() > max_dur {
                return Ok(Outcome::TimedOut(max_dur));
            }
        }

        // Periodic status
        if cycle % config.tip_prep.timing.status_interval == 0 {
            info!(
                "Cycle {}: elapsed={:.1}s",
                cycle,
                start_time.elapsed().as_secs_f64()
            );
        }

        // Pulse
        BiasPulse {
            voltage: get_pulse_voltage(config, cycle),
            duration_ms: config.tip_prep.timing.pulse_width_ms,
            ..Default::default()
        }
        .execute(&mut ctx)?;

        Wait {
            duration_ms: config.tip_prep.timing.post_pulse_settle_ms,
        }
        .execute(&mut ctx)?;

        // Read signal and check tip state
        let output = ReadSignal {
            index: freq_shift_index,
            ..Default::default()
        }
        .execute_and_store(&mut ctx, "freq_shift")?;

        if let ActionOutput::Value(freq_shift) = output {
            let is_sharp = freq_shift >= bounds.0 && freq_shift <= bounds.1;

            if is_sharp {
                info!(
                    "Tip sharp at cycle {} (freq_shift={:.3} Hz)",
                    cycle, freq_shift
                );
                // TODO: stability checking goes here
                // For now, sharp = done
                return Ok(Outcome::Completed);
            }
        }

        // Reposition for next cycle
        // TODO: replace with SafeReposition composite action
        Withdraw::default().execute(&mut ctx)?;

        Wait {
            duration_ms: config.tip_prep.timing.post_reposition_settle_ms,
        }
        .execute(&mut ctx)?;

        AutoApproach::default().execute(&mut ctx)?;

        Wait {
            duration_ms: config.tip_prep.timing.post_approach_settle_ms,
        }
        .execute(&mut ctx)?;
    }

    Ok(Outcome::CycleLimit(max_cycles))
}

/// Get pulse voltage for the current cycle.
///
/// Currently only supports fixed voltage from config.
/// Stepping and linear strategies will be added as a PulseStrategy action.
fn get_pulse_voltage(config: &AppConfig, _cycle: usize) -> f64 {
    match &config.pulse_method {
        rusty_tip::PulseMethod::Fixed { voltage, .. } => *voltage as f64,
        rusty_tip::PulseMethod::Stepping { voltage_bounds, .. } => voltage_bounds.0 as f64,
        rusty_tip::PulseMethod::Linear { voltage_bounds, .. } => voltage_bounds.0 as f64,
    }
}

// ============================================================================
// Setup helpers
// ============================================================================

fn setup_event_bus(config: &AppConfig) -> Result<EventBus, Box<dyn std::error::Error>> {
    let mut events = EventBus::new();
    events.add_observer(Box::new(ConsoleLogger));

    if config.experiment_logging.enabled {
        let log_path = create_log_file_path(&config.experiment_logging.output_path)?;
        info!("Event log: {}", log_path.display());
        let file = fs::File::create(&log_path)?;
        events.add_observer(Box::new(FileLogger::new(file)));
    }

    events.add_observer(Box::new(EventAccumulator::new(500)));

    Ok(events)
}

fn setup_shutdown_handler() -> ShutdownFlag {
    let shutdown = ShutdownFlag::new();
    let flag = shutdown.arc();

    ctrlc::set_handler(move || {
        info!("Ctrl+C received - initiating graceful shutdown...");
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    })
    .expect("Error setting Ctrl+C handler");

    shutdown
}

fn wait_for_user_confirmation() -> Result<(), Box<dyn std::error::Error>> {
    println!();
    println!("Press Enter to start tip preparation (or Ctrl+C to cancel)...");
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(())
}

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
    let dir = PathBuf::from(log_path);
    fs::create_dir_all(&dir)?;
    let filename = format!("tip_prep_{}.jsonl", Utc::now().format("%Y%m%d_%H%M%S"));
    Ok(dir.join(filename))
}

#[cfg(windows)]
fn ensure_console_allocated() {
    unsafe {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStrExt;

        if winapi::um::consoleapi::AllocConsole() != 0 {
            println!("Console allocated for tip preparation tool");
        }

        let title = "Rusty Tip Preparation Tool (v2)";
        let wide_title: Vec<u16> = OsString::from(title)
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        winapi::um::wincon::SetConsoleTitleW(wide_title.as_ptr());

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
