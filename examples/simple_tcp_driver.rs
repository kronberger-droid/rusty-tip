use rusty_tip::{ActionDriver, TCPLoggerConfig};
use std::{thread::sleep, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let mut driver = ActionDriver::builder("127.0.0.1", 6501)
        .with_tcp_logger_buffering(TCPLoggerConfig {
            stream_port: 6590,
            channels: vec![0, 8],
            oversampling: 100,
            auto_start: true,
            buffer_size: Some(5_000),
        })
        .build()?;

    sleep(Duration::from_secs(20));

    println!("{:?}", driver.get_recent_tcp_data(Duration::from_secs(20)));

    Ok(())
}
