use log::info;
use rusty_tip::NanonisClient;

/// Test direct NanonisClient without any interface layers
fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let mut client = NanonisClient::new("172.0.0.1", 6501)?;

    client.osci1t_ch_set(8)?;

    Ok(())
}
