use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

use chrono::Utc;
use log::info;
use rusty_tip::{
    init_terminal, restore_terminal, ActionDriver, Job, Logger, SignalIndex, SimpleTui,
    TipController,
};

/// Simple TUI demo for tip control with frequency shift visualization
fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Create ActionDriver
    let driver = ActionDriver::new("127.0.0.1", 6501)?;

    // Create controller with custom pulse stepping parameters
    let mut custom_controller = TipController::new(driver, SignalIndex(76), 2.0, (-2.0, 0.0));

    let file_path = create_log_file_path()?;
    println!("Log file: {:?}", file_path);

    // Configure custom pulse stepping parameters
    custom_controller
        .set_pulse_stepping_fixed(1.5, 2.0, 4, 10.0)
        .set_stability_threshold(5)
        .with_logger(Logger::new(file_path, 5));

    // Create channel for passing frequency shift data to TUI
    let (tx, rx) = mpsc::channel::<f32>();

    // Create atomic flag for graceful shutdown
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);
    let running_controller = Arc::clone(&running);

    // Wrap controller in Arc<Mutex<>> for sharing between threads
    let controller = Arc::new(Mutex::new(custom_controller));
    let controller_clone = Arc::clone(&controller);

    // Set up Ctrl-C handler
    ctrlc::set_handler(move || {
        info!("Received Ctrl-C, signaling stop and flushing logger...");
        running_clone.store(false, Ordering::SeqCst);

        if let Ok(mut ctrl) = controller_clone.lock() {
            match ctrl.flush_logger() {
                Ok(()) => info!("Logger flushed successfully on exit"),
                Err(e) => info!("Failed to flush logger on exit: {}", e),
            }
        }
    })?;

    // Initialize terminal for TUI
    let mut terminal = init_terminal()?;

    // Create TUI instance
    let tui = SimpleTui::new(rx);

    // Spawn controller thread
    let controller_thread = {
        let controller = Arc::clone(&controller);
        let running = Arc::clone(&running_controller);
        
        thread::spawn(move || {
                let result = run_controller_loop(controller, running, tx);
            match result {
                Ok(()) => info!("Controller thread completed successfully"),
                Err(e) => info!("Controller thread error: {}", e)
            }
        })
    };

    // Run TUI in main thread
    let tui_result = tui.run(&mut terminal);

    // Signal controller to stop
    running.store(false, Ordering::SeqCst);

    // Wait for controller thread to finish
    let _ = controller_thread.join();

    // Restore terminal
    restore_terminal(&mut terminal)?;

    match tui_result {
        Ok(()) => info!("TUI completed successfully"),
        Err(e) => info!("TUI error: {}", e),
    }

    Ok(())
}

/// Run the controller loop in a separate thread
fn run_controller_loop(
    controller: Arc<Mutex<TipController>>,
    running: Arc<AtomicBool>,
    tx: mpsc::Sender<f32>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut total_elapsed = Duration::from_secs(0);
    let max_duration = Duration::from_secs(1000);
    let check_interval = Duration::from_secs(2); // Shorter intervals for more frequent updates

    while total_elapsed < max_duration && running.load(Ordering::SeqCst) {
        let remaining = max_duration - total_elapsed;
        let run_duration = check_interval.min(remaining);

        // Always extract data before running controller
        if let Ok(ctrl) = controller.lock() {
            if let Some(freq_shift) = ctrl.get_last_signal(SignalIndex(76)) {
                // Send data to TUI (ignore if receiver is closed)
                if tx.send(freq_shift).is_err() {
                    break;
                }
            }
        }

        let result = {
            let mut ctrl = controller.lock().unwrap();
            ctrl.run(run_duration)
        };

        match result {
            Ok(final_state) => {
                info!("Controller finished with state: {:?}", final_state);
                // Send final data point
                if let Ok(ctrl) = controller.lock() {
                    if let Some(freq_shift) = ctrl.get_last_signal(SignalIndex(76)) {
                        let _ = tx.send(freq_shift);
                    }
                }
                break;
            }
            Err(e) if e.to_string().contains("Loop timeout") => {
                // Expected timeout, continue if still running
                total_elapsed += run_duration;

                // Extract frequency shift data and send to TUI after timeout
                if let Ok(ctrl) = controller.lock() {
                    if let Some(freq_shift) = ctrl.get_last_signal(SignalIndex(76)) {
                        // Send data to TUI (ignore if receiver is closed)
                        if tx.send(freq_shift).is_err() {
                            break;
                        }
                    }
                }

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

        // Small delay to prevent busy-waiting
        thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}

fn create_log_file_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root_dir = std::env::current_dir()?;
    let history_dir = root_dir.join("examples").join("history");

    // Ensure directory exists
    fs::create_dir_all(&history_dir)?;

    // Create timestamped filename
    let filename = format!("tui_log_{}.jsonl", Utc::now().format("%Y%m%d_%H%M%S"));
    let file_path = history_dir.join(filename);

    Ok(file_path)
}