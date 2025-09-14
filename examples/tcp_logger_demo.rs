use rusty_tip::{NanonisClient, TCPLoggerStream};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Current Problems:
    //  - setting signal channels does not work
    //  - getting int(65536) in some cases from status_get
    //  - starting and stopping is very unpredictable
    //
    //  - Also lets test stable osci signal get and the possibility to read z before and after pulse

    let mut control = NanonisClient::new("127.0.0.1", 6501)?;

    println!("{}", control.tcplog_status_get()?);

    control.tcplog_chs_set(vec![0, 8])?;

    println!("{}", control.tcplog_status_get()?);

    control
        .tcplog_stop()
        .unwrap_or_else(|_| println!("cant stop right now"));

    println!("{}", control.tcplog_status_get()?);

    let mut stream = TCPLoggerStream::connect("127.0.0.1", 6590)?;

    println!("{}", control.tcplog_status_get()?);

    control.tcplog_start()?;

    println!("{}", control.tcplog_status_get()?);

    for i in 0..100 {
        let frame = stream.read_frame()?;
        println!(
            "Frame {}: {} : {:?}, counter: {}",
            i, frame.num_channels, frame.data, frame.counter
        )
    }

    // control.tcplog_stop()?;
    Ok(())
}
