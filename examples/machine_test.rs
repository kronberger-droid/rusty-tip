use std::error::Error;

use rusty_tip::{BiasVoltage, NanonisClient};

fn main() -> Result<(), Box<dyn Error>> {
    let mut client = NanonisClient::builder()
        .address("127.0.0.1")
        .port(6501)
        .debug(true)
        .build()?;

    // Set bias to 1.5V
    client.set_bias(BiasVoltage(1.5))?;

    // Set bias to -0.5V
    let bias = client.read_signal_by_name("bias", true)?;

    println!("{bias}");

    let buffer_conf = client.scan_buffer_get();

    println!("{buffer_conf:?}");

    // let scan_frame = client.scan_frame_data_grab()?;

    println!("{scan_buff_conf:?}");

    Ok(())
}
