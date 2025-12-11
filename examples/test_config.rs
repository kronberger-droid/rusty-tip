use rusty_tip::{load_config, load_config_or_default};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    println!("Testing config loading...");

    // Test 1: Load defaults when no config file is found
    println!("\n=== Test 1: Default configuration ===");
    let default_config = load_config_or_default(None);
    println!("Default nanonis host_ip: {}", default_config.nanonis.host_ip);
    println!("Default control ports: {:?}", default_config.nanonis.control_ports);
    println!("Default pulse method: {:?}", default_config.pulse_method);

    // Test 2: Load from base_config.toml if it exists
    let base_config_path = Path::new("base_config.toml");
    if base_config_path.exists() {
        println!("\n=== Test 2: Loading from base_config.toml ===");
        match load_config(Some(base_config_path)) {
            Ok(config) => {
                println!("Loaded nanonis host_ip: {}", config.nanonis.host_ip);
                println!("Loaded control ports: {:?}", config.nanonis.control_ports);
                println!("Loaded pulse method: {:?}", config.pulse_method);
                println!("Loaded tip prep bounds: {:?}", config.tip_prep.sharp_tip_bounds);
            }
            Err(e) => {
                println!("Failed to load config: {}", e);
            }
        }
    } else {
        println!("\n=== Test 2: base_config.toml not found ===");
        println!("No base_config.toml found in current directory");
    }

    // Test 3: Load with automatic discovery (fallback to defaults if not found)
    println!("\n=== Test 3: Automatic config discovery ===");
    let auto_config = load_config_or_default(None);
    println!("Auto-loaded config host_ip: {}", auto_config.nanonis.host_ip);

    println!("\nAll config tests completed successfully!");
    Ok(())
}