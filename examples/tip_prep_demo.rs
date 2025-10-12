use chrono::Utc;
use rusty_tip::{tip_prep::TipControllerConfig, ActionDriver, TCPReaderConfig, TipController};
use std::{fs, path::PathBuf};

/// Tip control demo with pulse voltage stepping
fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Create ActionDriver
    let driver = ActionDriver::builder("127.0.0.1", 6501)
        .with_tcp_reader(TCPReaderConfig::default())
        .with_action_logging(create_log_file_path()?, 1000, true)
        .build()?;

    // Create controller with custom pulse stepping parameters
    let mut custom_controller = TipController::new(driver, TipControllerConfig::default());

    custom_controller.run()?;

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
