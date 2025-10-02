use crate::error::NanonisError;
use crate::types::{TCPLogStatus, TCPLoggerData};
use crate::NanonisClient;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Cursor, Read};
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Builder for configuring and initializing TCPLoggerStream.
///
/// Provides a fluent interface for setting up TCP Logger configuration
/// before connecting and starting data acquisition.
#[derive(Debug, Clone)]
pub struct TCPLoggerStreamBuilder {
    addr: String,
    stream_port: u16,
    control_port: u16,
    channels: Option<Vec<i32>>,
    oversampling: Option<i32>,
    timeout: Option<Duration>,
    auto_start: bool,
}

impl TCPLoggerStreamBuilder {
    /// Create a new builder with required connection parameters.
    pub fn new(addr: &str, stream_port: u16, control_port: u16) -> Self {
        Self {
            addr: addr.to_string(),
            stream_port,
            control_port,
            channels: None,
            oversampling: None,
            timeout: None,
            auto_start: true,
        }
    }

    /// Set the channels to record (signal slots 0-23).
    pub fn channels(mut self, channels: Vec<i32>) -> Self {
        self.channels = Some(channels);
        self
    }

    /// Set the oversampling value (0-1000).
    pub fn oversampling(mut self, oversampling: i32) -> Self {
        self.oversampling = Some(oversampling);
        self
    }

    /// Set connection timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Configure whether to automatically start logging after setup.
    pub fn auto_start(mut self, auto_start: bool) -> Self {
        self.auto_start = auto_start;
        self
    }

    /// Build and initialize the TCPLoggerStream.
    ///
    /// This will:
    /// 1. Connect to the stream and control ports
    /// 2. Configure channels if specified
    /// 3. Set oversampling if specified
    /// 4. Start logging if auto_start is true (default)
    pub fn build(self) -> Result<TCPLoggerStream, NanonisError> {
        // Connect to the stream
        let mut stream = if let Some(timeout) = self.timeout {
            TCPLoggerStream::connect_timeout(
                &self.addr,
                self.stream_port,
                self.control_port,
                timeout,
            )?
        } else {
            TCPLoggerStream::connect(&self.addr, self.stream_port, self.control_port)?
        };

        // Configure channels if specified
        if let Some(channels) = self.channels {
            stream.control.tcplog_chs_set(channels)?;
        }

        // Set oversampling if specified
        if let Some(oversampling) = self.oversampling {
            stream.control.tcplog_oversampl_set(oversampling)?;
        }

        // Start logging if requested
        if self.auto_start {
            stream.control.tcplog_start()?;
        }

        Ok(stream)
    }
}

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
    control: NanonisClient,
    /// Reusable buffer for frame data - resized as needed to fit frames
    buffer: Vec<u8>,
}

impl TCPLoggerStream {
    /// Create a builder for configuring the TCP Logger stream.
    ///
    /// # Arguments
    /// * `addr` - Server address (e.g., "127.0.0.1")
    /// * `stream_port` - TCP Logger data stream port (typically 6590)
    /// * `control_port` - Nanonis control port (typically 6501)
    ///
    /// # Returns
    /// A `TCPLoggerStreamBuilder` for fluent configuration.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::TCPLoggerStream;
    /// use std::time::Duration;
    ///
    /// let stream = TCPLoggerStream::builder("127.0.0.1", 6590, 6501)
    ///     .channels(vec![0, 1, 8])  // Record first 2 signals + bias
    ///     .oversampling(100)
    ///     .timeout(Duration::from_secs(10))
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn builder(addr: &str, stream_port: u16, control_port: u16) -> TCPLoggerStreamBuilder {
        TCPLoggerStreamBuilder::new(addr, stream_port, control_port)
    }

    /// Connect to the TCP Logger data stream.
    ///
    /// # Arguments
    /// * `addr` - Server address (e.g., "127.0.0.1")
    /// * `port` - Stream port (typically different from control port)
    ///
    /// # Returns
    /// Connected `TCPLoggerStream` ready to read data frames.
    pub fn connect(addr: &str, stream_port: u16, control_port: u16) -> Result<Self, NanonisError> {
        // create the socket address
        let socket_addr: SocketAddr = format!("{addr}:{stream_port}")
            .parse()
            .map_err(|_| NanonisError::InvalidAddress(addr.to_string()))?;

        // connect with timeout
        let stream = TcpStream::connect_timeout(&socket_addr, Duration::from_secs(5))?;

        stream.set_nonblocking(false)?;

        // set stream timeouts for continuous reading
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;

        let control = NanonisClient::new(addr, control_port)?;

        Ok(Self {
            stream,
            control,
            buffer: Vec::with_capacity(1024),
        })
    }

    /// Connect with custom timeout.
    pub fn connect_timeout(
        addr: &str,
        stream_port: u16,
        control_port: u16,
        timeout: Duration,
    ) -> Result<Self, NanonisError> {
        // create the socket address
        let socket_addr: SocketAddr = format!("{addr}:{stream_port}")
            .parse()
            .map_err(|_| NanonisError::InvalidAddress(addr.to_string()))?;

        // connect with timeout
        let stream = TcpStream::connect_timeout(&socket_addr, timeout)?;

        let control = NanonisClient::new(addr, control_port)?;

        // set stream timeouts for continuous reading
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;

        Ok(Self {
            stream,
            control,
            buffer: Vec::with_capacity(1024),
        })
    }

    /// Get nanonis side status
    pub fn get_status(&mut self) -> Result<TCPLogStatus, NanonisError> {
        self.control.tcplog_status_get()
    }

    //

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

    /// Spawn a background thread to continuously read frames.
    ///
    /// This method consumes the `TCPLoggerStream` and spawns a background thread
    /// that continuously reads frames and sends them over an mpsc channel.
    ///
    /// # Returns
    /// A `Receiver<TCPLoggerData>` that can be used to receive frames as they arrive.
    /// Use `try_recv()` for non-blocking access or `recv()` for blocking access.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::TCPLoggerStream;
    /// use std::time::Duration;
    ///
    /// let stream = TCPLoggerStream::builder("127.0.0.1", 6590, 6501)
    ///     .channels(vec![0, 8])
    ///     .build()?;
    ///
    /// let receiver = stream.spawn_background_reader();
    ///
    /// // Non-blocking: get latest available frame
    /// if let Ok(frame) = receiver.try_recv() {
    ///     println!("Got frame: {:?}", frame);
    /// }
    ///
    /// // Blocking: wait for next frame
    /// let frame = receiver.recv()?;
    /// println!("Frame data: {:?}", frame.data);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn spawn_background_reader(mut self) -> mpsc::Receiver<TCPLoggerData> {
        let (sender, receiver) = mpsc::channel();

        thread::spawn(move || {
            loop {
                match self.read_frame() {
                    Ok(frame) => {
                        // If receiver is dropped, break the loop
                        if sender.send(frame).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        // Frame read error - break the loop
                        // Could also send error over channel if needed
                        break;
                    }
                }
            }
        });

        receiver
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
        let _ = self.control.tcplog_stop();
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}
