use rusty_tip::NanonisClient;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    println!("Simple Dual Client Test");
    println!("======================");

    // Create two clients
    let mut client1 = NanonisClient::new("127.0.0.1", 6501)?;
    let mut client2 = NanonisClient::new("127.0.0.1", 6502)?;

    println!("Both clients connected");

    // Configure client 1 for signal 0 & 1
    client1.osci2t_ch_set(24, 30)?;
    client1.osci2t_run()?;
    println!("Client 1: configured for signals 0 & 1");

    // Configure client 2 for signal 24 & 2
    client2.osci2t_ch_set(28, 29)?;
    client2.osci2t_run()?;
    println!("Client 2: configured for signals 24 & 2");

    // Collect data from both
    let (_, _, data1a, data1b) = client1.osci2t_data_get(0)?;
    let (_, _, data2a, data2b) = client2.osci2t_data_get(0)?;

    println!("Client 1: {:?} + {:?} samples", data1a, data1b);
    println!("Client 2: {:?} + {:?} samples", data2a, data2b);

    Ok(())
}
