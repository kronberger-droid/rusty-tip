use plotters::prelude::*;
use rusty_tip::NanonisClient;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    
    println!("Connecting to Nanonis and retrieving scan data for heatmap plotting...");
    
    // Connect to Nanonis
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    
    // Get scan buffer configuration to see available channels
    let (channel_indices, pixels, lines) = client.scan_buffer_get()?;
    println!("Scan buffer configuration:");
    println!("  Available channels: {:?}", channel_indices);
    println!("  Pixels per line: {}", pixels);
    println!("  Number of lines: {}", lines);
    
    if channel_indices.is_empty() {
        println!("No scan channels configured. Please configure scan channels first.");
        return Ok(());
    }
    
    // Use the first available channel
    let channel_index = channel_indices[0] as u32;
    let data_direction = 1; // Forward direction
    
    println!("\nRetrieving scan data for channel {} (forward direction)...", channel_index);
    
    // Get the scan frame data
    match client.scan_frame_data_grab(channel_index, data_direction) {
        Ok((channel_name, rows, columns, scan_data, scan_direction_up)) => {
            println!("Scan data retrieved successfully:");
            println!("  Channel: {}", channel_name);
            println!("  Dimensions: {} rows × {} columns", rows, columns);
            println!("  Scan direction: {}", if scan_direction_up { "Up" } else { "Down" });
            println!("  Data points: {}", rows * columns);
            
            if scan_data.is_empty() || scan_data[0].is_empty() {
                println!("No scan data available. Please ensure a scan has been completed.");
                return Ok(());
            }
            
            // Calculate data range for color mapping
            let mut min_val = f32::INFINITY;
            let mut max_val = f32::NEG_INFINITY;
            
            for row in &scan_data {
                for &value in row {
                    min_val = min_val.min(value);
                    max_val = max_val.max(value);
                }
            }
            
            println!("  Data range: {:.6} to {:.6}", min_val, max_val);
            
            // Create the heatmap plot with proper aspect ratio
            let output_file = "scan_heatmap.png";
            
            // Calculate image dimensions to maintain aspect ratio
            let aspect_ratio = columns as f32 / rows as f32;
            let base_size = 800;
            let (img_width, img_height) = if aspect_ratio > 1.0 {
                // Wider than tall
                (base_size, (base_size as f32 / aspect_ratio) as u32)
            } else {
                // Taller than wide or square
                ((base_size as f32 * aspect_ratio) as u32, base_size)
            };
            
            // Add margins for labels and title
            let total_width = img_width + 120; // 60px margin on each side  
            let total_height = img_height + 120; // 60px margin top/bottom
            
            let root = BitMapBackend::new(output_file, (total_width, total_height)).into_drawing_area();
            root.fill(&WHITE)?;
            
            let mut chart = ChartBuilder::on(&root)
                .caption(format!("Scan Heatmap: {} ({}×{})", channel_name, columns, rows), ("sans-serif", 30))
                .margin(20)
                .x_label_area_size(40)
                .y_label_area_size(60)
                .build_cartesian_2d(0..columns, 0..rows)?;
            
            chart.configure_mesh()
                .x_desc("X (pixels)")
                .y_desc("Y (pixels)")
                .draw()?;
            
            // Create color map function
            let color_range = max_val - min_val;
            let get_color = |value: f32| -> RGBColor {
                if color_range == 0.0 {
                    return RGBColor(128, 128, 128); // Gray for constant data
                }
                
                let normalized = ((value - min_val) / color_range).clamp(0.0, 1.0);
                
                // Create a colormap from blue (low) to red (high) through green
                if normalized < 0.5 {
                    // Blue to Green
                    let t = normalized * 2.0;
                    RGBColor(
                        0,
                        (255.0 * t) as u8,
                        (255.0 * (1.0 - t)) as u8,
                    )
                } else {
                    // Green to Red
                    let t = (normalized - 0.5) * 2.0;
                    RGBColor(
                        (255.0 * t) as u8,
                        (255.0 * (1.0 - t)) as u8,
                        0,
                    )
                }
            };
            
            // Draw the heatmap
            for (row_idx, row) in scan_data.iter().enumerate() {
                for (col_idx, &value) in row.iter().enumerate() {
                    let color = get_color(value);
                    chart.draw_series(std::iter::once(Rectangle::new([
                        (col_idx as i32, row_idx as i32),
                        (col_idx as i32 + 1, row_idx as i32 + 1)
                    ], color.filled())))?;
                }
            }
            
            root.present()?;
            
            println!("\nHeatmap saved to: {}", output_file);
            println!("Heatmap statistics:");
            println!("  Scan dimensions: {} × {} pixels", columns, rows);
            println!("  Aspect ratio: {:.3}", aspect_ratio);
            println!("  Image size: {} × {} pixels", total_width, total_height);
            println!("  Min value: {:.6}", min_val);
            println!("  Max value: {:.6}", max_val);
            
            // Calculate some basic statistics
            let total_points = (rows * columns) as f32;
            let sum: f32 = scan_data.iter().flat_map(|row| row.iter()).sum();
            let average = sum / total_points;
            
            println!("  Average: {:.6}", average);
            
            // Automatically open the heatmap
            println!("\nOpening heatmap...");
            match std::process::Command::new("xdg-open")
                .arg(output_file)
                .spawn()
            {
                Ok(_) => println!("Heatmap opened in default image viewer"),
                Err(e) => {
                    println!("Could not automatically open heatmap: {}", e);
                    println!("You can manually open: {}", output_file);
                }
            }
        },
        Err(e) => {
            println!("Error retrieving scan data: {}", e);
            println!("Make sure:");
            println!("  1. Nanonis is running and accessible");
            println!("  2. A scan has been completed");
            println!("  3. The specified channel is in the scan buffer");
        }
    }
    
    Ok(())
}