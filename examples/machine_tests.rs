use std::error::Error;

use nanonis_rust::{BiasVoltage, NanonisClient};

fn main() -> Result<(), Box<dyn Error>> {
    let mut client = NanonisClient::builder()
        .address("127.0.0.1:6501")
        .debug(true)
        .build()?;

    // Set bias to 1.5V
    client.set_bias(BiasVoltage(1.5))?;

    // Set bias to -0.5V
    let bias = client.read_signal_by_name("bias", true)?;

    println!("{bias}");

    Ok(())
}
