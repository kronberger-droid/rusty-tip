use std::{thread::sleep, time::Duration};

use rusty_tip::{NanonisClient, TCPLoggerStream};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;

    let mut stream = TCPLoggerStream::connect("127.0.0.1", 6590)?;

    println!("{:?}", client.tcplog_status_get());

    client.tcplog_chs_set(vec![0, 8])?;

    sleep(Duration::from_millis(500));

    println!("{:?}", client.tcplog_status_get());

    client.tcplog_start()?;

    println!("{:?}", client.tcplog_status_get());

    sleep(Duration::from_millis(500));

    for i in 0..20 {
        let frame = stream.read_frame()?;
        println!(
            "Frame {}: {} : {:?}, counter: {}",
            i, frame.num_channels, frame.data, frame.counter
        )
    }

    println!("{:?}", client.tcplog_status_get());

    sleep(Duration::from_millis(500));

    client.tcplog_stop()?;

    println!("{:?}", client.tcplog_status_get());

    Ok(())
}
