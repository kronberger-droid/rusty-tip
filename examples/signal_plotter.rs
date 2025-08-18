use plotters::prelude::*;
use rusty_tip::NanonisClient;
use std::error::Error;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    
    println!("Connecting to Nanonis and collecting signal data for plotting...");
    
    // Connect to Nanonis
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    
    // Get available signals
    let signal_names = client.signal_names_get(false)?;
    println!("Available signals:");
    for (i, name) in signal_names.iter().enumerate() {
        if !name.is_empty() {
            println!("  {}: {}", i, name);
        }
    }
    
    // Choose first available signal (or a specific one like bias voltage)
    let signal_index = 24; // Typically bias voltage
    let default_name = "Unknown".to_string();
    let signal_name = signal_names.get(signal_index).unwrap_or(&default_name);
    
    println!("\nCollecting data for signal {}: {}", signal_index, signal_name);
    
    // Collect data points
    let mut data_points = Vec::new();
    let start_time = Instant::now();
    let collection_duration = Duration::from_secs(10);
    
    while start_time.elapsed() < collection_duration {
        match client.signals_val_get(vec![signal_index as i32], true) {
            Ok(values) => {
                if let Some(&value) = values.first() {
                    let time_sec = start_time.elapsed().as_secs_f32();
                    data_points.push((time_sec, value));
                    print!("\rCollected {} points...", data_points.len());
                }
            }
            Err(e) => {
                eprintln!("\nError reading signal: {}", e);
                break;
            }
        }
        
        std::thread::sleep(Duration::from_millis(100));
    }
    
    println!("\nCollected {} data points", data_points.len());
    
    if data_points.is_empty() {
        println!("No data collected. Make sure Nanonis is running and accessible.");
        return Ok(());
    }
    
    // Calculate data range for plotting
    let time_range = 0f32..data_points.last().unwrap().0;
    let values: Vec<f32> = data_points.iter().map(|(_, v)| *v).collect();
    let min_val = values.iter().fold(f32::INFINITY, |a, &b| a.min(b));
    let max_val = values.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let value_range = (min_val - 0.1 * (max_val - min_val))..(max_val + 0.1 * (max_val - min_val));
    
    // Create the plot
    let output_file = "signal_plot.png";
    let root = BitMapBackend::new(output_file, (1024, 768)).into_drawing_area();
    root.fill(&WHITE)?;
    
    let mut chart = ChartBuilder::on(&root)
        .caption(format!("Signal Data: {}", signal_name), ("sans-serif", 40))
        .margin(20)
        .x_label_area_size(40)
        .y_label_area_size(60)
        .build_cartesian_2d(time_range, value_range)?;
    
    chart.configure_mesh()
        .x_desc("Time (s)")
        .y_desc("Value")
        .draw()?;
    
    // Draw the data series
    chart.draw_series(LineSeries::new(
        data_points.iter().map(|(t, v)| (*t, *v)),
        &BLUE,
    ))?
    .label("Signal Data")
    .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 10, y)], BLUE));
    
    chart.configure_series_labels().draw()?;
    
    root.present()?;
    
    println!("Plot saved to: {}", output_file);
    println!("Data statistics:");
    println!("  Points: {}", data_points.len());
    println!("  Duration: {:.1} seconds", data_points.last().unwrap().0);
    println!("  Min value: {:.6}", min_val);
    println!("  Max value: {:.6}", max_val);
    println!("  Average: {:.6}", values.iter().sum::<f32>() / values.len() as f32);
    
    // Automatically open the plot
    println!("\nOpening plot...");
    match std::process::Command::new("xdg-open")
        .arg(output_file)
        .spawn()
    {
        Ok(_) => println!("Plot opened in default image viewer"),
        Err(e) => {
            println!("Could not automatically open plot: {}", e);
            println!("You can manually open: {}", output_file);
        }
    }
    
    Ok(())
}