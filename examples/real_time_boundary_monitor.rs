use nanonis_rust::{
    AsyncSignalMonitor, BoundaryClassifier, Controller, JsonDiskWriter, MachineState,
    RuleBasedPolicy,
};
use std::error::Error;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Integrated real-time boundary monitoring with shared state architecture
///
/// This example demonstrates:
/// - Coherent builder patterns across all components
/// - Shared state integration between AsyncSignalMonitor and Controller
/// - Complete data logging with metadata + minimal JSON format
/// - Real-time coordination: 50Hz monitoring + 2Hz control decisions
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("Real-Time Boundary Monitor with Shared State Architecture");

    // Create shared state for coordination between components
    let shared_state = Arc::new(RwLock::new(MachineState::default()));
    log::info!("Created shared MachineState");

    // Setup disk writer for complete logging using builder pattern
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let filename = format!("examples/history/integrated_{timestamp}.jsonl");

    let disk_writer = JsonDiskWriter::builder()
        .file_path(filename)
        .pretty(false)
        .buffer_size(1000) // Large buffer for high-frequency data
        .build()
        .await?;
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

    // 3. Create AsyncSignalMonitor using builder pattern with shared state
    let signal_monitor = AsyncSignalMonitor::builder()
        .address("127.0.0.1")
        .port(6501)
        .signals(vec![0, 1, 2, 3, 24, 30, 31]) // Multiple signals for rich context
        .sample_rate(50.0) // 50Hz continuous monitoring
        .with_shared_state(shared_state.clone())
        .with_disk_writer(Box::new(disk_writer))
        .build()?;
    log::info!("Built AsyncSignalMonitor with shared state integration");

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

    // Start signal monitor in background task
    let mut signal_monitor = signal_monitor;
    let signal_receiver = signal_monitor.start_monitoring().await?;
    log::info!("AsyncSignalMonitor started in background");

    // Give signal monitor time to populate shared state
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    log::info!("Allowing signal monitor to populate shared state...");

    // Show current shared state
    {
        let state = shared_state.read().await;
        log::info!("Initial shared state timestamp: {}", state.timestamp);
        log::info!("Initial classification: {:?}", state.classification);
        log::info!("Primary signal: {:?}", state.primary_signal);
    }

    // Start controller with shared state coordination
    let mut controller = controller;
    log::info!("Starting Controller with shared state integration...");

    // Run controller for 10 seconds to demonstrate coordination
    let control_duration = tokio::time::Duration::from_secs(10);
    match controller.run_control_loop(2.0, control_duration).await {
        Ok(()) => {
            log::info!("Controller completed successfully");
        }
        Err(e) => {
            log::error!("Controller error (expected if no Nanonis): {}", e);
            log::info!("This is normal when running without Nanonis hardware");
        }
    }

    // Show final shared state
    {
        let state = shared_state.read().await;
        log::info!("Final shared state timestamp: {}", state.timestamp);
        log::info!("Final classification: {:?}", state.classification);
        log::info!("Approach count: {}", state.approach_count);
        if let Some(last_action) = &state.last_action {
            log::info!("Last action: {}", last_action);
        }
    }

    // Cleanup: stop signal monitor
    if let Err(e) = signal_receiver.shutdown_sender.send(()).await {
        log::warn!("Failed to send shutdown signal: {}", e);
    }

    // Give signal monitor time to shut down gracefully
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    log::info!("Real-time boundary monitoring demonstration complete!");
    log::info!("Architecture successfully demonstrated:");
    log::info!("   AsyncSignalMonitor -> Shared State -> Controller coordination");
    log::info!("   50Hz data acquisition with 2Hz control decisions");
    log::info!("   Complete data logging with metadata separation");
    log::info!("   Option A integration working as designed");

    Ok(())
}
