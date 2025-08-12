use nanonis_rust::{
    BoundaryClassifier, Controller, JsonDiskWriter, MachineState, RuleBasedPolicy,
    SyncSignalMonitor,
};
use std::error::Error;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Integrated real-time boundary monitoring with shared state architecture
///
/// This example demonstrates:
/// - Coherent builder patterns across all components
/// - Shared state integration between SyncSignalMonitor and Controller
/// - Complete data logging with metadata + minimal JSON format
/// - Real-time coordination: 50Hz monitoring + 2Hz control decisions
fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("Real-Time Boundary Monitor with Shared State Architecture");

    // Create shared state for coordination between components
    let shared_state = Arc::new(Mutex::new(MachineState::default()));
    log::info!("Created shared MachineState");

    // Setup disk writer for complete logging using builder pattern
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let filename = format!("examples/history/integrated_{timestamp}.jsonl");

    let disk_writer = JsonDiskWriter::builder()
        .file_path(filename)
        .pretty(false)
        .buffer_size(1000) // Large buffer for high-frequency data
        .build()?;
    log::info!("Built JsonDiskWriter with builder pattern");

    // 1. Create BoundaryClassifier using builder pattern
    let classifier = BoundaryClassifier::builder()
        .name("Real-Time Bias Monitor")
        .signal_index(24) // Bias voltage signal
        .bounds(0.0, 2.0) // 0V to 2V bounds
        .buffer_config(10, 2) // 10 samples, drop first 2
        .stability_threshold(3) // 3 consecutive good for stable
        .build()?;
    log::info!("Built BoundaryClassifier with builder pattern");

    // 2. Create RuleBasedPolicy using builder pattern
    let policy = RuleBasedPolicy::builder()
        .name("Integrated Control Policy")
        .build()?;
    log::info!("Built RuleBasedPolicy with builder pattern");

    // 3. Create SyncSignalMonitor using builder pattern with shared state
    let mut signal_monitor = SyncSignalMonitor::builder()
        .address("127.0.0.1")
        .port(6501)
        .signals(vec![0, 1, 2, 3, 24, 30, 31]) // Multiple signals for rich context
        .sample_rate(50.0) // 50Hz continuous monitoring
        .buffer_size(20) // Signal history buffer size
        .with_shared_state(shared_state.clone())
        .with_disk_writer(Box::new(disk_writer))
        .build()?;

    // Set primary signal index based on classifier knowledge (clean separation of concerns)
    signal_monitor.set_primary_signal_for_metadata(24); // Bias voltage signal
    log::info!("Built SyncSignalMonitor with shared state integration");

    // 4. Create Controller using builder pattern with shared state
    let controller = Controller::builder()
        .address("127.0.0.1")
        .port(6502)
        .classifier(Box::new(classifier))
        .policy(Box::new(policy))
        .with_shared_state(shared_state.clone())
        .control_interval(2.0) // 2Hz control decisions
        .max_approaches(5)
        .build()?;

    log::info!("Built Controller with shared state integration");

    // 5. Start the integrated system
    log::info!("Starting integrated real-time monitoring system...");
    log::info!("   Signal Monitor: 50Hz continuous data acquisition");
    log::info!("   Controller: 2Hz classification + control decisions");
    log::info!("   Complete logging: signals + classifications + actions");
    log::info!("   Shared state: Real-time coordination");

    // Start signal monitor in background thread
    let signal_receiver = signal_monitor.start_monitoring()?;
    log::info!("SyncSignalMonitor started in background");

    // Give signal monitor time to populate shared state
    std::thread::sleep(std::time::Duration::from_secs(2));
    log::info!("Allowing signal monitor to populate shared state...");

    // Show current shared state
    {
        if let Ok(state) = shared_state.lock() {
            log::info!("Initial shared state timestamp: {}", state.timestamp);
            log::info!("Initial classification: {:?}", state.classification);
            log::info!("Signal history length: {}", state.signal_history.len());
            log::info!(
                "All signals: {:?}",
                state.all_signals.as_ref().map(|s| s.len()).unwrap_or(0)
            );
        }
    }

    // Start controller with shared state coordination
    let mut controller = controller;
    log::info!("Starting Controller with shared state integration...");

    // Create shutdown signal
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_thread = shutdown_flag.clone();

    // Start user input thread
    let input_handle = std::thread::spawn(move || {
        print!("Press Enter to stop the monitoring system...");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        shutdown_flag_thread.store(true, Ordering::Relaxed);
        log::info!("User requested stop - shutting down...");
    });

    // Run controller with periodic checks for shutdown
    log::info!("Running monitoring system indefinitely...");
    log::info!("System will continue monitoring even after achieving STABLE state");

    let mut iteration = 0;
    while !shutdown_flag.load(Ordering::Relaxed) {
        iteration += 1;

        // Run a short control loop (1 second)
        match controller.run_control_loop(2.0, std::time::Duration::from_secs(1)) {
            Ok(()) => {
                // Continue to next iteration
            }
            Err(e) => {
                log::error!("Controller error: {e}");
                if iteration == 1 {
                    log::info!("This is normal when running without Nanonis hardware");
                }
                // Continue running even with errors to show monitoring behavior
            }
        }

        // Small delay to prevent busy loop
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Wait for input thread to complete
    input_handle.join().unwrap();

    // Show final shared state
    {
        if let Ok(state) = shared_state.lock() {
            log::info!("Final shared state timestamp: {}", state.timestamp);
            log::info!("Final classification: {:?}", state.classification);
            log::info!("Approach count: {}", state.approach_count);
            if let Some(last_action) = &state.last_action {
                log::info!("Last action: {last_action}");
            }
        }
    }

    // Cleanup: stop signal monitor
    if let Err(e) = signal_receiver.shutdown_sender.send(()) {
        log::warn!("Failed to send shutdown signal: {e}");
    }

    // Give signal monitor time to shut down gracefully
    std::thread::sleep(std::time::Duration::from_millis(500));

    Ok(())
}
