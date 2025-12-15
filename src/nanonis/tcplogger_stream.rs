use crate::error::NanonisError;
use crate::types::TCPLogStatus;
use crate::SignalFrame;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Cursor, Read};
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Simple TCP Logger Stream - connects to data stream only, no control
pub struct TCPLoggerStream {
    stream: TcpStream,
    buffer: Vec<u8>,
}

impl TCPLoggerStream {
    /// Connect to TCP Logger data stream only
    ///
    /// Creates a simple connection to the TCP data stream without any control operations.
    /// All control (start/stop/configure) should be handled externally.
    ///
    /// # Arguments
    /// * `addr` - Server address (e.g., "127.0.0.1")
    /// * `stream_port` - TCP Logger data stream port (typically 6590)
    ///
    /// # Returns
    /// Connected `TCPLoggerStream` ready to read data frames.
    pub fn new(addr: &str, stream_port: u16) -> Result<Self, NanonisError> {
        let socket_addr: SocketAddr = format!("{addr}:{stream_port}")
            .parse()
            .map_err(|_| NanonisError::InvalidAddress(addr.to_string()))?;

        let stream = TcpStream::connect(socket_addr).map_err(|e| NanonisError::Io {
            source: e,
            context: format!("Failed to connect to TCP stream at {}", socket_addr),
        })?;

        // Set read timeout for continuous reading
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| NanonisError::Io {
                source: e,
                context: "Setting TCP stream read timeout".to_string(),
            })?;

        Ok(Self {
            stream,
            buffer: Vec::with_capacity(1024),
        })
    }

    /// Spawn background reader thread
    ///
    /// Creates a background thread that continuously reads TCP Logger data frames
    /// and sends them through a channel. The thread automatically exits when the
    /// receiver is dropped.
    ///
    /// # Returns
    /// A receiver channel for `TCPLoggerData` frames.
    pub fn spawn_background_reader(mut self) -> mpsc::Receiver<SignalFrame> {
        let (sender, receiver) = mpsc::channel();

        thread::spawn(move || {
            while let Ok(frame) = self.read_frame() {
                if sender.send(frame).is_err() {
                    break;
                }
            }
        });

        receiver
    }

    /// Read a single data frame from the stream
    ///
    /// # Returns
    /// `TCPLoggerData` containing the frame header and signal data.
    ///
    /// # Frame Format
    /// Always reads 18 bytes header first, then reads data based on num_channels.
    pub fn read_frame(&mut self) -> Result<SignalFrame, NanonisError> {
        // First read header to determine frame size
        let header_size = 18;
        self.buffer.resize(header_size, 0);

        // Read header into buffer
        self.stream
            .read_exact(&mut self.buffer[..header_size])
            .map_err(|e| NanonisError::Io {
                source: e,
                context: "Reading TCP Logger frame header".to_string(),
            })?;

        // Parse header from buffer
        let mut cursor = Cursor::new(&self.buffer[..header_size]);
        let num_channels = cursor.read_u32::<BigEndian>()?;
        let _oversampling = cursor.read_f32::<BigEndian>()?;
        let counter = cursor.read_u64::<BigEndian>()?;
        let state_val = cursor.read_u16::<BigEndian>()?;
        let _state = TCPLogStatus::try_from(state_val as i32)?;

        // Calculate total frame size and read data portion
        let data_size = (num_channels * 4) as usize; // 4 bytes per f32
        self.buffer.resize(data_size, 0);

        self.stream
            .read_exact(&mut self.buffer[..data_size])
            .map_err(|e| NanonisError::Io {
                source: e,
                context: "Reading TCP Logger frame data".to_string(),
            })?;

        // Parse data values from buffer
        let mut cursor = Cursor::new(&self.buffer[..data_size]);
        let mut data = Vec::with_capacity(num_channels as usize);
        for _ in 0..num_channels {
            data.push(cursor.read_f32::<BigEndian>()?);
        }

        Ok(SignalFrame { counter, data })
    }
}
