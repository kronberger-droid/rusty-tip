use std::{path::PathBuf, time::Duration, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};

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

    // Create atomic flag for graceful shutdown
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);

    // Wrap controller in Arc<Mutex<>> for signal handler
    let controller = Arc::new(Mutex::new(custom_controller));
    let controller_clone = Arc::clone(&controller);

    // Set up Ctrl-C handler to flush logger and signal stop
    ctrlc::set_handler(move || {
        info!("Received Ctrl-C, signaling stop and flushing logger...");
        running_clone.store(false, Ordering::SeqCst);
        
        if let Ok(mut ctrl) = controller_clone.lock() {
            // Force flush the logger
            match ctrl.flush_logger() {
                Ok(()) => info!("Logger flushed successfully on exit"),
                Err(e) => info!("Failed to flush logger on exit: {}", e),
            }
        }
    })?;

    // Run control loop with periodic checks for the stop signal
    let result: Result<(), Box<dyn std::error::Error>> = {
        let mut ctrl = controller.lock().unwrap();
        
        // Run in shorter intervals so we can check the stop signal
        let mut total_elapsed = Duration::from_secs(0);
        let max_duration = Duration::from_secs(1000);
        let check_interval = Duration::from_secs(5);
        
        while total_elapsed < max_duration && running.load(Ordering::SeqCst) {
            let remaining = max_duration - total_elapsed;
            let run_duration = check_interval.min(remaining);
            
            match ctrl.run(run_duration) {
                Ok(final_state) => {
                    info!("Controller finished with state: {:?}", final_state);
                    break;
                }
                Err(e) if e.to_string().contains("Loop timeout") => {
                    // Expected timeout, continue if still running
                    total_elapsed += run_duration;
                    if !running.load(Ordering::SeqCst) {
                        info!("Stop signal received, exiting gracefully");
                        break;
                    }
                }
                Err(e) => {
                    info!("Controller failed: {}", e);
                    break;
                }
            }
        }
        
        Ok(())
    };

    match result {
        Ok(()) => {
            info!("Controller loop completed");
        }
        Err(e) => {
            info!("Controller loop failed: {}", e);
        }
    }

    Ok(())
}
