use std::time::Duration;

use log::info;
use rusty_tip::{ActionDriver, Job, SignalIndex, TipController};

/// Tip control demo with pulse voltage stepping
fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Create ActionDriver
    let driver = ActionDriver::new("127.0.0.1", 6501)?;

    info!("=== Custom Configuration ===");
    // Create controller with custom pulse stepping parameters
    let mut custom_controller = TipController::new(driver, SignalIndex(76), 2.0, -2.0, 0.0);

    // Configure custom pulse stepping parameters with dynamic threshold
    custom_controller
        .set_pulse_stepping(1.5, Box::new(|signal| signal.abs() / 10.0), 4, 8.0)
        .set_stability_threshold(5);

    // Run the custom configured controller
    match custom_controller.run(Duration::from_secs(1000)) {
        Ok(final_state) => {
            info!("Custom controller result: {:?}", final_state);
            info!(
                "Final pulse voltage: {:.3}V",
                custom_controller.current_pulse_voltage()
            );
            if let Some(avg) = custom_controller.average_signal() {
                info!("Average signal: {:.3}", avg);
            }
            info!("Signal history: {:?}", custom_controller.signal_history());
        }
        Err(e) => {
            info!("Custom controller failed: {}", e);
        }
    }

    Ok(())
}
