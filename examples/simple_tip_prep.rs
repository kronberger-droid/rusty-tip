use chrono::Utc;
use rusty_tip::{
    tip_prep::TipControllerConfig, ActionDriver, SignalIndex, TCPReaderConfig, TipController,
};
use std::{fs, path::PathBuf};

/// Simple tip preparation demo - minimal configuration and straightforward execution
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();

    let driver = ActionDriver::builder("127.0.0.1", 6501)
        .with_tcp_reader(TCPReaderConfig::default())
        .with_action_logging(create_log_file_path()?, 1000, true)
        .build()?;

    // Demonstrate registry-based signal lookup
    println!("📊 Signal Registry Demo:");
    println!("Available TCP signals: {}", driver.signal_registry().tcp_signals().len());
    println!("Total signals: {}", driver.signal_registry().all_names().len());
    
    // Debug: Show signals containing "freq" or "oc"
    println!("\n🔍 Signals containing 'freq':");
    for signal in driver.signal_registry().find_signals_like("freq") {
        println!("  [{}] {} -> TCP: {:?}", signal.nanonis_index, signal.name, signal.tcp_channel);
    }
    
    println!("\n🔍 Signals containing 'oc':");
    for signal in driver.signal_registry().find_signals_like("oc") {
        println!("  [{}] {} -> TCP: {:?}", signal.nanonis_index, signal.name, signal.tcp_channel);
    }
    
    // Show signal at index 76 specifically
    if let Some(signal_76) = driver.signal_registry().get_by_nanonis_index(76) {
        println!("\n📍 Signal at index 76: {} -> TCP: {:?}", signal_76.name, signal_76.tcp_channel);
    }
    
    // Find frequency shift signal by name (case-insensitive)
    // Try different variations of the name
    let freq_shift_signal = match SignalIndex::from_name("oc m1 freq. shift", &driver) {
        Ok(signal) => {
            println!("\n✅ Found frequency shift signal: {} (index {})", 
                signal.name(&driver).unwrap_or("Unknown".to_string()), signal.get());
            signal
        }
        Err(e) => {
            println!("\n❌ {}", e);
            println!("Using default signal index 76");
            SignalIndex::new(76)
        }
    };

    // Create tip controller configuration with registry-based signal
    let config = TipControllerConfig {
        freq_shift_index: freq_shift_signal,
        ..TipControllerConfig::default()
    };

    // Create controller
    let mut controller = TipController::new(driver, config);

    match controller.run() {
        Ok(()) => {
            println!("Tip preparation completed successfully!");
        }
        Err(e) => {
            println!("Tip preparation failed: {}", e);
        }
    }

    Ok(())
}

fn create_log_file_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root_dir = std::env::current_dir()?;
    let history_dir = root_dir.join("examples").join("history");

    // Ensure directory exists
    fs::create_dir_all(&history_dir)?;

    // Create timestamped filename
    let filename = format!("log_{}.jsonl", Utc::now().format("%Y%m%d_%H%M%S"));
    let file_path = history_dir.join(filename);

    Ok(file_path)
}
