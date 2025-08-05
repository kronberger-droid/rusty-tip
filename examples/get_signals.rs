use nanonis_rust::NanonisClient;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut client = NanonisClient::new("127.0.0.1", "6501")?;

    client.signal_names_get(true)?;

    Ok(())
}
