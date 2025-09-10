use std::{path::PathBuf, time::Duration};

use chrono::Utc;
use log::info;
use rusty_tip::{ActionDriver, Job, Logger, SignalIndex, TipController};

/// Tip control demo with pulse voltage stepping
fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Create ActionDriver
    let driver = ActionDriver::new("127.0.0.1", 6501)?;

    // Create controller with custom pulse stepping parameters
    let mut custom_controller = TipController::new(driver, SignalIndex(24), 2.0, -2.0, 0.0);

    let file_path = PathBuf::from(format!(
        "./examples/history/log_{}.json",
        Utc::now().format("%Y%m%d_%H%M%S") // Added seconds
    ));

    // Configure custom pulse stepping parameters with dynamic threshold
    custom_controller
        //.set_pulse_stepping(1.5, Box::new(|signal| signal.abs() / 10.0), 4, 8.0)
        .set_pulse_stepping_fixed(1.5, 2.0, 4, 10.0)
        .set_stability_threshold(5)
        .with_logger(Logger::new(file_path, 5));

    // Run the custom configured controller
    match custom_controller.run(Duration::from_secs(1000)) {
        Ok(final_state) => {
            info!("Custom controller result: {:?}", final_state);
        }
        Err(e) => {
            info!("Custom controller failed: {}", e);
        }
    }

    Ok(())
}
