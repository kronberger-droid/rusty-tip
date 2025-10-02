use rusty_tip::client::NanonisClient;
use std::time::Duration;
use std::thread;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    println!("Simple TCP Logger Demo");
    println!("======================");

    // Connect to Nanonis
    let mut client = NanonisClient::new("127.0.0.1:6501")?;

    // Configure TCP logger
    println!("Setting up TCP logger...");

    // Set channels to log (channels 0 and 8)
    client.tcplog_chs_set(&[0, 8])?;
    println!("Configured channels: 0, 8");

    // Set oversampling to 10
    client.tcplog_oversampl_set(10)?;
    println!("Set oversampling: 10");

    // Check status before starting
    let status = client.tcplog_status_get()?;
    println!("Initial status: {:?}", status);

    // Start logging
    println!("Starting TCP logger...");
    client.tcplog_start()?;

    // Wait a bit for logger to start
    thread::sleep(Duration::from_millis(500));

    // Check status after starting
    let status = client.tcplog_status_get()?;
    println!("Status after start: {:?}", status);

    // Let it run for a few seconds
    println!("Logging for 5 seconds...");
    thread::sleep(Duration::from_secs(5));

    // Stop logging
    println!("Stopping TCP logger...");
    client.tcplog_stop()?;

    // Check final status
    let status = client.tcplog_status_get()?;
    println!("Final status: {:?}", status);

    println!("Demo complete!");

    Ok(())
}