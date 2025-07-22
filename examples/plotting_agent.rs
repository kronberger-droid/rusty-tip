use nanonis_rust::NanonisClient;
use serde::Serialize;
use std::error::Error;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use textplots::{Chart, Plot, Shape};

#[derive(Debug, Serialize)]
struct DataPoint {
    timestamp: f64,
    value: f64,
    scaled_value: f64,
    unit: String,
}

/// Signal plotting and data logging agent
///
/// Usage: cargo run --example plotting_agent [signal_name] [duration_seconds] [samples_per_second]
/// Example: cargo run --example plotting_agent "Z (m)" 30 5
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();

    // Parse command line arguments
    let signal_name = args.get(1).map(|s| s.as_str()).unwrap_or("Z (m)");
    let duration_secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let samples_per_sec: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);

    let sample_interval = Duration::from_millis(1000 / samples_per_sec);

    println!("Signal Data Logger & Plotter");
    println!("============================");
    println!("Signal: {}", signal_name);
    println!("Duration: {} seconds", duration_secs);
    println!("Sample rate: {} Hz", samples_per_sec);
    println!();

    // Connect to Nanonis
    let mut client = NanonisClient::new("127.0.0.1:6501")?;

    // Find signal
    let signals = client.get_signal_names()?;
    let signal_index = signals
        .iter()
        .position(|s| s.to_lowercase().contains(&signal_name.to_lowercase()))
        .ok_or_else(|| format!("Signal '{}' not found", signal_name))?;

    let full_signal_name = &signals[signal_index];
    println!("Found '{}' at index {}", full_signal_name, signal_index);

    // Determine units and scaling
    let (unit, scale_factor, _precision) = if full_signal_name.contains("(m)") {
        ("pm", 1e12, 1)
    } else if full_signal_name.contains("(A)") {
        ("nA", 1e9, 3)
    } else if full_signal_name.contains("(V)") {
        ("mV", 1e3, 3)
    } else {
        ("", 1.0, 6)
    };

    // Create CSV file for data logging
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let filename = format!(
        "{}_{}.csv",
        signal_name
            .replace(" ", "_")
            .replace("(", "")
            .replace(")", ""),
        timestamp
    );

    // Data collection
    let mut data_points = Vec::new();
    let start_time = Instant::now();
    let end_time = start_time + Duration::from_secs(duration_secs);
    let mut last_plot = Instant::now();
    let plot_interval = Duration::from_secs(2); // Update plot every 2 seconds

    println!("Starting data collection...");
    println!();

    while Instant::now() < end_time {
        let timestamp = start_time.elapsed().as_secs_f64();

        match client.signals_val_get(signal_index as i32, true) {
            Ok(raw_value) => {
                let scaled_value = (raw_value * scale_factor) as f64;

                let data_point = DataPoint {
                    timestamp,
                    value: raw_value as f64,
                    scaled_value,
                    unit: unit.to_string(),
                };

                // Store for plotting
                data_points.push((timestamp, scaled_value));

                // Print current value
                if unit.is_empty() {
                    print!("\r[{:6.1}s] Value: {:8.3} ", timestamp, scaled_value);
                } else {
                    print!(
                        "\r[{:6.1}s] Value: {:8.3} {} ",
                        timestamp, scaled_value, unit
                    );
                }
                std::io::Write::flush(&mut std::io::stdout())?;

                // Plot every few seconds or when we have enough data
                if (last_plot.elapsed() >= plot_interval && data_points.len() > 10)
                    || Instant::now() + Duration::from_millis(500) >= end_time
                {
                    println!(); // New line before plot
                    plot_data(&data_points, unit, full_signal_name)?;
                    last_plot = Instant::now();
                }
            }
            Err(e) => {
                eprintln!("Error reading signal: {}", e);
            }
        }

        std::thread::sleep(sample_interval);
    }

    println!("\n\nFinal plot:");
    plot_data(&data_points, unit, full_signal_name)?;

    Ok(())
}

fn plot_data(data: &[(f64, f64)], unit: &str, signal_name: &str) -> Result<(), Box<dyn Error>> {
    if data.is_empty() {
        return Ok(());
    }

    let points: Vec<(f32, f32)> = data.iter().map(|(t, v)| (*t as f32, *v as f32)).collect();

    let _unit_label = if unit.is_empty() { "Value" } else { unit };

    Chart::new(120, 30, 0.0, data.last().unwrap().0 as f32)
        .lineplot(&Shape::Lines(&points))
        .nice();

    println!(
        "Signal: {} | Samples: {} | Time Range: 0 - {:.1}s",
        signal_name,
        data.len(),
        data.last().unwrap().0
    );

    Ok(())
}
