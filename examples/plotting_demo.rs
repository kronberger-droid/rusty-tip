use rusty_tip::plot_values;
use std::f64::consts::PI;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Plotting Demo ===\n");

    // Demo 1: Simple data
    let simple_data = vec![1.0, 2.0, 3.0, 2.5, 1.5, 0.5, 1.0, 2.0];
    println!("Demo 1: Simple values");
    plot_values(&simple_data, Some("Simple Data"), None, None)?;
    println!("\n");

    // Demo 2: Small values (should use pico scaling)
    let pico_data: Vec<f64> = (0..50)
        .map(|i| 2e-12 * (i as f64 * 0.2).sin() + 1e-12)
        .collect();
    println!("Demo 2: Picoamp-scale values");
    plot_values(&pico_data, Some("Current Signal (Picoamps)"), None, None)?;
    println!("\n");

    // Demo 3: Medium values (should use nano scaling) 
    let nano_data: Vec<f64> = (0..30)
        .map(|i| 5e-9 * (i as f64 * 0.5).cos())
        .collect();
    println!("Demo 3: Nanometer-scale values");
    plot_values(&nano_data, Some("Position Signal (Nanometers)"), None, None)?;
    println!("\n");

    // Demo 4: Sine wave with custom size
    let sine_wave: Vec<f64> = (0..100)
        .map(|i| (i as f64 * 2.0 * PI / 50.0).sin())
        .collect();
    println!("Demo 4: Sine wave with custom size");
    plot_values(&sine_wave, Some("Sine Wave"), Some(120), Some(40))?;
    println!("\n");

    // Demo 5: Real-world-like oscilloscope data
    let mut osci_data: Vec<f64> = Vec::new();
    for i in 0..80 {
        let t = i as f64 * 1e-6; // microsecond timebase
        let signal = 2e-12 * (2.0 * PI * 1e3 * t).sin() + // 1kHz signal
                     0.5e-12 * (2.0 * PI * 10e3 * t).sin() + // 10kHz harmonic
                     0.1e-12 * (i as f64 * 0.1).cos(); // slow drift
        osci_data.push(signal);
    }
    println!("Demo 5: Realistic oscilloscope signal");
    plot_values(&osci_data, Some("Mixed Frequency Signal"), Some(160), Some(50))?;

    println!("\n=== All demos completed! ===");
    Ok(())
}