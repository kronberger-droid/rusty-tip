use rusty_tip::TCPLoggerStream;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stream = TCPLoggerStream::builder("127.0.0.1", 6590, 6501)
        .channels(vec![0, 8])
        .oversampling(100)
        .build()?;

    println!("TCP Logger started, receiving data in background...");

    let receiver = stream.spawn_background_reader();

    for i in 0..20 {
        let frame = receiver.recv()?;
        println!("Frame {} - {:?}", i + 1, frame.data,);
    }

    Ok(())
}
