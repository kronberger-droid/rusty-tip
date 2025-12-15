use rusty_tip::{ActionDriver, SignalIndex, TCPReaderConfig};

/// Integrated Signal Registry Demo
/// 
/// This example demonstrates the integrated SignalRegistry system within ActionDriver.
/// The registry is automatically initialized when the driver is built, using real
/// signal names from the Nanonis system via signal_names_get().
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Signal Registry Integration Demo");
    println!("==============================");

    // Initialize ActionDriver - this automatically creates and initializes the signal registry
    // using real signal names from the connected Nanonis system
    println!("Connecting to Nanonis and initializing signal registry...");
    
    let driver = ActionDriver::builder("127.0.0.1", 6501)
        .with_tcp_reader(TCPReaderConfig::default())
        .build()?;

    println!("Signal registry initialized with {} signals", 
        driver.signal_registry().all_names().len());
    println!("TCP channels available: {}", 
        driver.signal_registry().tcp_signals().len());

    // Demonstrate case-insensitive signal lookup using the integrated registry
    println!("\nCase-insensitive signal lookup:");

    let test_queries = [
        "current",
        "bias", 
        "oc m1 freq shift",
        "x",
        "z",
    ];

    for query in &test_queries {
        match SignalIndex::from_name(query, &driver) {
            Ok(signal_index) => {
                let name = signal_index.name(&driver).unwrap_or("Unknown".to_string());
                let tcp_channel = signal_index.tcp_channel(&driver);
                println!("  '{}' -> {} (index: {}, TCP: {:?})", 
                    query, name, signal_index.0, tcp_channel);
            }
            Err(e) => {
                println!("  '{}' -> {}", query, e);
            }
        }
    }

    // Demonstrate registry access through driver
    println!("\nSignal registry information:");
    
    // Show all TCP-mapped signals
    println!("Signals available in TCP logger:");
    for signal in driver.signal_registry().tcp_signals() {
        println!("  TCP[{}] = Nanonis[{}] = '{}'",
            signal.tcp_channel.unwrap(),
            signal.nanonis_index,
            signal.name
        );
    }

    // Demonstrate practical usage for tip preparation
    println!("\nPractical usage for tip preparation:");
    
    // Find frequency shift signal (common in AFM tip preparation)
    match SignalIndex::from_name("oc m1 freq shift", &driver) {
        Ok(freq_signal) => {
            println!("  Frequency shift signal found:");
            println!("    - Index: {}", freq_signal.0);
            println!("    - Name: {}", freq_signal.name(&driver).unwrap_or("Unknown".to_string()));
            
            if let Some(tcp_ch) = freq_signal.tcp_channel(&driver) {
                println!("    - TCP channel: {}", tcp_ch);
                println!("    - Available for real-time monitoring");
            } else {
                println!("    - Not mapped to TCP logger");
            }
            
            println!("    - Usage: SignalIndex({})", freq_signal.0);
        }
        Err(e) => {
            println!("  Frequency shift signal not found: {}", e);
            println!("  Available signals starting with 'oc':");
            for name in driver.signal_registry().all_names() {
                if name.to_lowercase().starts_with("oc") {
                    println!("    - {}", name);
                }
            }
        }
    }

    // Show index conversion capabilities
    println!("\nIndex conversion examples:");
    
    // Convert some Nanonis indices to TCP channels
    for nanonis_idx in [0, 1, 8, 12] {
        let nanonis_typed = rusty_tip::NanonisIndex::from(nanonis_idx);
        if let Ok(tcp_ch) = driver.signal_registry().nanonis_to_tcp(nanonis_typed) {
            if let Some(signal) = driver.signal_registry().get_by_nanonis_index(nanonis_idx) {
                println!("  Nanonis[{}] ('{}') -> TCP[{}]", 
                    nanonis_idx, signal.name, tcp_ch);
            }
        }
    }

    Ok(())
}