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
use rusty_tip::action::motor::MoveMotor3D;
use rusty_tip::action::pll::CenterFreqShift;
use rusty_tip::action::signals::ReadSignal;
use rusty_tip::action::util::Wait;
use rusty_tip::action::z_controller::{AutoApproach, SetZSetpoint, Withdraw};
use rusty_tip::PolaritySign;
use rusty_tip::action::Action;
use rusty_tip::action::DataStore;
use rusty_tip::event::{ConsoleLogger, EventAccumulator, EventBus, FileLogger};
use rusty_tip::nanonis_controller::NanonisController;
use rusty_tip::signal_registry::SignalRegistry;
use rusty_tip::spm_controller::SpmController;
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
    let mut controller = NanonisController::new(client);
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

    // Setup event bus
    let events = setup_event_bus(&config)?;

    // Setup shutdown handler
    let shutdown = setup_shutdown_handler();

    // Wait for user confirmation
    wait_for_user_confirmation()?;

    // Run tip preparation
    let result = run_tip_prep(controller, &events, &shutdown, &config, freq_shift_index);

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
    freq_shift_index: u32,
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
    CenterFreqShift.execute(&mut ctx)?;

    Wait {
        duration_ms: config.tip_prep.timing.post_approach_settle_ms,
    }
    .execute(&mut ctx)?;

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
    let mut pulse_state = PulseState::new(config);

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

        // Read signal before pulse (for pulse strategy decisions)
        let output = ReadSignal {
            index: freq_shift_index,
            ..Default::default()
        }
        .execute_and_store(&mut ctx, "freq_shift")?;

        let current_freq_shift = match output {
            ActionOutput::Value(v) => Some(v),
            _ => None,
        };

        // Pulse with strategy-determined voltage (magnitude + polarity sign)
        pulse_state.update_voltage(config, current_freq_shift);
        let pulse_voltage = pulse_state.signed_voltage();
        BiasPulse {
            voltage: pulse_voltage,
            duration_ms: config.tip_prep.timing.pulse_width_ms,
            ..Default::default()
        }
        .execute(&mut ctx)?;

        Wait {
            duration_ms: config.tip_prep.timing.post_pulse_settle_ms,
        }
        .execute(&mut ctx)?;

        // Read signal after pulse and check tip state
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

                if !config.tip_prep.stability.check_stability {
                    info!("Stability checking disabled - accepting sharp tip");
                    return Ok(Outcome::Completed);
                }

                // Simplified stability confirmation: re-read 3 times to verify
                // Full bias sweep stability check will be added later
                let confirmed = confirm_sharp(
                    &mut ctx,
                    freq_shift_index,
                    bounds,
                    config,
                )?;

                if confirmed {
                    info!("Tip confirmed stable after re-checks");
                    return Ok(Outcome::Completed);
                }

                info!("Tip sharpness not confirmed - continuing");
            }
        }

        // Reposition for next cycle (withdraw + motor move + approach)
        reposition(
            &mut ctx,
            config.tip_prep.timing.reposition_steps,
            config.tip_prep.timing.post_reposition_settle_ms,
            config.tip_prep.timing.post_approach_settle_ms,
        )?;
    }

    Ok(Outcome::CycleLimit(max_cycles))
}

/// Withdraw, move motor to a new XY position, and re-approach.
///
/// This mirrors the old SafeReposition: withdraw -> motor move (x, y, z=-3) -> settle -> approach -> center -> settle.
fn reposition(
    ctx: &mut ActionContext,
    reposition_steps: [i16; 2],
    post_reposition_settle_ms: u64,
    post_approach_settle_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    Withdraw::default().execute(ctx)?;

    MoveMotor3D {
        x: reposition_steps[0],
        y: reposition_steps[1],
        z: -3,
        wait: true,
    }
    .execute(ctx)?;

    Wait {
        duration_ms: post_reposition_settle_ms,
    }
    .execute(ctx)?;

    AutoApproach::default().execute(ctx)?;
    CenterFreqShift.execute(ctx)?;

    Wait {
        duration_ms: post_approach_settle_ms,
    }
    .execute(ctx)?;

    Ok(())
}

/// Simplified stability confirmation: reposition and re-read freq_shift 3 times.
/// All readings must fall within sharp bounds.
///
/// Repositioning between each read is critical: a sharp reading at one spot
/// could be a sample artifact, not an actually sharp tip.
///
/// Full bias sweep stability check (from StabilityConfig) will be added later.
fn confirm_sharp(
    ctx: &mut ActionContext,
    freq_shift_index: u32,
    bounds: (f64, f64),
    config: &AppConfig,
) -> Result<bool, Box<dyn std::error::Error>> {
    const CONFIRMATION_READS: usize = 3;

    for i in 0..CONFIRMATION_READS {
        reposition(
            ctx,
            config.tip_prep.timing.reposition_steps,
            config.tip_prep.timing.post_reposition_settle_ms,
            config.tip_prep.timing.post_approach_settle_ms,
        )?;

        let output = ReadSignal {
            index: freq_shift_index,
            ..Default::default()
        }
        .execute(ctx)?;

        if let ActionOutput::Value(fs) = output {
            let in_bounds = fs >= bounds.0 && fs <= bounds.1;
            info!(
                "Stability check {}/{}: freq_shift={:.3} Hz, in_bounds={}",
                i + 1,
                CONFIRMATION_READS,
                fs,
                in_bounds
            );
            if !in_bounds {
                return Ok(false);
            }
        }
    }

    Ok(true)
}

/// Mutable state for pulse voltage strategies that evolve across cycles.
struct PulseState {
    current_voltage: f64,
    cycles_without_change: usize,
    last_freq_shift: Option<f64>,
    /// Base polarity from config
    base_polarity: PolaritySign,
    /// Pulse counter for random polarity switching
    pulse_count: u32,
    /// Random polarity switch config (cloned from PulseMethod)
    random_switch: Option<rusty_tip::RandomPolaritySwitch>,
}

impl PulseState {
    fn new(config: &AppConfig) -> Self {
        let (initial_voltage, base_polarity, random_switch) = match &config.pulse_method {
            rusty_tip::PulseMethod::Fixed {
                voltage,
                polarity,
                random_polarity_switch,
            } => (*voltage as f64, *polarity, random_polarity_switch.clone()),
            rusty_tip::PulseMethod::Stepping {
                voltage_bounds,
                polarity,
                random_polarity_switch,
                ..
            } => (voltage_bounds.0 as f64, *polarity, random_polarity_switch.clone()),
            rusty_tip::PulseMethod::Linear {
                voltage_bounds,
                polarity,
                random_polarity_switch,
                ..
            } => (voltage_bounds.0 as f64, *polarity, random_polarity_switch.clone()),
        };
        Self {
            current_voltage: initial_voltage,
            cycles_without_change: 0,
            last_freq_shift: None,
            base_polarity,
            pulse_count: 0,
            random_switch,
        }
    }

    /// Get the signed voltage for the next pulse.
    ///
    /// Applies polarity sign and random polarity switching to the magnitude.
    fn signed_voltage(&mut self) -> f64 {
        self.pulse_count += 1;

        let effective_polarity = if self.should_use_opposite_polarity() {
            self.base_polarity.opposite()
        } else {
            self.base_polarity
        };

        let sign = match effective_polarity {
            PolaritySign::Positive => 1.0,
            PolaritySign::Negative => -1.0,
        };

        sign * self.current_voltage
    }

    fn should_use_opposite_polarity(&self) -> bool {
        if let Some(ref switch) = self.random_switch {
            switch.enabled
                && self.pulse_count > 0
                && self.pulse_count % switch.switch_every_n_pulses == 0
        } else {
            false
        }
    }

    /// Update pulse voltage magnitude based on the latest freq_shift reading.
    fn update_voltage(&mut self, config: &AppConfig, freq_shift: Option<f64>) {
        match &config.pulse_method {
            rusty_tip::PulseMethod::Fixed { .. } => {
                // Fixed: voltage never changes
            }

            rusty_tip::PulseMethod::Stepping {
                voltage_bounds,
                voltage_steps,
                cycles_before_step,
                threshold_value,
                ..
            } => {
                let (significant, positive_change) = match (freq_shift, self.last_freq_shift) {
                    (Some(current), Some(previous)) => {
                        let change = current - previous;
                        (change.abs() > *threshold_value as f64, change >= 0.0)
                    }
                    _ => (true, true),
                };

                if significant && positive_change {
                    // Positive change: tip is improving, reset to minimum voltage
                    self.cycles_without_change = 0;
                    self.current_voltage = voltage_bounds.0 as f64;
                } else if significant {
                    // Negative significant change: increment counter
                    self.cycles_without_change += 1;
                } else {
                    // No significant change: increment counter
                    self.cycles_without_change += 1;
                }

                if self.cycles_without_change >= *cycles_before_step as usize {
                    let step_size = (voltage_bounds.1 - voltage_bounds.0) as f64
                        / *voltage_steps as f64;
                    let new_voltage =
                        (self.current_voltage + step_size).min(voltage_bounds.1 as f64);
                    if new_voltage > self.current_voltage {
                        info!(
                            "Stepping pulse voltage: {:.3}V -> {:.3}V",
                            self.current_voltage, new_voltage
                        );
                        self.current_voltage = new_voltage;
                    }
                    self.cycles_without_change = 0;
                }

                self.last_freq_shift = freq_shift;
            }

            rusty_tip::PulseMethod::Linear {
                voltage_bounds,
                linear_clamp,
                ..
            } => {
                if let Some(fs) = freq_shift {
                    if fs < linear_clamp.0 as f64 || fs > linear_clamp.1 as f64 {
                        self.current_voltage = voltage_bounds.1 as f64;
                    } else {
                        let slope = (voltage_bounds.1 - voltage_bounds.0) as f64
                            / (linear_clamp.1 - linear_clamp.0) as f64;
                        let intercept = voltage_bounds.0 as f64 - slope * linear_clamp.0 as f64;
                        self.current_voltage = slope * fs + intercept;
                    }
                }

                self.last_freq_shift = freq_shift;
            }
        }
    }
}

// ============================================================================
// Setup helpers
// ============================================================================

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
