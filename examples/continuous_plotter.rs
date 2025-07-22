use nanonis_rust::NanonisClient;
use serde::Serialize;
use std::error::Error;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use textplots::{Chart, Plot, Shape};

#[derive(Debug, Serialize)]
struct DataPoint {
    timestamp: f64,
    value: f64,
    scaled_value: f64,
    unit: String,
}

/// Continuous signal plotting and data logging agent
/// 
/// Usage: cargo run --example continuous_plotter [signal_name] [samples_per_second] [window_size]
/// Example: cargo run --example continuous_plotter "Z (m)" 5 50
/// Press Ctrl+C to quit
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    
    // Parse command line arguments
    let signal_name = args.get(1).map(|s| s.as_str()).unwrap_or("Z (m)");
    let samples_per_sec: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    let window_size: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(100);
    
    let sample_interval = Duration::from_millis(1000 / samples_per_sec);
    
    println!("Continuous Signal Monitor");
    println!("========================");
    println!("Signal: {}", signal_name);
    println!("Sample rate: {} Hz", samples_per_sec);
    println!("Window size: {} samples", window_size);
    println!("Press Ctrl+C to quit");
    println!();

    // Set up Ctrl+C handler
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        println!("\nReceived Ctrl+C, stopping...");
        r.store(false, Ordering::SeqCst);
    })?;

    // Connect to Nanonis
    let mut client = NanonisClient::new("127.0.0.1:6501")?;
    
    // Find signal
    let signals = client.get_signal_names()?;
    let signal_index = signals.iter()
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
    let filename = format!("{}_{}.csv", 
                          signal_name.replace(" ", "_").replace("(", "").replace(")", ""), 
                          timestamp);
    let mut csv_writer = csv::Writer::from_path(&filename)?;
    println!("Logging data to: {}", filename);
    println!();

    // Data collection
    let mut data_points = Vec::new();
    let start_time = Instant::now();
    let mut last_plot = Instant::now();
    let plot_interval = Duration::from_secs(2); // Update plot every 2 seconds
    let mut sample_count = 0;

    println!("Starting continuous data collection...");
    println!("Use Ctrl+C to stop and save data");
    println!();

    while running.load(Ordering::SeqCst) {
        let timestamp = start_time.elapsed().as_secs_f64();
        
        match client.signals_val_get(signal_index as i32, true) {
            Ok(raw_value) => {
                let scaled_value = (raw_value * scale_factor) as f64;
                sample_count += 1;
                
                let data_point = DataPoint {
                    timestamp,
                    value: raw_value as f64,
                    scaled_value,
                    unit: unit.to_string(),
                };
                
                // Save to CSV
                csv_writer.serialize(&data_point)?;
                csv_writer.flush()?;
                
                // Store for plotting with sliding window
                data_points.push((timestamp, scaled_value));
                
                // Keep only the most recent window_size samples
                if data_points.len() > window_size {
                    data_points.remove(0);
                }
                
                // Print current value
                if unit.is_empty() {
                    print!("\r[{:6.1}s] Current: {:8.3} | Samples: {} ", 
                           timestamp, scaled_value, sample_count);
                } else {
                    print!("\r[{:6.1}s] Current: {:8.3} {} | Samples: {} ", 
                           timestamp, scaled_value, unit, sample_count);
                }
                io::stdout().flush()?;
                
                // Plot every few seconds and when we have enough data
                if last_plot.elapsed() >= plot_interval && data_points.len() > 5 {
                    println!(); // New line before plot
                    plot_data_with_stats(&data_points, unit, full_signal_name, window_size)?;
                    last_plot = Instant::now();
                }
            }
            Err(e) => {
                eprintln!("\rError reading signal: {}", e);
                io::stdout().flush()?;
            }
        }
        
        std::thread::sleep(sample_interval);
    }

    println!("\n\nFinal summary:");
    plot_data_with_stats(&data_points, unit, full_signal_name, window_size)?;
    print_final_statistics(&data_points, unit, sample_count, start_time.elapsed(), window_size);
    
    println!("\nData saved to: {}", filename);
    println!("Session completed successfully!");

    Ok(())
}

fn plot_data_with_stats(data: &[(f64, f64)], unit: &str, signal_name: &str, window_size: usize) -> Result<(), Box<dyn Error>> {
    if data.is_empty() {
        return Ok(());
    }

    let points: Vec<(f32, f32)> = data.iter()
        .map(|(t, v)| (*t as f32, *v as f32))
        .collect();

    // Calculate current plot statistics
    let values: Vec<f64> = data.iter().map(|(_, v)| *v).collect();
    let mean: f64 = values.iter().sum::<f64>() / values.len() as f64;
    let min = values.iter().fold(f64::INFINITY, |a, &b| a.min(b));
    let max = values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
    
    let variance: f64 = values.iter()
        .map(|&x| (x - mean).powi(2))
        .sum::<f64>() / values.len() as f64;
    let std_dev = variance.sqrt();

    // Use sliding window for x-axis if we have a full window
    let x_min = if data.len() == window_size {
        data[0].0 as f32
    } else {
        0.0
    };
    
    Chart::new(120, 25, x_min, data.last().unwrap().0 as f32)
        .lineplot(&Shape::Lines(&points))
        .nice();

    // Calculate time window for moving plot
    let time_start = if data.len() == window_size {
        data[0].0
    } else {
        0.0
    };
    let time_end = data.last().unwrap().0;
    
    // Print enhanced statistics for current window
    if unit.is_empty() {
        println!("Signal: {} | Window: {}/{} samples | Time: {:.1}-{:.1}s", 
                signal_name, data.len(), window_size, time_start, time_end);
        println!("Local Mean: {:8.6} | Min: {:8.6} | Max: {:8.6} | StdDev: {:8.6}", 
                mean, min, max, std_dev);
    } else {
        println!("Signal: {} | Window: {}/{} samples | Time: {:.1}-{:.1}s", 
                signal_name, data.len(), window_size, time_start, time_end);
        println!("Local Mean: {:8.3} {} | Min: {:8.3} {} | Max: {:8.3} {} | StdDev: {:8.3} {}", 
                mean, unit, min, unit, max, unit, std_dev, unit);
    }
    
    let stability = if mean.abs() > f64::EPSILON {
        (std_dev / mean.abs() * 100.0).abs()
    } else {
        0.0
    };
    println!("Stability: {:.3}%", stability);
    println!();
    
    Ok(())
}

fn print_final_statistics(data: &[(f64, f64)], unit: &str, total_samples: u64, total_duration: Duration, window_size: usize) {
    if data.is_empty() {
        println!("No data collected");
        return;
    }

    let values: Vec<f64> = data.iter().map(|(_, v)| *v).collect();
    let mean: f64 = values.iter().sum::<f64>() / values.len() as f64;
    let min = values.iter().fold(f64::INFINITY, |a, &b| a.min(b));
    let max = values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
    
    let variance: f64 = values.iter()
        .map(|&x| (x - mean).powi(2))
        .sum::<f64>() / values.len() as f64;
    let std_dev = variance.sqrt();

    println!("Final Session Statistics:");
    println!("{}", "─".repeat(40));
    println!("Duration: {:.1}s", total_duration.as_secs_f64());
    println!("Total samples: {}", total_samples);
    println!("Window size: {} samples", window_size);
    println!("Final window: {}/{} samples", data.len(), window_size);
    println!("Effective rate: {:.1} Hz", total_samples as f64 / total_duration.as_secs_f64());
    
    if unit.is_empty() {
        println!("Mean:     {:10.6}", mean);
        println!("Min:      {:10.6}", min);
        println!("Max:      {:10.6}", max);
        println!("Range:    {:10.6}", max - min);
        println!("Std Dev:  {:10.6}", std_dev);
    } else {
        println!("Mean:     {:10.3} {}", mean, unit);
        println!("Min:      {:10.3} {}", min, unit);
        println!("Max:      {:10.3} {}", max, unit);
        println!("Range:    {:10.3} {}", max - min, unit);
        println!("Std Dev:  {:10.3} {}", std_dev, unit);
    }
    
    let stability = if mean.abs() > f64::EPSILON {
        (std_dev / mean.abs() * 100.0).abs()
    } else {
        0.0
    };
    println!("Stability: {:9.3}%", stability);
}