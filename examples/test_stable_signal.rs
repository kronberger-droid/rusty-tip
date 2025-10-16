use rusty_tip::{actions::SignalStabilityMethod, Action, ActionDriver};
use std::{error::Error, thread::sleep, time::Duration};

fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logger to see debug output
    env_logger::init();
    let mut driver = ActionDriver::builder("127.0.0.1", 6501)
        .with_tcp_reader(rusty_tip::TCPReaderConfig::default())
        .build()?;

    // Check if TCP logger is available
    if !driver.has_tcp_reader() {
        println!("TCP logger not available");
        return Ok(());
    }

    println!("TCP logger is available, starting data collection...");

    // Wait a moment for TCP logger to start collecting data
    sleep(Duration::from_secs(1));

    let signal = driver
        .run(Action::ReadStableSignal {
            signal: rusty_tip::SignalIndex::new(1),
            data_points: Some(100),
            use_new_data: false,
            stability_method: SignalStabilityMethod::RelativeStandardDeviation {
                threshold_percent: 0.1,
            },
            timeout: Duration::from_secs(10),
            retry_count: Some(5), // Allow 5 retries for stable signal acquisition
        })
        .go()?;

    println!("{signal:?}");

    let mut full_data = Vec::new();

    if let Some(reader) = driver.tcp_reader_mut() {
        full_data = reader.get_all_data();
    }

    let values: Vec<Vec<f32>> = full_data
        .iter()
        .map(|entry| entry.signal_frame.data.clone())
        .collect();

    println!("{values:?}");

    Ok(())
}
