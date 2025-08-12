use nanonis_rust::{JsonDiskWriter, MachineState, NanonisClient, SyncSignalMonitor};
use std::sync::{Arc, Mutex};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    println!("Signal Monitor Test - Writing minimal JSON + metadata to examples/history/");

    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let filename = format!("examples/history/{timestamp}.jsonl");
    let buffer_size: usize = 20; // in kB

    let disk_writer = JsonDiskWriter::builder()
        .file_path(filename)
        .pretty(false)
        .buffer_size(1000) // Large buffer for high-frequency data
        .build()?;

    // Create signal monitor with batched disk writing
    let mut client = NanonisClient::builder()
        .address("127.0.0.1")
        .port(6501)
        .debug(true)
        .build()?;

    // Create shared state for coordinated updates
    let shared_state = Arc::new(Mutex::new(MachineState::default()));

    let mut signal_monitor = SyncSignalMonitor::builder()
        .address("127.0.0.1")
        .port(6501)
        .signals((0..=127).into_iter().collect()) // Multiple signals for rich context
        .sample_rate(50.0) // 50Hz continuous monitoring
        .buffer_size(20) // Signal history buffer size
        .with_shared_state(shared_state.clone())
        .with_disk_writer(Box::new(disk_writer))
        .build()?;

    // Start monitoring
    println!("Starting signal monitor with minimal JSON format...");
    println!("Buffer size: {buffer_size} samples per batch");
    println!("Metadata will be written to .metadata.json file");

    let mut monitor = signal_monitor;
    let mut receiver = monitor.start_monitoring()?;

    let mut samples_received = 0;

    println!("Collecting samples... Press Ctrl+C to stop");

    // Handle Ctrl+C gracefully
    sync::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nCtrl+C received, shutting down...");
        }
        _ = async {
            loop {
                match receiver.data_receiver.recv().await {
                    Some(machine_state) => {
                        samples_received += 1;
                        let signal_count = machine_state.all_signals.as_ref().map(|s| s.len()).unwrap_or(0);
                        println!(
                            "Sample {}: Primary={:.4}, Signals={}, Classification={:?}, Time={:.3}",
                            samples_received,
                            machine_state.primary_signal,
                            signal_count,
                            machine_state.classification,
                            machine_state.timestamp
                        );
                    }
                    None => {
                        println!("Monitor channel closed");
                        break;
                    }
                }
            }
        } => {}
    }

    // Shutdown monitor gracefully
    println!("Shutting down monitor...");
    let _ = receiver.shutdown_sender.send(());

    // Give the background task time to complete cleanup
    std::thread::sleep(std::time::Duration::from_millis(1000));

    println!("Total samples collected: {samples_received}");

    Ok(())
}
