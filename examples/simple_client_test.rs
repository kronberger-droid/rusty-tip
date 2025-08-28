use log::info;
use rusty_tip::NanonisClient;

/// Test direct NanonisClient without any interface layers
fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    
    info!("=== Simple NanonisClient Test ===");

    // Test direct NanonisClient call (no ActionDriver, no SPMInterface)
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    
    info!("Testing signal_names_get...");
    match client.signal_names_get(false) {
        Ok(names) => info!("SUCCESS: Found {} signals", names.len()),
        Err(e) => {
            info!("FAILED: {}", e);
            return Err(e.into());
        }
    }

    info!("Testing get_bias...");
    match client.get_bias() {
        Ok(bias) => info!("SUCCESS: Current bias = {:.3}V", bias),
        Err(e) => {
            info!("FAILED: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}