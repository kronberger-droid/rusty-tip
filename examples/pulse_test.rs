use nanonis_rust::NanonisClient;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    client.set_debug(true);

    // Get signals
    client.signal_names_get(true)?;

    let signal_indices = (0..=127).collect::<Vec<i32>>();

    let signal_values = client.signals_val_get(signal_indices, true)?;

    println!("Signal on channel 1: {signal_values:?}");

    Ok(())
}
