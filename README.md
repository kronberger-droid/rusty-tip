# rusty-tip

Rust library for Nanonis SPM (Scanning Probe Microscopy) system control via TCP.

## Installation

```bash
cargo add rusty-tip
```

## Usage

```rust
use rusty_tip::{NanonisClient, TCPLoggerStream};

// Connect to Nanonis system
let mut client = NanonisClient::new("127.0.0.1", 6501)?;
let mut stream = TCPLoggerStream::connect("127.0.0.1", 6590)?;

// Configure and start TCP logging
client.tcplog_chs_set(vec![0, 8])?;
client.tcplog_start()?;

// Read data frames
let frame = stream.read_frame()?;
println!("Data: {:?}", frame.data);
```

## Examples

- `tcp_logger_demo` - TCP data logging
- `tip_prep_demo` - Automated tip preparation with pulse stepping
- `osci_demo` - Oscilloscope data acquisition

## License

MIT
