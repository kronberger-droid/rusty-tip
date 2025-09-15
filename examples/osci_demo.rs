use rusty_tip::action_driver::ActionDriver;
use rusty_tip::types::{DataToGet, SignalIndex};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Create action driver (will attempt to connect to local Nanonis instance)
    let mut driver = ActionDriver::new("127.0.0.1", 6501)?;

    for i in 1..=10 {
        println!("Measurement {i}:");
        if let Some(osci_data) =
            driver.read_oscilloscope(SignalIndex(0), None, DataToGet::Stable { 
                readings: 5, 
                timeout: Duration::from_secs(10) 
            })?
        {
            // Use convenience methods for cleaner data access
            let values = osci_data.values();
            let max = values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
            let min = values.iter().fold(f64::INFINITY, |a, &b| a.min(b));

            // Show enhanced statistics
            if let Some(stats) = osci_data.stats() {
                println!("  Signal: mean={:.2e}, std={:.2e} (relative: {:.1}%)", 
                         stats.mean, stats.std_dev, stats.relative_std * 100.0);
                println!("  Stability: method={}, window_size={}", 
                         stats.stability_method, stats.window_size);
                println!("  Range: min={:.2e}, max={:.2e}", min, max);
            }
        } else {
            println!("  Could not find a stable value");
        }
        println!();
    }

    Ok(())
}
