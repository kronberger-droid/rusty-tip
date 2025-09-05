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
    let mut custom_controller = TipController::new(
        driver,
        SignalIndex(24), // Signal to monitor
        1.5,             // Initial pulse voltage (V)
        -0.5,            // Min signal bound (V)
        2.5,             // Max signal bound (V)
    );

    // Configure custom pulse stepping parameters
    custom_controller
        .set_pulse_stepping(0.15, 0.08, 4, 4.0) // 0.15V steps, 0.08 threshold, 4 cycles, max 4V
        .set_stability_threshold(5) // Need 5 consecutive good readings for stable
        .set_max_moves(20); // Allow up to 20 moves before giving up

    // Run the custom configured controller
    match custom_controller.run(Duration::from_secs(30)) {
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
