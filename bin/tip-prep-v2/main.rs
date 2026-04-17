use chrono::Utc;
use clap::Parser;
use env_logger::Env;
use log::{LevelFilter, error, info};
use std::{
    collections::HashMap, fs, io, path::PathBuf, process::ExitCode,
    time::Duration,
};

use rusty_tip::config::{AppConfig, load_config};
use rusty_tip::event::{ConsoleLogger, EventAccumulator, EventBus, FileLogger};
use rusty_tip::nanonis_controller::{NanonisController, NanonisSetupConfig};
use rusty_tip::signal_registry::SignalRegistry;
use rusty_tip::spm_controller::SpmController;
use rusty_tip::spm_error::SpmError;
use rusty_tip::tip_prep::{Outcome, run_tip_prep};
use rusty_tip::workflow::ShutdownFlag;

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

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        // 2 = completed-but-aborted-by-user / didn't converge. Reserve
        // ExitCode::FAILURE (1) for hard errors so CI can distinguish
        // "the tip didn't get sharp" from "the program crashed."
        Err(RunError::Incomplete) => ExitCode::from(2),
        Err(RunError::Fatal(e)) => {
            error!("{e}");
            ExitCode::FAILURE
        }
    }
}

enum RunError {
    /// Outcome that isn't success but isn't a crash either (cycle limit,
    /// timeout, user-requested stop).
    Incomplete,
    Fatal(Box<dyn std::error::Error>),
}

impl<E: Into<Box<dyn std::error::Error>>> From<E> for RunError {
    fn from(e: E) -> Self {
        RunError::Fatal(e.into())
    }
}

fn run() -> Result<(), RunError> {
    #[cfg(windows)]
    ensure_console_allocated();

    let args = Args::parse();
    let config = load_config(&args.config)?;

    let log_level = args.log_level.unwrap_or(config.console.verbosity.clone());
    initialize_logging(&log_level)?;

    info!("=== Rusty Tip Preparation Tool (v2) ===");
    info!("Configuration: {}", args.config.display());
    info!(
        "Nanonis: {}:{}",
        config.nanonis.host_ip, config.nanonis.control_ports[0]
    );

    // Log configuration parameters
    info!(
        "Sharp tip bounds: {:.2} to {:.2}",
        config.tip_prep.sharp_tip_bounds[0],
        config.tip_prep.sharp_tip_bounds[1]
    );
    info!(
        "Stable tip allowed change: {:.3}",
        config.tip_prep.stability.stable_tip_allowed_change
    );
    info!(
        "Check stability: {}",
        config.tip_prep.stability.check_stability
    );
    match config.tip_prep.max_cycles {
        Some(n) => info!("Max cycles: {}", n),
        None => info!("Max cycles: unlimited"),
    }
    match config.tip_prep.max_duration_secs {
        Some(s) => info!("Max duration: {} seconds", s),
        None => info!("Max duration: unlimited"),
    }
    log_pulse_method_config(&config.pulse_method);

    // Connect to hardware
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
    info!(
        "Frequency shift signal: index {}{}",
        freq_shift_signal.index,
        freq_shift_signal
            .tcp_channel
            .map(|ch| format!(", TCP channel {}", ch))
            .unwrap_or_default()
    );
    let freq_shift_index = freq_shift_signal.index as u32;

    // Setup TCP data stream for stable signal reading
    setup_tcp_stream(&mut controller, &registry, &config)?;

    // Setup event bus
    let events = setup_event_bus(&config)?;

    // Setup shutdown handler
    let shutdown = setup_shutdown_handler();

    // Wait for user confirmation
    wait_for_user_confirmation()?;

    // Run tip preparation using library function
    let result = run_tip_prep(
        Box::new(controller),
        &events,
        &shutdown,
        &config,
        freq_shift_index,
    );

    // Convert ShutdownRequested errors into StoppedByUser outcome
    let result = match result {
        Err(e)
            if e.downcast_ref::<SpmError>()
                .is_some_and(|e| matches!(e, SpmError::ShutdownRequested)) =>
        {
            Ok(Outcome::StoppedByUser)
        }
        other => other,
    };

    match result {
        Ok(Outcome::Completed) => {
            info!("Tip preparation completed successfully!");
            Ok(())
        }
        Ok(Outcome::StoppedByUser) => {
            info!("Tip preparation stopped by user");
            Err(RunError::Incomplete)
        }
        Ok(Outcome::CycleLimit(n)) => {
            error!("Max cycles ({}) exceeded", n);
            Err(RunError::Incomplete)
        }
        Ok(Outcome::TimedOut(d)) => {
            error!("Max duration ({:.0}s) exceeded", d.as_secs_f64());
            Err(RunError::Incomplete)
        }
        Err(e) => {
            error!("Tip preparation failed: {}", e);
            Err(RunError::Fatal(e))
        }
    }
}

// ============================================================================
// Setup helpers
// ============================================================================

fn setup_tcp_stream(
    controller: &mut NanonisController,
    registry: &SignalRegistry,
    config: &AppConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let tcp_signals = registry.tcp_signals();
    if tcp_signals.is_empty() {
        log::warn!(
            "No signals with TCP channel mappings found - stable signal reads will fall back to polling"
        );
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
        if let Some(tcp_ch) = signal.tcp_channel
            && let Some(&position) = tcp_to_position.get(&tcp_ch)
        {
            signal_mapping.insert(signal.index as u32, position);
        }
    }

    info!(
        "TCP stream: {} channels, {} signals mapped",
        tcp_channels.len(),
        signal_mapping.len()
    );

    let oversampling = config.data_acquisition.sample_rate as i32;
    controller.data_stream_configure(&tcp_channels, oversampling)?;
    controller.set_channel_mapping(signal_mapping);

    // Stop any lingering stream from a prior session, then start a fresh one
    // BEFORE attaching the reader — otherwise the reader's first frames may
    // be stale bytes left in the TCP buffer from the previous session.
    let _ = controller.data_stream_stop();
    std::thread::sleep(Duration::from_millis(200));
    controller.data_stream_start()?;

    let buffer_size = 10_000;
    controller.start_tcp_reader(
        &config.nanonis.host_ip,
        config.data_acquisition.data_port,
        buffer_size,
    )?;
    info!("TCP data stream started");

    Ok(())
}

fn log_pulse_method_config(method: &rusty_tip::PulseMethod) {
    match method {
        rusty_tip::PulseMethod::Fixed {
            voltage,
            polarity,
            random_polarity_switch,
        } => {
            info!("Pulse method: Fixed ({:.2}V, {:?})", voltage, polarity);
            log_random_switch(random_polarity_switch);
        }
        rusty_tip::PulseMethod::Stepping {
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
            log_random_switch(random_polarity_switch);
        }
        rusty_tip::PulseMethod::Linear {
            voltage_bounds,
            linear_clamp,
            polarity,
            random_polarity_switch,
        } => {
            info!(
                "Pulse method: Linear (voltage: {:.2}V to {:.2}V, freq_shift range: {:.2} to {:.2} Hz, {:?})",
                voltage_bounds.0,
                voltage_bounds.1,
                linear_clamp.0,
                linear_clamp.1,
                polarity
            );
            log_random_switch(random_polarity_switch);
        }
    }
}

fn log_random_switch(switch: &Option<rusty_tip::RandomPolaritySwitch>) {
    match switch {
        Some(s) if s.enabled => {
            info!(
                "Random polarity switching: every {} pulses",
                s.switch_every_n_pulses
            );
        }
        _ => {
            info!("Random polarity switching: disabled");
        }
    }
}

fn build_signal_registry(
    signal_names: &[String],
    config: &AppConfig,
) -> SignalRegistry {
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

fn setup_event_bus(
    config: &AppConfig,
) -> Result<EventBus, Box<dyn std::error::Error>> {
    let mut events = EventBus::new();
    events.add_observer(Box::new(ConsoleLogger));

    if config.experiment_logging.enabled {
        let log_path =
            create_log_file_path(&config.experiment_logging.output_path)?;
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
    let dir = PathBuf::from(log_path);
    fs::create_dir_all(&dir)?;
    let filename =
        format!("tip_prep_{}.jsonl", Utc::now().format("%Y%m%d_%H%M%S"));
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
