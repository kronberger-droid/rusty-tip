use nanonis_rust::NanonisClient;
use std::error::Error;

/// Minimal signal reader agent
///
/// Usage: cargo run --example small_agent [signal_name]
/// Example: cargo run --example small_agent "Input 3"
fn main() -> Result<(), Box<dyn Error>> {
    // Get signal name from command line argument, default to "Current"
    let args: Vec<String> = std::env::args().collect();
    let signal_name = if args.len() > 1 { &args[1] } else { "Current" };

    println!("Reading signal: {}", signal_name);

    // Connect to Nanonis
    let mut client = NanonisClient::new("127.0.0.1:6501")?;

    // Find signal index by name
    let signals = client.get_signal_names()?;
    let signal_index = signals
        .iter()
        .position(|s| s.to_lowercase().contains(&signal_name.to_lowercase()))
        .ok_or_else(|| format!("Signal '{}' not found", signal_name))?;

    println!(
        "Found '{}' at index {}",
        signals[signal_index], signal_index
    );

    // Determine appropriate units and scaling based on signal name
    let (unit, scale_factor, precision) = if signals[signal_index].contains("(m)") {
        ("pm", 1e12, 1)  // Convert meters to picometers
    } else if signals[signal_index].contains("(A)") {
        ("nA", 1e9, 3)   // Convert amperes to nanoamperes
    } else if signals[signal_index].contains("(V)") {
        ("mV", 1e3, 3)   // Convert volts to millivolts
    } else {
        ("", 1.0, 6)     // No conversion, show raw value
    };

    // Read signal value
    let value = client.signals_val_get(signal_index as i32, false)?;
    if unit.is_empty() {
        println!("Value: {:.precision$}", value, precision = precision);
    } else {
        println!("Value: {:.precision$} {}", value * scale_factor, unit, precision = precision);
    }

    // Read 5 samples
    println!("Reading 5 samples:");
    for i in 1..=5 {
        let value = client.signals_val_get(signal_index as i32, true)?;
        if unit.is_empty() {
            println!("  Sample {}: {:.precision$}", i, value, precision = precision);
        } else {
            println!("  Sample {}: {:.precision$} {}", i, value * scale_factor, unit, precision = precision);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    Ok(())
}
