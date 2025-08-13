use rusty_tip::{JsonDiskWriter, MachineState, SyncSignalMonitor};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("Signal Monitor Test - Writing minimal JSON + metadata to examples/history/");

    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let filename = format!("examples/history/{timestamp}.jsonl");
    let buffer_size: usize = 20; // in kB

    let disk_writer = JsonDiskWriter::builder()
        .file_path(filename)
        .pretty(false)
        .buffer_size(1000) // Large buffer for high-frequency data
        .build()?;

    // Create shared state for coordinated updates
    let shared_state = Arc::new(Mutex::new(MachineState::default()));

    let signal_monitor = SyncSignalMonitor::builder()
        .address("127.0.0.1")
        .port(6501)
        .signals((0..=127).collect()) // Multiple signals for rich context
        .sample_rate(50.0) // 50Hz continuous monitoring
        .buffer_size(20) // Signal history buffer size
        .with_shared_state(shared_state.clone())
        .with_disk_writer(Box::new(disk_writer))
        .build()?;

    // Start monitoring
    log::info!("Starting signal monitor with minimal JSON format...");
    log::info!("Buffer size: {buffer_size} samples per batch");
    log::info!("Metadata will be written to .metadata.json file");

    let mut monitor = signal_monitor;
    let receiver = monitor.start_monitoring()?;

    log::info!("Collecting samples... Press Ctrl+C to stop");

    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_ctrlc = shutdown_flag.clone();

    ctrlc::set_handler(move || {
        log::info!("Ctrl+C pressed - initiating graceful shutdown...");
        shutdown_flag_ctrlc.store(true, Ordering::Relaxed);
    })?;

    while !shutdown_flag.load(Ordering::Relaxed) {
        let signal_running = receiver.is_running.load(Ordering::Relaxed);

        // Log every collected sample from the channel
        while let Ok(sample) = receiver.data_receiver.try_recv() {
            let secs = sample.timestamp as i64;
            let nanos = ((sample.timestamp - secs as f64) * 1_000_000_000.0) as u32;
            let local_time = chrono::DateTime::from_timestamp(secs, nanos)
                .unwrap()
                .with_timezone(&chrono::Local)
                .format("%H:%M:%S%.3f");
            log::info!(
                "Sample at {}: signals={}",
                local_time,
                sample.all_signals.as_ref().map(|s| s.len()).unwrap_or(0),
            );
        }

        if !signal_running {
            log::warn!("Background thread stopped");
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(100)); // Check more frequently
    }

    // Shutdown monitor gracefully
    log::info!("Shutting down monitor...");

    // Stop signal monitor
    if let Err(e) = receiver.shutdown_sender.send(()) {
        log::warn!("Failed to send signal monitor shutdown signal: {e}");
    }

    // Give the background task time to complete cleanup
    std::thread::sleep(std::time::Duration::from_millis(1000));

    Ok(())
}
