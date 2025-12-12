use chrono::Utc;
use env_logger::Env;
use log::{info, LevelFilter};
use rusty_tip::{
    load_config_or_default, ActionDriver, SignalIndex, TCPReaderConfig,
};
use std::{
    env, fs,
    path::PathBuf,
    time::Duration,
};

use rusty_tip::actions::{Action, TipCheckMethod};

/// Debug script to test SafeReposition + CheckTipState and log all buffer data
///
/// Usage:
///   cargo run --example debug_tip_state
///   cargo run --example debug_tip_state -- --config path/to/config.toml
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let config_path = if args.len() > 2 && args[1] == "--config" {
        Some(PathBuf::from(&args[2]))
    } else {
        None
    };

    // Load configuration
    let app_config = load_config_or_default(config_path.as_deref());

    // Initialize logging at debug level to see all details
    env_logger::Builder::from_env(Env::default())
        .filter_level(LevelFilter::Debug)
        .format_timestamp_millis()
        .init();

    info!("=== Debug: SafeReposition + CheckTipState ===");
    info!(
        "Configuration loaded from: {:?}",
        config_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "defaults".to_string())
    );
    info!(
        "Nanonis host: {}:{}",
        app_config.nanonis.host_ip, app_config.nanonis.control_ports[0]
    );

    // Create driver from config
    let mut builder = ActionDriver::builder(
        &app_config.nanonis.host_ip,
        app_config.nanonis.control_ports[0],
    )
    .with_tcp_reader(TCPReaderConfig {
        stream_port: app_config.data_acquisition.data_port,
        oversampling: (2000 / app_config.data_acquisition.sample_rate) as i32,
        ..Default::default()
    });

    // Apply custom TCP channel mapping from config if provided
    if let Some(ref tcp_mapping) = app_config.tcp_channel_mapping {
        let mapping: Vec<(u8, u8)> = tcp_mapping
            .iter()
            .map(|m| (m.nanonis_index, m.tcp_channel))
            .collect();
        info!(
            "Using custom TCP channel mapping from config with {} entries",
            mapping.len()
        );
        builder = builder.with_custom_tcp_mapping(&mapping);
    }

    let mut driver = builder.build()?;

    info!("Connected to Nanonis system");

    // Get frequency shift signal
    let freq_shift_signal = SignalIndex::from_name("freq shift", &driver)?;
    info!("Using frequency shift signal index: {}", freq_shift_signal.0.0);

    // Validate TCP mapping
    match driver.validate_tcp_signal(freq_shift_signal) {
        Ok(tcp_ch) => info!("Frequency shift signal maps to TCP channel: {}", tcp_ch),
        Err(e) => info!("WARNING: Frequency shift signal has no TCP mapping: {}", e),
    }

    // Get sharp tip bounds from config
    let sharp_tip_bounds = (
        app_config.tip_prep.sharp_tip_bounds[0],
        app_config.tip_prep.sharp_tip_bounds[1],
    );
    info!(
        "Sharp tip bounds: {:.2} to {:.2}",
        sharp_tip_bounds.0, sharp_tip_bounds.1
    );

    // Create output file for logging
    let log_dir = PathBuf::from("./debug_logs");
    fs::create_dir_all(&log_dir)?;
    let log_file = log_dir.join(format!(
        "tip_state_debug_{}.csv",
        Utc::now().format("%Y%m%d_%H%M%S")
    ));
    let mut csv_content = String::from("iteration,timestamp,tip_shape,confidence,measured_value,mean,std_dev,dataset_size,raw_data_sample\n");

    info!("Logging data to: {}", log_file.display());

    // Run 10 iterations of SafeReposition + CheckTipState
    for i in 0..10 {
        info!("\n=== Iteration {} ===", i + 1);

        // Perform SafeReposition
        info!("Executing SafeReposition...");
        driver
            .run(Action::SafeReposition {
                x_steps: 2,
                y_steps: 2,
            })
            .go()?;

        // Wait for signal to stabilize
        info!("Waiting 1s for signal to stabilize...");
        std::thread::sleep(Duration::from_secs(1));

        // Check tip state
        info!("Checking tip state...");
        use rusty_tip::actions::TipState;
        let tip_state: TipState = driver
            .run(Action::CheckTipState {
                method: TipCheckMethod::SignalBounds {
                    signal: freq_shift_signal,
                    bounds: sharp_tip_bounds,
                },
            })
            .expecting()?;

        let measured_value = tip_state
            .measured_signals
            .get(&freq_shift_signal)
            .copied()
            .unwrap_or(0.0);

        // Extract metadata
        let mean = tip_state
            .metadata
            .get("most_recent_metric_mean")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(0.0);
        let std_dev = tip_state
            .metadata
            .get("most_recent_metric_std_dev")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(0.0);
        let dataset_size = tip_state
            .metadata
            .get("most_recent_raw_data_full_count")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        let raw_data_sample = tip_state
            .metadata
            .get("most_recent_raw_data_summary")
            .cloned()
            .unwrap_or_else(|| "N/A".to_string());

        info!(
            "Result: shape={:?}, confidence={:.3}, value={:.6}",
            tip_state.shape, tip_state.confidence, measured_value
        );
        info!(
            "Statistics: mean={:.6}, std_dev={:.6}, dataset_size={}",
            mean, std_dev, dataset_size
        );
        info!("Raw data sample: {}", raw_data_sample);

        // Log all metadata
        info!("Full metadata:");
        for (key, value) in &tip_state.metadata {
            info!("  {}: {}", key, value);
        }

        // Append to CSV
        csv_content.push_str(&format!(
            "{},{},{:?},{:.6},{:.6},{:.6},{:.6},{},\"{}\"\n",
            i + 1,
            Utc::now().to_rfc3339(),
            tip_state.shape,
            tip_state.confidence,
            measured_value,
            mean,
            std_dev,
            dataset_size,
            raw_data_sample.replace("\"", "\"\"")
        ));

        // Small delay between iterations
        std::thread::sleep(Duration::from_millis(500));
    }

    // Write CSV file
    fs::write(&log_file, csv_content)?;
    info!("\n=== Debug Complete ===");
    info!("Data saved to: {}", log_file.display());

    Ok(())
}
