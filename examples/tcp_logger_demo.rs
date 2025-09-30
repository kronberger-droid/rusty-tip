use std::{thread::sleep, time::Duration};

use rusty_tip::{NanonisClient, TCPLoggerStream};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut control = NanonisClient::new("127.0.0.1", 6501)?;

    let mut stream = TCPLoggerStream::connect("127.0.0.1", 6590)?;

    sleep(Duration::from_millis(500));

    // control.tcplog_chs_set(vec![0, 8])?;

    // sleep(Duration::from_millis(500));

    match control.tcplog_start() {
        Ok(_) => {}
        Err(_) => {
            control.tcplog_stop()?;
            control.tcplog_start()?;
        }
    }

    sleep(Duration::from_millis(500));

    for i in 0..1000 {
        let frame = stream.read_frame()?;
        println!(
            "Frame {}: {} : {:?}, counter: {}",
            i, frame.num_channels, frame.data, frame.counter
        )
    }

    sleep(Duration::from_millis(500));

    control.tcplog_stop()?;

    Ok(())
}
