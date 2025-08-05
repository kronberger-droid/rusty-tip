use nanonis_rust::{BoundaryClassifier, Controller, NanonisClient, RuleBasedPolicy};
use std::error::Error;
use std::time::Duration;

/// Demonstrates different logging levels and configuration
fn main() -> Result<(), Box<dyn Error>> {
    // Configure logging with different levels based on environment variable
    // Set RUST_LOG=trace for maximum verbosity
    // Set RUST_LOG=debug for detailed debugging
    // Set RUST_LOG=info for normal operation (default)
    // Set RUST_LOG=warn for warnings only
    // Set RUST_LOG=error for errors only
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    
    log::info!("Logging Demo - Showing different log levels");
    log::debug!("This is a debug message - shows detailed operational info");
    log::trace!("This is a trace message - shows very detailed internal state");
    log::warn!("This is a warning message - shows potential issues");
    
    // Connect to Nanonis (will fail if no server running, showing error logging)
    match NanonisClient::new("127.0.0.1", "6501") {
        Ok(client) => {
            log::info!("Successfully connected to Nanonis server");
            
            // Create classifier with trace-level buffer logging
            let classifier = BoundaryClassifier::new(
                "Demo Boundary Classifier".to_string(),
                24,  // Signal index (bias voltage)
                0.0, // min bound (V) 
                2.0, // max bound (V)
            )
            .with_buffer_config(5, 1)   // Small buffer for demo
            .with_stability_config(2);  // Quick stability for demo
            
            let policy = RuleBasedPolicy::new("Demo Policy".to_string());
            let mut controller = Controller::with_client(
                client, 
                Box::new(classifier), 
                Box::new(policy)
            );
            
            // Run short control loop to demonstrate logging
            log::info!("Starting short demo control loop");
            controller.run_control_loop(1.0, Duration::from_secs(5))?;
        }
        Err(e) => {
            log::error!("Failed to connect to Nanonis server: {}", e);
            log::info!("This is expected if no Nanonis server is running");
            log::info!("The error demonstrates error-level logging");
        }
    }
    
    log::info!("Logging demo completed");
    Ok(())
}