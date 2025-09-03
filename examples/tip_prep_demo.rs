use std::time::Duration;

use log::info;
use rusty_tip::{ActionDriver, SignalIndex, TipController};

/// Simple tip control loop demo
///
/// Minimal replication of original controller behavior:
/// - Read signal → Classify → Execute based on state
/// - Bad: move tip, withdraw if too many moves  
/// - Good: wait, count towards stable
/// - Stable: success, exit
fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    info!("=== Simple Tip Control Demo ===");

    // Create ActionDriver
    let driver = ActionDriver::new("127.0.0.1", 6501)?;

    // Create simple tip controller
    let mut controller = TipController::new(
        driver,
        SignalIndex(24), // Bias voltage signal
        0.0,             // min bound (V)
        2.0,             // max bound (V)
    );

    info!("Starting tip control loop...");
    info!("  - Monitoring signal index 24 (bias)");
    info!("  - Target range: 0.0V to 2.0V");
    info!("  - Bad → move/withdraw, Good → wait, Stable → success");

    // Run the simple control loop (timeout after 30 seconds)
    match controller.run_loop(Duration::from_secs(30)) {
        Ok(final_state) => {
            info!("=== Success! ===");
            info!("Final state: {:?}", final_state);
        }
        Err(e) => {
            info!("Control loop failed: {}", e);
        }
    }

    Ok(())
}
