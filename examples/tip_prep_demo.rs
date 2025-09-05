use std::time::Duration;

use log::info;
use rusty_tip::{ActionDriver, SignalIndex, TipController};

/// Simple tip control loop demo
fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    info!("=== Simple Tip Control Demo ===");

    // Create ActionDriver
    let driver = ActionDriver::new("127.0.0.1", 6501)?;

    // Create simple tip controller
    let mut controller = TipController::new(
        driver,
        SignalIndex(24), // Bias voltage signal
        4.0,
        0.0, // min bound (V)
        2.0, // max bound (V)
    );

    // Run the simple control loop (timeout after 30 seconds)
    match controller.run_loop(Duration::from_secs(30)) {
        Ok(final_state) => {
            info!("Final state: {:?}", final_state);
        }
        Err(e) => {
            info!("Control loop failed: {}", e);
        }
    }

    Ok(())
}
