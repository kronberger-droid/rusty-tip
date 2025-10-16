use chrono::Utc;
use log::{error, info};
use rusty_tip::{
    tip_prep::{PulseMethod, TipControllerConfig},
    ActionDriver, SignalIndex, TCPReaderConfig, TipController,
};
use std::{fs, path::PathBuf};

/// Simple tip preparation demo - minimal configuration and straightforward execution
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();

    let driver = ActionDriver::builder("127.0.0.1", 6501)
        .with_tcp_reader(TCPReaderConfig::default())
        .with_action_logging(create_log_file_path()?, 1000, true)
        .build()?;

    // Try different variations of the name
    let freq_shift_signal = SignalIndex::from_name("bias", &driver)?;

    let pulse_method = PulseMethod::stepping_fixed_threshold((2.0, 6.0), 4, 2, 1.0);

    // Create tip controller configuration with registry-based signal
    let config = TipControllerConfig {
        freq_shift_index: freq_shift_signal,
        sharp_tip_bounds: (-2.0, 0.0),
        pulse_method,
        ..Default::default()
    };

    // Create controller
    let mut controller = TipController::new(driver, config);

    match controller.run() {
        Ok(()) => {
            info!("Tip preparation completed successfully!");
        }
        Err(e) => {
            error!("Tip preparation failed: {}", e);
        }
    }

    Ok(())
}

fn create_log_file_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root_dir = std::env::current_dir()?;
    let history_dir = root_dir.join("examples").join("history");

    // Ensure directory exists
    fs::create_dir_all(&history_dir)?;

    // Create timestamped filename
    let filename = format!("log_{}.jsonl", Utc::now().format("%Y%m%d_%H%M%S"));
    let file_path = history_dir.join(filename);

    Ok(file_path)
}
