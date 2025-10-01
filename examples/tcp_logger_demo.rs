use std::{thread::sleep, time::Duration};

use rusty_tip::{NanonisClient, TCPLogStatus, TCPLoggerStream};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;

    let mut stream = TCPLoggerStream::connect("127.0.0.1", 6590)?;

    // stream.read_frame()?;
    // client.tcplog_chs_set(vec![0, 8])?;

    // sleep(Duration::from_millis(500));

    toggle_tcplog(&mut client)?;

    sleep(Duration::from_millis(500));

    for i in 0..1000 {
        let frame = stream.read_frame()?;
        println!(
            "Frame {}: {} : {:?}, counter: {}",
            i, frame.num_channels, frame.data, frame.counter
        )
    }

    sleep(Duration::from_millis(500));

    toggle_tcplog(&mut client)?;

    Ok(())
}

fn toggle_tcplog(client: &mut NanonisClient) -> Result<TCPLogStatus, Box<dyn std::error::Error>> {
    let mut status = client.tcplog_status_get()?;

    match status {
        rusty_tip::TCPLogStatus::Start => {
            client.tcplog_stop()?;
        }
        rusty_tip::TCPLogStatus::Stop => {
            client.tcplog_start()?;
        }
        _ => {}
    }

    status = client.tcplog_status_get()?;

    Ok(status)
}
