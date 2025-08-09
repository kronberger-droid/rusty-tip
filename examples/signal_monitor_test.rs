use nanonis_rust::{
    AsyncSignalMonitor, DiskWriterConfig, DiskWriterFormat, JsonDiskWriter, NanonisClient,
};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    println!("Signal Monitor Test - Writing batched samples to examples/history/");

    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let filename = format!("examples/history/{timestamp}.jsonl");
    let buffer_size: usize = 10;

    // Setup disk writer to save JSON data
    let writer_config = DiskWriterConfig {
        file_path: PathBuf::from(filename),
        format: DiskWriterFormat::Json { pretty: false },
        buffer_size,
    };

    let disk_writer = JsonDiskWriter::new(writer_config).await?;

    // Create signal monitor with batched disk writing
    let mut client = NanonisClient::builder()
        .address("127.0.0.1")
        .port("6501")
        .debug(true)
        .build()?;

    let non_zero_signals: Vec<usize> = client
        .signals_val_get((0..=127).collect::<Vec<i32>>(), true)?
        .into_iter()
        .enumerate()
        .filter(|(_, val)| !val.eq(&0.0))
        .map(|(i, _)| i)
        .collect();
    print!("{non_zero_signals:?}");

    let monitor = AsyncSignalMonitor::new("127.0.0.1", 6502, non_zero_signals, 50.0)?
        .with_disk_writer(Box::new(disk_writer));

    // Start monitoring
    println!("Starting signal monitor with batched writing...");
    println!("Buffer size: {buffer_size} samples per batch");

    let mut monitor = monitor;
    let mut receiver = monitor.start_monitoring().await?;

    let mut samples_received = 0;

    println!("Collecting samples... Press Ctrl+C to stop");

    // Handle Ctrl+C gracefully
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nCtrl+C received, shutting down...");
        }
        _ = async {
            loop {
                match receiver.data_receiver.recv().await {
                    Some(machine_state) => {
                        samples_received += 1;
                        println!(
                            "Sample {}: Signal={:?}, Time={:?}",
                            samples_received, machine_state.primary_signal, machine_state.timestamp
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
    let _ = receiver.shutdown_sender.send(()).await;

    // Give the background task time to complete cleanup
    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

    println!("Total samples collected: {samples_received}");

    Ok(())
}
