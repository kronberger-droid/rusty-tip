use rusty_tip::{ActionDriver, TCPLoggerConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ActionDriver handles ALL control - simple and clean
    let driver = ActionDriver::builder("127.0.0.1", 6501)
        .with_tcp_logger(TCPLoggerConfig {
            stream_port: 6590,
            channels: vec![0, 8],
            oversampling: 100,
            auto_start: true,
        })
        .build()?;

    if let Some(receiver) = driver.tcp_logger_receiver() {
        for i in 0..20 {
            let frame = receiver.recv()?;
            println!("Frame {} - {:?}", i + 1, frame.data);
        }
    }

    Ok(())
}
