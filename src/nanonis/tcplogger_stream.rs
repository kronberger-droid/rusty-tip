use crate::error::NanonisError;
use crate::types::{TCPLogStatus, TCPLoggerData};
use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Cursor, Read};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// TCP Logger stream client for reading continuous data from Nanonis TCP Logger.
///
/// This client reads the binary data stream from the TCP Logger module.
/// Uses an internal buffer that's reused across frame reads for efficiency.
///
/// The stream format is:
/// - Header (18 bytes):
///   - Number of channels: 32-bit integer (4 bytes)
///   - Oversampling: 32-bit float (4 bytes)
///   - Counter: 64-bit integer (8 bytes)
///   - State: 16-bit unsigned integer (2 bytes)
/// - Data: N × 32-bit floats (N × 4 bytes)
pub struct TCPLoggerStream {
    stream: TcpStream,
    /// Reusable buffer for frame data - resized as needed to fit frames
    buffer: Vec<u8>,
}

impl TCPLoggerStream {
    /// Connect to the TCP Logger data stream.
    ///
    /// # Arguments
    /// * `addr` - Server address (e.g., "127.0.0.1")
    /// * `port` - Stream port (typically different from control port)
    ///
    /// # Returns
    /// Connected `TCPLoggerStream` ready to read data frames.
    pub fn connect(addr: &str, port: u16) -> Result<Self, NanonisError> {
        // create the socket address
        let socket_addr: SocketAddr = format!("{addr}:{port}")
            .parse()
            .map_err(|_| NanonisError::InvalidAddress(addr.to_string()))?;

        // connect with timeout
        let stream = TcpStream::connect_timeout(&socket_addr, Duration::from_secs(5))?;

        stream.set_nonblocking(false)?;

        // set stream timeouts for continuous reading
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;

        Ok(Self {
            stream,
            buffer: Vec::with_capacity(1024),
        })
    }

    /// Connect with custom timeout.
    pub fn connect_timeout(addr: &str, port: u16, timeout: Duration) -> Result<Self, NanonisError> {
        // create the socket address
        let socket_addr: SocketAddr = format!("{addr}:{port}")
            .parse()
            .map_err(|_| NanonisError::InvalidAddress(addr.to_string()))?;

        // connect with timeout
        let stream = TcpStream::connect_timeout(&socket_addr, timeout)?;

        // set stream timeouts for continuous reading
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;

        Ok(Self {
            stream,
            buffer: Vec::with_capacity(1024),
        })
    }

    /// Read a single data frame from the stream.
    ///
    /// # Returns
    /// `TCPLoggerData` containing the frame header and signal data.
    ///
    /// # Frame Format
    /// Always reads 18 bytes header first, then reads data based on num_channels.
    pub fn read_frame(&mut self) -> Result<TCPLoggerData, NanonisError> {
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
        let oversampling = cursor.read_f32::<BigEndian>()?;
        let counter = cursor.read_u64::<BigEndian>()?;
        let state_val = cursor.read_u16::<BigEndian>()?;
        let state = TCPLogStatus::try_from(state_val as i32)?;

        // Calculate total frame size and read data portion
        let data_size = num_channels as usize * 4;
        let total_size = header_size + data_size;

        // Resize buffer to fit entire frame and read data portion
        self.buffer.resize(total_size, 0);

        // Read data portion into buffer
        self.stream
            .read_exact(&mut self.buffer[header_size..])
            .map_err(|e| NanonisError::Io {
                source: e,
                context: "Reading TCP Logger frame data".to_string(),
            })?;

        // Parse data from buffer
        let mut cursor = Cursor::new(&self.buffer[header_size..]);
        let mut data = Vec::with_capacity(num_channels as usize);

        for _ in 0..num_channels {
            data.push(cursor.read_f32::<BigEndian>()?);
        }

        Ok(TCPLoggerData {
            num_channels,
            oversampling,
            counter,
            state,
            data,
        })
    }

    /// Set read timeout for the stream.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), NanonisError> {
        self.stream
            .set_read_timeout(timeout)
            .map_err(|e| NanonisError::Io {
                source: e,
                context: "Setting read timeout".to_string(),
            })
    }

    /// Check if data is available to read without blocking.
    pub fn data_available(&self) -> Result<bool, NanonisError> {
        let mut buf = [0u8; 1];

        match self.stream.peek(&mut buf) {
            Ok(_) => Ok(true),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
            Err(e) => Err(NanonisError::Io {
                source: e,
                context: "Checking data availability".to_string(),
            }),
        }
    }
}

impl Drop for TCPLoggerStream {
    fn drop(&mut self) {
        // Attempt to gracefully shutdown the TCP connection
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}
