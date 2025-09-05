use rusty_tip::{
    ActionDriver, Job, NanonisClient, SignalIndex, 
    TipController, TipControllerConfig
};
use std::time::Duration;
use log::info;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    
    info!("Enhanced TipController Demo");
    
    // This example shows how to use the enhanced TipController with:
    // 1. Dynamic bias adjustment based on signal feedback
    // 2. Configurable parameters
    // 3. Signal history tracking
    
    // Note: This would connect to real Nanonis hardware
    // let client = NanonisClient::new("127.0.0.1:6501")?;
    // let driver = ActionDriver::new(client);
    
    info!("=== Basic Usage ===");
    // Example 1: Basic tip controller (same as before)
    /* 
    let mut basic_controller = TipController::new(
        driver.clone(),
        SignalIndex(24), // Bias voltage signal
        1.0,             // Pulse voltage
        0.0,             // Min bound
        2.0,             // Max bound
    );
    
    // Set some parameters for stepped bias adjustment
    basic_controller
        .set_bias_stepping(0.2, 0.1, 3, 8.0)  // 0.2V steps, 0.1 change threshold, 3 cycles, max 8V
        .set_stability_threshold(5);           // Need 5 good readings for stable
    
    info!("Basic controller settings:");
    info!("  Current bias: {:.3}V", basic_controller.current_bias());
    info!("  Signal history: {:?}", basic_controller.signal_history());
    */
    
    info!("=== Advanced Configuration ===");
    // Example 2: Using the configuration builder for more complex setups
    /*
    let mut advanced_controller = TipController::with_config(
        driver,
        SignalIndex(24), // Signal to monitor
        1.5,             // Pulse voltage  
        -0.5,            // Min bound
        2.5,             // Max bound
    )
    .bias_stepping(0.15, 0.08, 4, 12.0) // 0.15V steps, 0.08 threshold, 4 cycles, max 12V
    .stability_threshold(4)              // 4 consecutive good readings
    .max_moves(15)                       // Allow up to 15 moves
    .history_size(20)                    // Keep 20 signal readings in history
    .build();
    
    info!("Advanced controller configured with:");
    info!("  Bias stepping: 0.15V steps, 0.08 change threshold, 4 cycles trigger, max 12V");
    info!("  Stability threshold: 4 readings");
    info!("  History size: 20 readings");
    info!("  Max moves: 15");
    
    // Run the controller as a Job
    match advanced_controller.run(Duration::from_secs(30)) {
        Ok(final_state) => {
            info!("TipController completed with state: {:?}", final_state);
            info!("Final bias: {:.3}V", advanced_controller.current_bias());
            info!("Signal history: {:?}", advanced_controller.signal_history());
            if let Some(avg) = advanced_controller.average_signal() {
                info!("Average signal: {:.3}", avg);
            }
        }
        Err(e) => {
            info!("TipController failed: {}", e);
        }
    }
    */
    
    info!("=== How Stepped Bias Adjustment Works ===");
    info!("The enhanced TipController now uses stepped bias adjustment:");
    info!("1. Reads current bias voltage on startup as starting point");
    info!("2. Sets first signal reading as reference for change detection");
    info!("3. For each cycle:");
    info!("   - Reads signal and adds to history");
    info!("   - Compares signal to last significant change");
    info!("   - If change >= threshold: reset cycle counter, update reference");
    info!("   - If change < threshold: increment cycles_without_change");
    info!("   - If cycles_without_change >= trigger: step bias up by voltage_step");
    info!("   - Continue stepping until signal changes or max_bias reached");
    info!("4. Maintains signal history for analysis");
    
    info!("=== Usage Patterns ===");
    info!("- Use basic constructor for simple cases with defaults:");
    info!("  * voltage_step = 0.1V, change_threshold = 0.05, cycles = 3, max = 10V");
    info!("- Use with_config().bias_stepping() for custom parameters");
    info!("- Adjust change_threshold based on signal noise level");
    info!("- Set voltage_step based on desired bias resolution");
    info!("- Set max_bias to prevent dangerous voltage levels");
    info!("- Monitor signal_history() and current_bias() for analysis");
    
    Ok(())
}