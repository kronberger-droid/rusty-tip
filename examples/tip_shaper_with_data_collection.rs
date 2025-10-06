use rusty_tip::{Action, ActionDriver, TCPLoggerConfig};
use std::time::Duration;
use textplots::Plot;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();

    println!("🔬 Tip Shaper with Data Collection Demo");
    println!("=======================================");

    // Setup ActionDriver with always-buffer TCP logger for data collection
    println!("📡 Setting up ActionDriver with TCP buffering for experiment tracking...");
    let mut driver = ActionDriver::builder("127.0.0.1", 6501)
        .with_tcp_logger_buffering(TCPLoggerConfig {
            stream_port: 6590,
            channels: vec![0, 8, 14], // Bias, current, and Z position
            oversampling: 100,
            auto_start: true,
            buffer_size: Some(10_000), // Large buffer for long experiments
        })
        .build()?;

    // Wait a moment for initial data collection
    std::thread::sleep(Duration::from_secs(1));

    // Define the tip shaping action sequence
    let tip_shaping_actions = vec![
        Action::AutoApproach {
            wait_until_finished: true,
            timeout: Duration::from_secs(10),
        },
        Action::PulseRetract {
            pulse_width: Duration::from_millis(500),
            pulse_height_v: 5.0,
        },
        Action::Withdraw {
            wait_until_finished: true,
            timeout: Duration::from_secs(1),
        },
    ];

    // Execute the tip shaping sequence with comprehensive data collection
    let experiment_data = driver.execute_chain_with_data_collection(
        tip_shaping_actions,
        Duration::from_millis(200),  // Collect 200ms before sequence
        Duration::from_millis(1000), // Collect 1s after sequence
    )?;

    // Analyze the experiment results
    println!("\n📊 Experiment Analysis:");
    let (total_actions, successful_actions, total_frames, chain_duration) =
        experiment_data.chain_summary();

    println!("  📈 Overall Results:");
    println!("    - Total actions: {}", total_actions);
    println!("    - Successful actions: {}", successful_actions);
    println!(
        "    - Success rate: {:.1}%",
        (successful_actions as f64 / total_actions as f64) * 100.0
    );
    println!(
        "    - Total chain duration: {:.1}ms",
        chain_duration.as_millis()
    );
    println!("    - Total signal frames collected: {}", total_frames);

    // Plot the collected signal data using textplots
    println!("\n📊 Signal Data Plots:");

    if !experiment_data.signal_frames.is_empty() {
        // Extract time and signal data for plotting
        let time_data: Vec<f32> = experiment_data
            .signal_frames
            .iter()
            .map(|frame| frame.relative_time.as_secs_f32())
            .collect();

        let channel_names = ["Bias (V)", "Current (nA)", "Z Position (nm)"];
        let channel_scales = [1.0, 1e9, 1e9]; // Convert to V, nA, nm

        // Plot each channel
        for (channel_idx, (name, scale)) in
            channel_names.iter().zip(channel_scales.iter()).enumerate()
        {
            if experiment_data
                .signal_frames
                .first()
                .unwrap()
                .signal_frame
                .data
                .len()
                > channel_idx
            {
                let signal_data: Vec<f32> = experiment_data
                    .signal_frames
                    .iter()
                    .map(|frame| frame.signal_frame.data[channel_idx] as f32 * scale)
                    .collect();

                if !signal_data.is_empty() {
                    println!("\n🔍 Channel {}: {}", channel_idx, name);

                    // Create time-signal pairs for plotting
                    let points: Vec<(f32, f32)> = time_data
                        .iter()
                        .zip(signal_data.iter())
                        .map(|(&t, &s)| (t, s))
                        .collect();

                    // Calculate plot bounds
                    let min_time = time_data.iter().fold(f32::INFINITY, |a, &b| a.min(b));
                    let max_time = time_data.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
                    let min_signal = signal_data.iter().fold(f32::INFINITY, |a, &b| a.min(b));
                    let max_signal = signal_data.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));

                    // Add some padding to the ranges
                    let time_range = max_time - min_time;
                    let signal_range = max_signal - min_signal;
                    let time_padding = time_range * 0.05;
                    let signal_padding = signal_range * 0.1;

                    let plot_min_time = min_time - time_padding;
                    let plot_max_time = max_time + time_padding;
                    let _plot_min_signal = min_signal - signal_padding;
                    let _plot_max_signal = max_signal + signal_padding;

                    // Create and display the plot
                    println!(
                        "  Time range: {:.3}s to {:.3}s, Signal range: {:.3} to {:.3}",
                        min_time, max_time, min_signal, max_signal
                    );

                    // Create chart with appropriate dimensions
                    textplots::Chart::new(120, 20, plot_min_time, plot_max_time)
                        .lineplot(&textplots::Shape::Lines(&points))
                        .display();
                }
            }
        }

        // Mark action boundaries on the time axis
        println!("\n⏱️ Action Timing Markers:");
        for (i, result) in experiment_data.action_results.iter().enumerate() {
            if let Some((start, end, duration)) = experiment_data.action_timing(i) {
                let action_name = match i {
                    0 => "AutoApproach",
                    1 => "PulseRetract",
                    2 => "Withdraw",
                    _ => "Unknown",
                };

                let start_time = start
                    .duration_since(experiment_data.chain_start)
                    .as_secs_f64();
                let end_time = end
                    .duration_since(experiment_data.chain_start)
                    .as_secs_f64();

                println!(
                    "  {}. {} - {:.3}s to {:.3}s ({:.1}ms) - Result: {:?}",
                    i + 1,
                    action_name,
                    start_time,
                    end_time,
                    duration.as_millis(),
                    result
                );
            }
        }
    } else {
        println!("  ⚠️ No signal data collected to plot");
    }

    Ok(())
}
