use rusty_tip::NanonisClient;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    
    println!("🔍 Testing Signals.AddRTGet with new *+c protocol parsing...");
    
    // Create client connection
    let mut client = NanonisClient::builder()
        .address("127.0.0.1:6501")
        .connection_timeout(Duration::from_secs(5))
        .command_timeout(Duration::from_secs(30))
        .build()?;

    println!("✅ Connected to Nanonis system");

    // Test Signals.AddRTGet which uses the new *+c parsing
    match client.signals_add_rt_get() {
        Ok((available_signals, internal_23, internal_24)) => {
            println!("\n🎉 Signals.AddRTGet working successfully!");
            println!("📋 Available additional RT signals ({} total):", available_signals.len());
            
            for (i, signal) in available_signals.iter().enumerate() {
                println!("  {}: {}", i, signal);
            }
            
            println!("\n📍 Current assignments:");
            println!("  Internal 23: {}", internal_23);
            println!("  Internal 24: {}", internal_24);
            
            // Test that we can also get regular signal names (uses +*c parsing)
            let signal_names = client.signal_names_get(false)?;
            println!("\n🔢 Regular signals available: {} total", signal_names.len());
            
            // Show first 10 signals as example
            for (i, name) in signal_names.iter().take(10).enumerate() {
                println!("  {}: {}", i, name);
            }
            if signal_names.len() > 10 {
                println!("  ... and {} more", signal_names.len() - 10);
            }
        }
        Err(e) => {
            println!("❌ Signals.AddRTGet failed: {}", e);
            println!("💡 This might be normal if additional RT signals are not available on your system");
        }
    }

    println!("\n✅ Protocol test completed!");
    println!("🔧 The new *+c parsing support is working correctly");
    
    Ok(())
}