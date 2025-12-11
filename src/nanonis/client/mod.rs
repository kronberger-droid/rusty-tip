use super::protocol::{Protocol, HEADER_SIZE};
use crate::error::NanonisError;
use crate::types::NanonisValue;
use crate::{MotorDirection, MotorGroup};
use log::{debug, warn};
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

pub mod auto_approach;
pub mod bias;
pub mod bias_sweep;
pub mod current;
pub mod folme;
pub mod motor;
pub mod osci_1t;
pub mod osci_2t;
pub mod osci_hr;
pub mod pll;
pub mod safe_tip;
pub mod scan;
pub mod signals;
pub mod tcplog;
pub mod tip_recovery;
pub mod z_ctrl;
pub mod z_spectr;

// Re-export types from submodules
pub use tip_recovery::{TipShaperConfig, TipShaperProps};
pub use z_spectr::ZSpectroscopyResult;

/// Connection configuration for the Nanonis TCP client.
///
/// Contains timeout settings for different phases of the TCP connection lifecycle.
/// All timeouts have sensible defaults but can be customized for specific network conditions.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use rusty_tip::ConnectionConfig;
///
/// // Use default timeouts
/// let config = ConnectionConfig::default();
///
/// // Customize timeouts for slow network
/// let config = ConnectionConfig {
///     connect_timeout: Duration::from_secs(30),
///     read_timeout: Duration::from_secs(60),
///     write_timeout: Duration::from_secs(10),
/// };
/// ```
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// Timeout for establishing the initial TCP connection
    pub connect_timeout: Duration,
    /// Timeout for reading data from the Nanonis server
    pub read_timeout: Duration,
    /// Timeout for writing data to the Nanonis server
    pub write_timeout: Duration,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(10),
            write_timeout: Duration::from_secs(5),
        }
    }
}

/// Builder for constructing [`NanonisClient`] instances with flexible configuration.
///
/// The builder pattern allows you to configure various aspects of the client
/// before establishing the connection. This is more ergonomic than having
/// multiple constructor variants.
///
/// # Examples
///
/// Basic usage:
/// ```no_run
/// use rusty_tip::NanonisClient;
///
/// let client = NanonisClient::builder()
///     .address("127.0.0.1")
///     .port(6501)
///     .debug(true)
///     .build()?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// With custom timeouts:
/// ```no_run
/// use std::time::Duration;
/// use rusty_tip::NanonisClient;
///
/// let client = NanonisClient::builder()
///     .address("192.168.1.100")
///     .port(6501)
///     .connect_timeout(Duration::from_secs(30))
///     .read_timeout(Duration::from_secs(60))
///     .debug(false)
///     .build()?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Default)]
pub struct NanonisClientBuilder {
    address: Option<String>,
    port: Option<u16>,
    config: ConnectionConfig,
    debug: bool,
}

impl NanonisClientBuilder {
    pub fn address(mut self, addr: &str) -> Self {
        self.address = Some(addr.to_string());
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Enable or disable debug logging
    pub fn debug(mut self, debug: bool) -> Self {
        self.debug = debug;
        self
    }

    /// Set the full connection configuration
    pub fn config(mut self, config: ConnectionConfig) -> Self {
        self.config = config;
        self
    }

    /// Set connect timeout
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.config.connect_timeout = timeout;
        self
    }

    /// Set read timeout
    pub fn read_timeout(mut self, timeout: Duration) -> Self {
        self.config.read_timeout = timeout;
        self
    }

    /// Set write timeout
    pub fn write_timeout(mut self, timeout: Duration) -> Self {
        self.config.write_timeout = timeout;
        self
    }

    /// Build the NanonisClient
    pub fn build(self) -> Result<NanonisClient, NanonisError> {
        let address = self
            .address
            .ok_or_else(|| NanonisError::InvalidCommand("Address must be specified".to_string()))?;

        let port = self
            .port
            .ok_or_else(|| NanonisError::InvalidCommand("Port must be specified".to_string()))?;

        let socket_addr: SocketAddr = format!("{address}:{port}")
            .parse()
            .map_err(|_| NanonisError::InvalidAddress(address.clone()))?;

        debug!("Connecting to Nanonis at {address}");

        let stream = TcpStream::connect_timeout(&socket_addr, self.config.connect_timeout)
            .map_err(|e| {
                warn!("Failed to connect to {address}: {e}");
                if e.kind() == std::io::ErrorKind::TimedOut {
                    NanonisError::Timeout
                } else {
                    NanonisError::Io {
                        source: e,
                        context: format!("Failed to connect to {address}"),
                    }
                }
            })?;

        // Set socket timeouts
        stream.set_read_timeout(Some(self.config.read_timeout))?;
        stream.set_write_timeout(Some(self.config.write_timeout))?;

        debug!("Successfully connected to Nanonis");

        Ok(NanonisClient {
            stream,
            debug: self.debug,
            config: self.config,
        })
    }
}

/// High-level client for communicating with Nanonis SPM systems via TCP.
///
/// `NanonisClient` provides a type-safe, Rust-friendly interface to the Nanonis
/// TCP protocol. It handles connection management, protocol serialization/deserialization,
/// and provides convenient methods for common operations like reading signals,
/// controlling bias voltage, and managing the scanning probe.
///
/// # Connection Management
///
/// The client maintains a persistent TCP connection to the Nanonis server.
/// Connection timeouts and retry logic are handled automatically.
///
/// # Protocol Support
///
/// Supports the standard Nanonis TCP command set including:
/// - Signal reading (`Signals.ValsGet`, `Signals.NamesGet`)
/// - Bias control (`Bias.Set`, `Bias.Get`)
/// - Position control (`FolMe.XYPosSet`, `FolMe.XYPosGet`)
/// - Motor control (`Motor.*` commands)
/// - Auto-approach (`AutoApproach.*` commands)
///
/// # Examples
///
/// Basic usage:
/// ```no_run
/// use rusty_tip::NanonisClient;
///
/// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
///
/// // Read signal names
/// let signals = client.signal_names_get(false)?;
///
/// // Set bias voltage
/// client.set_bias(1.0)?;
///
/// // Read signal values
/// let values = client.signals_vals_get(vec![0, 1, 2], true)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// With builder pattern:
/// ```no_run
/// use std::time::Duration;
/// use rusty_tip::NanonisClient;
///
/// let mut client = NanonisClient::builder()
///     .address("192.168.1.100")
///     .port(6501)
///     .debug(true)
///     .connect_timeout(Duration::from_secs(30))
///     .build()?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct NanonisClient {
    stream: TcpStream,
    debug: bool,
    config: ConnectionConfig,
}

impl NanonisClient {
    /// Create a new client with default configuration.
    ///
    /// This is the most convenient way to create a client for basic usage.
    /// Uses default timeouts and disables debug logging.
    ///
    /// # Arguments
    /// * `addr` - Server address (e.g., "127.0.0.1")
    /// * `port` - Server port (e.g., 6501)
    ///
    /// # Returns
    /// A connected `NanonisClient` ready for use.
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - The address format is invalid
    /// - Connection to the server fails
    /// - Connection times out
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let client = NanonisClient::new("127.0.0.1", 6501)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(addr: &str, port: u16) -> Result<Self, NanonisError> {
        Self::builder().address(addr).port(port).build()
    }

    /// Create a builder for flexible configuration.
    ///
    /// Use this when you need to customize timeouts, enable debug logging,
    /// or other advanced configuration options.
    ///
    /// # Returns
    /// A `NanonisClientBuilder` with default settings that can be customized.
    ///
    /// # Examples
    /// ```no_run
    /// use std::time::Duration;
    /// use rusty_tip::NanonisClient;
    ///
    /// let client = NanonisClient::builder()
    ///     .address("192.168.1.100")
    ///     .port(6501)
    ///     .debug(true)
    ///     .connect_timeout(Duration::from_secs(30))
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn builder() -> NanonisClientBuilder {
        NanonisClientBuilder::default()
    }

    /// Create a new client with custom configuration (legacy method).
    ///
    /// **Deprecated**: Use [`NanonisClient::builder()`] instead for more flexibility.
    ///
    /// # Arguments
    /// * `addr` - Server address in format "host:port"
    /// * `config` - Connection configuration with custom timeouts
    pub fn with_config(addr: &str, config: ConnectionConfig) -> Result<Self, NanonisError> {
        Self::builder().address(addr).config(config).build()
    }

    /// Enable or disable debug output
    pub fn set_debug(&mut self, debug: bool) {
        self.debug = debug;
    }

    /// Get the current connection configuration
    pub fn config(&self) -> &ConnectionConfig {
        &self.config
    }

    /// Send a quick command with minimal response handling.
    ///
    /// This is a low-level method for sending custom commands that don't fit
    /// the standard method patterns. Most users should use the specific
    /// command methods instead.
    pub fn quick_send(
        &mut self,
        command: &str,
        args: Vec<NanonisValue>,
        argument_types: Vec<&str>,
        return_types: Vec<&str>,
    ) -> Result<Vec<NanonisValue>, NanonisError> {
        debug!("=== COMMAND START: {} ===", command);
        debug!("Arguments: {:?}", args);
        debug!("Argument types: {:?}", argument_types);
        debug!("Return types: {:?}", return_types);

        // Serialize arguments
        let mut body = Vec::new();
        for (arg, arg_type) in args.iter().zip(argument_types.iter()) {
            debug!("Serializing {:?} as {}", arg, arg_type);
            Protocol::serialize_value(arg, arg_type, &mut body)?;
        }

        // Create command header
        let header = Protocol::create_command_header(command, body.len() as u32);

        debug!("Header size: {}, Body size: {}", header.len(), body.len());
        debug!("Full header bytes: {:02x?}", header);
        debug!(
            "Command in header: {:?}",
            String::from_utf8_lossy(&header[0..32]).trim_end_matches('\0')
        );
        debug!(
            "Body size in header: {}",
            u32::from_be_bytes([header[32], header[33], header[34], header[35]])
        );

        if !body.is_empty() {
            debug!("Body bytes: {:02x?}", body);
        }

        // Send command
        debug!("Sending header ({} bytes)...", header.len());
        self.stream.write_all(&header).map_err(|e| {
            debug!("Failed to write header: {}", e);
            NanonisError::Io {
                source: e,
                context: "Writing command header".to_string(),
            }
        })?;

        if !body.is_empty() {
            debug!("Sending body ({} bytes)...", body.len());
            self.stream.write_all(&body).map_err(|e| {
                debug!("Failed to write body: {}", e);
                NanonisError::Io {
                    source: e,
                    context: "Writing command body".to_string(),
                }
            })?;
        }

        debug!("Command data sent successfully");

        // Read response header with improved error handling
        debug!("Reading response header ({} bytes)...", HEADER_SIZE);
        let response_header =
            Protocol::read_exact_bytes::<HEADER_SIZE>(&mut self.stream).map_err(|e| {
                debug!("Failed to read response header: {}", e);
                e
            })?;

        debug!("Response header received: {:02x?}", response_header);
        debug!(
            "Response command: {:?}",
            String::from_utf8_lossy(&response_header[0..32]).trim_end_matches('\0')
        );

        // Validate and get body size
        let body_size = Protocol::validate_response_header(&response_header, command)?;
        debug!("Expected response body size: {}", body_size);

        // Read response body with size validation
        let response_body = if body_size > 0 {
            debug!("Reading response body ({} bytes)...", body_size);
            let body = Protocol::read_variable_bytes(&mut self.stream, body_size as usize)
                .map_err(|e| {
                    debug!("Failed to read response body: {}", e);
                    e
                })?;
            debug!(
                "Response body received ({} bytes): {:02x?}",
                body.len(),
                if body.len() <= 100 {
                    &body[..]
                } else {
                    &body[..100]
                }
            );
            body
        } else {
            debug!("No response body expected");
            Vec::new()
        };

        // Parse response with error checking
        debug!("Parsing response with types: {:?}", return_types);
        let result = Protocol::parse_response_with_error_check(&response_body, &return_types)
            .map_err(|e| {
                debug!("Failed to parse response: {}", e);
                e
            })?;

        debug!("=== COMMAND SUCCESS: {} ===", command);
        debug!("Parsed result: {:?}", result);

        Ok(result)
    }
}

impl Drop for NanonisClient {
    fn drop(&mut self) {
        let _ = self.z_ctrl_withdraw(false, Duration::from_secs(2));
        let _ = self.motor_start_move(MotorDirection::ZMinus, 15u16, MotorGroup::Group1, false);
    }
}
