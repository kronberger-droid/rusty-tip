use crate::error::NanonisError;
use crate::protocol::{Protocol, HEADER_SIZE, MAX_RETRY_COUNT};
use crate::types::{BiasVoltage, NanonisValue, Position};
use log::{debug, trace, warn};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

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
                    NanonisError::Io(e)
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
/// use rusty_tip::{NanonisClient, BiasVoltage};
///
/// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
///
/// // Read signal names
/// let signals = client.signal_names_get(false)?;
///
/// // Set bias voltage
/// client.set_bias(BiasVoltage(1.0))?;
///
/// // Read signal values
/// let values = client.signals_val_get(vec![0, 1, 2], true)?;
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
    /// * `addr` - Server address in format "host:port" (e.g., "127.0.0.1:6501")
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

    /// Enable or disable debug logging
    pub fn set_debug(&mut self, debug: bool) {
        self.debug = debug;
    }

    /// Get current connection configuration
    pub fn config(&self) -> &ConnectionConfig {
        &self.config
    }

    /// Send a command and receive response
    pub fn quick_send(
        &mut self,
        command: &str,
        body: &[NanonisValue],
        body_types: &[&str],
        response_types: &[&str],
    ) -> Result<Vec<NanonisValue>, NanonisError> {
        if body.len() != body_types.len() {
            return Err(NanonisError::InvalidCommand(format!(
                "Body length ({}) doesn't match body types length ({})",
                body.len(),
                body_types.len()
            )));
        }

        let response = self.send(command, body, body_types)?;
        if !response.is_empty() {
            let response_data = Protocol::parse_response(&response, response_types)?;
            if self.debug {
                debug!("Response: {response_data:?}");
            }
            Ok(response_data)
        } else {
            debug!("No data returned for command: {command}");
            Ok(vec![])
        }
    }

    fn send(
        &mut self,
        command: &str,
        body: &[NanonisValue],
        body_types: &[&str],
    ) -> Result<Vec<u8>, NanonisError> {
        let mut body_part = Vec::new();

        // Serialize body
        for (i, value) in body.iter().enumerate() {
            let body_type = body_types[i];
            Protocol::serialize_value(value, body_type, &mut body_part)?;
        }

        let body_size = body_part.len() as u32;

        // Create message header
        let mut message = Protocol::create_command_header(command, body_size);

        // Append body
        message.extend_from_slice(&body_part);

        if self.debug {
            trace!("Send message: {message:?}");
        }

        // Send message
        self.stream.write_all(&message)?;
        self.stream.flush()?;

        trace!("Message sent, waiting for response");

        // Read response header (40 bytes)
        let mut header = [0u8; HEADER_SIZE];
        self.stream.read_exact(&mut header)?;

        // Validate header and get body size
        let response_body_size = Protocol::validate_response_header(&header, command)? as usize;

        // Read response body with retry logic
        let mut response_body = vec![0u8; response_body_size];
        let mut bytes_read = 0;
        let mut counter = 0;

        while bytes_read < response_body_size && counter < MAX_RETRY_COUNT {
            match self.stream.read(&mut response_body[bytes_read..]) {
                Ok(n) if n > 0 => bytes_read += n,
                Ok(_) => break, // EOF
                Err(e) => return Err(NanonisError::Io(e)),
            }
            counter += 1;
        }

        if bytes_read < response_body_size {
            warn!("Incomplete response: got {bytes_read} bytes, expected {response_body_size}");
        }

        response_body.truncate(bytes_read);

        if self.debug {
            debug!("Body size: {response_body_size}, received: {bytes_read}");
            trace!("Header: {header:?}");
            trace!("Body: {response_body:?}");
        }

        Ok(response_body)
    }

    // ==================== Type-safe method implementations ====================

    /// Set the bias voltage applied to the scanning probe tip.
    ///
    /// This corresponds to the Nanonis `Bias.Set` command.
    ///
    /// # Arguments
    /// * `voltage` - The bias voltage to set, wrapped in a type-safe [`BiasVoltage`]
    ///
    /// # Errors
    /// Returns `NanonisError` if the command fails or communication times out.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::{NanonisClient, BiasVoltage};
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Set bias to 1.5V
    /// client.set_bias(BiasVoltage(1.5))?;
    ///
    /// // Set bias to -0.5V  
    /// client.set_bias(BiasVoltage(-0.5))?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_bias(&mut self, voltage: BiasVoltage) -> Result<(), NanonisError> {
        self.quick_send("Bias.Set", &[NanonisValue::F32(voltage.0)], &["f"], &[])?;
        Ok(())
    }

    /// Get the current bias voltage applied to the scanning probe tip.
    ///
    /// This corresponds to the Nanonis `Bias.Get` command.
    ///
    /// # Returns
    /// The current bias voltage wrapped in a type-safe [`BiasVoltage`].
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - The command fails or communication times out
    /// - The server returns invalid or missing data
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// let current_bias = client.get_bias()?;
    /// println!("Current bias voltage: {:.3}V", current_bias.0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn get_bias(&mut self) -> Result<BiasVoltage, NanonisError> {
        let result = self.quick_send("Bias.Get", &[], &[], &["f"])?;
        match result.first() {
            Some(value) => Ok(BiasVoltage(value.as_f32()?)),
            None => Err(NanonisError::Protocol("No bias value returned".to_string())),
        }
    }

    /// Get available signal names
    pub fn signal_names_get(&mut self, print: bool) -> Result<Vec<String>, NanonisError> {
        let result = self.quick_send("Signals.NamesGet", &[], &[], &["+*c"])?;
        match result.first() {
            Some(value) => {
                let signal_names = value.as_string_array()?.to_vec();

                if print {
                    Self::print_signal_names(&signal_names);
                }

                Ok(signal_names)
            }
            None => Err(NanonisError::Protocol(
                "No signal names returned".to_string(),
            )),
        }
    }

    /// Helper function for printing signal names
    fn print_signal_names(names: &[String]) {
        log::info!("Available signal names ({} total):", names.len());
        for (index, name) in names.iter().enumerate() {
            log::info!("  {index}: {name}");
        }
    }

    /// Get calibration and offset of a signal by index
    pub fn signals_calibr_get(&mut self, signal_index: i32) -> Result<(f32, f32), NanonisError> {
        let result = self.quick_send(
            "Signals.CalibrGet",
            &[NanonisValue::I32(signal_index)],
            &["i"],
            &["f", "f"],
        )?;
        if result.len() >= 2 {
            Ok((result[0].as_f32()?, result[1].as_f32()?))
        } else {
            Err(NanonisError::Protocol(
                "Invalid calibration response".to_string(),
            ))
        }
    }

    /// Get range limits of a signal by index
    pub fn signals_range_get(&mut self, signal_index: i32) -> Result<(f32, f32), NanonisError> {
        let result = self.quick_send(
            "Signals.RangeGet",
            &[NanonisValue::I32(signal_index)],
            &["i"],
            &["f", "f"],
        )?;
        if result.len() >= 2 {
            Ok((result[0].as_f32()?, result[1].as_f32()?)) // (max, min)
        } else {
            Err(NanonisError::Protocol("Invalid range response".to_string()))
        }
    }

    /// Get current values of signals by index(es)
    pub fn signals_val_get(
        &mut self,
        signal_indexes: Vec<i32>,
        wait_for_newest_data: bool,
    ) -> Result<Vec<f32>, NanonisError> {
        let indexes = signal_indexes;
        let wait_flag = if wait_for_newest_data { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "Signals.ValsGet",
            &[
                NanonisValue::ArrayI32(indexes),
                NanonisValue::U32(wait_flag),
            ],
            &["+*i", "I"],
            &["i", "*f"],
        )?;

        if result.len() >= 2 {
            match &result[1] {
                NanonisValue::ArrayF32(values) => Ok(values.clone()),
                _ => Err(NanonisError::Protocol(
                    "Invalid signal values response".to_string(),
                )),
            }
        } else {
            Err(NanonisError::Protocol(
                "Incomplete signal values response".to_string(),
            ))
        }
    }

    /// Find signal index by name (case-insensitive)
    pub fn find_signal_index(&mut self, signal_name: &str) -> Result<Option<usize>, NanonisError> {
        let signals = self.signal_names_get(false)?;
        let signal_name_lower = signal_name.to_lowercase();

        for (index, name) in signals.iter().enumerate() {
            if name.to_lowercase().contains(&signal_name_lower) {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    /// Read a signal by name (finds index automatically)
    pub fn read_signal_by_name(
        &mut self,
        signal_name: &str,
        wait_for_newest: bool,
    ) -> Result<f32, NanonisError> {
        match self.find_signal_index(signal_name)? {
            Some(index) => {
                let values = self.signals_val_get(vec![index as i32], wait_for_newest)?;
                values
                    .first()
                    .copied()
                    .ok_or_else(|| NanonisError::Protocol("No signal value returned".to_string()))
            }
            None => Err(NanonisError::InvalidCommand(format!(
                "Signal '{signal_name}' not found"
            ))),
        }
    }

    // ==================== AutoApproach Functions ====================

    /// Open the Auto-Approach module
    pub fn auto_approach_open(&mut self) -> Result<(), NanonisError> {
        self.quick_send("AutoApproach.Open", &[], &[], &[])?;
        Ok(())
    }

    /// Start or stop the Z auto-approach procedure
    pub fn auto_approach_on_off_set(&mut self, on_off: bool) -> Result<(), NanonisError> {
        let value = if on_off { 1u16 } else { 0u16 };
        self.quick_send(
            "AutoApproach.OnOffSet",
            &[NanonisValue::U16(value)],
            &["H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the on-off status of the Z auto-approach procedure
    pub fn auto_approach_on_off_get(&mut self) -> Result<bool, NanonisError> {
        let result = self.quick_send("AutoApproach.OnOffGet", &[], &[], &["H"])?;
        match result.first() {
            Some(value) => {
                let status = value.as_u16()?;
                Ok(status == 1)
            }
            None => Err(NanonisError::Protocol(
                "No auto-approach status returned".to_string(),
            )),
        }
    }

    /// Auto-approach and wait until completion (convenience function)
    pub fn auto_approach_and_wait(&mut self) -> Result<(), NanonisError> {
        log::info!("Starting auto-approach...");

        // Open auto-approach module
        self.auto_approach_open()?;

        // Wait a bit for module to initialize
        std::thread::sleep(std::time::Duration::from_millis(1000));

        // Start auto-approach
        self.auto_approach_on_off_set(true)?;

        log::info!("Waiting for auto-approach to complete...");

        // Wait until auto-approach completes
        loop {
            let is_running = self.auto_approach_on_off_get()?;
            if !is_running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        log::info!("Auto-approach finished");
        Ok(())
    }

    // ==================== Z-Controller Functions ====================

    /// Withdraw the tip
    /// Switches off the Z-Controller and fully withdraws the tip to upper limit
    pub fn z_ctrl_withdraw(
        &mut self,
        wait_until_finished: bool,
        timeout_ms: i32,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u32 } else { 0u32 };
        self.quick_send(
            "ZCtrl.Withdraw",
            &[NanonisValue::U32(wait_flag), NanonisValue::I32(timeout_ms)],
            &["I", "i"],
            &[],
        )?;
        Ok(())
    }

    // ==================== FolMe Functions ====================

    /// Get current X,Y tip coordinates
    /// Returns the X,Y tip coordinates (oversampled during the Acquisition Period time)
    pub fn folme_xy_pos_get(
        &mut self,
        wait_for_newest_data: bool,
    ) -> Result<Position, NanonisError> {
        let wait_flag = if wait_for_newest_data { 1u32 } else { 0u32 };
        let result = self.quick_send(
            "FolMe.XYPosGet",
            &[NanonisValue::U32(wait_flag)],
            &["I"],
            &["d", "d"],
        )?;

        if result.len() >= 2 {
            let x = result[0].as_f64()?;
            let y = result[1].as_f64()?;
            Ok(Position { x, y })
        } else {
            Err(NanonisError::Protocol(
                "Invalid XY position response".to_string(),
            ))
        }
    }

    /// Set XY position with type safety
    pub fn folme_xy_pos_set(
        &mut self,
        position: Position,
        wait_end: bool,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "FolMe.XYPosSet",
            &[
                NanonisValue::F64(position.x),
                NanonisValue::F64(position.y),
                NanonisValue::U32(if wait_end { 1 } else { 0 }),
            ],
            &["d", "d", "I"],
            &[],
        )?;
        Ok(())
    }

    // ==================== Motor Functions ====================

    /// Move the coarse positioning device (motor, piezo actuator)
    /// Direction: 0=X+, 1=X-, 2=Y+, 3=Y-, 4=Z+, 5=Z-
    /// Group: 0=Group1, 1=Group2, 2=Group3, 3=Group4, 4=Group5, 5=Group6
    pub fn motor_start_move(
        &mut self,
        direction: u32,
        number_of_steps: u16,
        group: u32,
        wait_until_finished: bool,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u32 } else { 0u32 };
        self.quick_send(
            "Motor.StartMove",
            &[
                NanonisValue::U32(direction),
                NanonisValue::U16(number_of_steps),
                NanonisValue::U32(group),
                NanonisValue::U32(wait_flag),
            ],
            &["I", "H", "I", "I"],
            &[],
        )?;
        Ok(())
    }

    /// Move the coarse positioning device in closed loop
    /// absolute_relative: 0=relative, 1=absolute movement
    /// Group: 0=Group1, 1=Group2, 2=Group3, 3=Group4, 4=Group5, 5=Group6
    pub fn motor_start_closed_loop(
        &mut self,
        absolute_relative: u32,
        target_x_m: f64,
        target_y_m: f64,
        target_z_m: f64,
        wait_until_finished: bool,
        group: u32,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u32 } else { 0u32 };
        self.quick_send(
            "Motor.StartClosedLoop",
            &[
                NanonisValue::U32(absolute_relative),
                NanonisValue::F64(target_x_m),
                NanonisValue::F64(target_y_m),
                NanonisValue::F64(target_z_m),
                NanonisValue::U32(wait_flag),
                NanonisValue::U32(group),
            ],
            &["I", "d", "d", "d", "I", "I"],
            &[],
        )?;
        Ok(())
    }

    /// Stop the motor motion
    pub fn motor_stop_move(&mut self) -> Result<(), NanonisError> {
        self.quick_send("Motor.StopMove", &[], &[], &[])?;
        Ok(())
    }

    /// Get the positions of the motor control module
    /// Group: 0=Group1, 1=Group2, 2=Group3, 3=Group4, 4=Group5, 5=Group6
    /// timeout_ms: timeout in milliseconds (recommended: 500ms)
    pub fn motor_pos_get(
        &mut self,
        group: u32,
        timeout_ms: u32,
    ) -> Result<(f64, f64, f64), NanonisError> {
        let result = self.quick_send(
            "Motor.PosGet",
            &[NanonisValue::U32(group), NanonisValue::U32(timeout_ms)],
            &["I", "I"],
            &["d", "d", "d"],
        )?;

        if result.len() >= 3 {
            let x = result[0].as_f64()?;
            let y = result[1].as_f64()?;
            let z = result[2].as_f64()?;
            Ok((x, y, z))
        } else {
            Err(NanonisError::Protocol(
                "Invalid motor position response".to_string(),
            ))
        }
    }

    /// Get step counter values and optionally reset them
    /// Available only on Attocube ANC150 devices
    pub fn motor_step_counter_get(
        &mut self,
        reset_x: bool,
        reset_y: bool,
        reset_z: bool,
    ) -> Result<(i32, i32, i32), NanonisError> {
        let reset_x_flag = if reset_x { 1u32 } else { 0u32 };
        let reset_y_flag = if reset_y { 1u32 } else { 0u32 };
        let reset_z_flag = if reset_z { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "Motor.StepCounterGet",
            &[
                NanonisValue::U32(reset_x_flag),
                NanonisValue::U32(reset_y_flag),
                NanonisValue::U32(reset_z_flag),
            ],
            &["I", "I", "I"],
            &["i", "i", "i"],
        )?;

        if result.len() >= 3 {
            let step_x = result[0].as_i32()?;
            let step_y = result[1].as_i32()?;
            let step_z = result[2].as_i32()?;
            Ok((step_x, step_y, step_z))
        } else {
            Err(NanonisError::Protocol(
                "Invalid step counter response".to_string(),
            ))
        }
    }

    /// Get frequency and amplitude of the motor control module
    /// Available only for PD5, PMD4, and Attocube ANC150 devices
    /// axis: 0=Default, 1=X, 2=Y, 3=Z
    pub fn motor_freq_amp_get(&mut self, axis: u16) -> Result<(f32, f32), NanonisError> {
        let result = self.quick_send(
            "Motor.FreqAmpGet",
            &[NanonisValue::U16(axis)],
            &["H"],
            &["f", "f"],
        )?;

        if result.len() >= 2 {
            let frequency = result[0].as_f32()?;
            let amplitude = result[1].as_f32()?;
            Ok((frequency, amplitude))
        } else {
            Err(NanonisError::Protocol(
                "Invalid frequency/amplitude response".to_string(),
            ))
        }
    }

    /// Set frequency and amplitude of the motor control module
    /// Available only for PD5, PMD4, and Attocube ANC150 devices
    /// axis: 0=All, 1=X, 2=Y, 3=Z
    pub fn motor_freq_amp_set(
        &mut self,
        frequency_hz: f32,
        amplitude_v: f32,
        axis: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Motor.FreqAmpSet",
            &[
                NanonisValue::F32(frequency_hz),
                NanonisValue::F32(amplitude_v),
                NanonisValue::U16(axis),
            ],
            &["f", "f", "H"],
            &[],
        )?;
        Ok(())
    }

    // ==================== OsciHR (Oscilloscope High Resolution) Functions ====================

    /// Set the measured signal index of the selected channel from the Oscilloscope High Resolution
    pub fn osci_hr_ch_set(
        &mut self,
        osci_index: i32,
        signal_index: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.ChSet",
            &[
                NanonisValue::I32(osci_index),
                NanonisValue::I32(signal_index),
            ],
            &["i", "i"],
            &[],
        )?;
        Ok(())
    }

    /// Get the measured signal index of the selected channel from the Oscilloscope High Resolution
    pub fn osci_hr_ch_get(&mut self, osci_index: i32) -> Result<i32, NanonisError> {
        let result = self.quick_send(
            "OsciHR.ChGet",
            &[NanonisValue::I32(osci_index)],
            &["i"],
            &["i"],
        )?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No signal index returned".to_string(),
            )),
        }
    }

    /// Set the oversampling index of the Oscilloscope High Resolution
    pub fn osci_hr_oversampl_set(&mut self, oversampling_index: i32) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.OversamplSet",
            &[NanonisValue::I32(oversampling_index)],
            &["i"],
            &[],
        )?;
        Ok(())
    }

    /// Get the oversampling index of the Oscilloscope High Resolution
    pub fn osci_hr_oversampl_get(&mut self) -> Result<i32, NanonisError> {
        let result = self.quick_send("OsciHR.OversamplGet", &[], &[], &["i"])?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No oversampling index returned".to_string(),
            )),
        }
    }

    /// Set the calibration mode of the selected channel from the Oscilloscope High Resolution
    /// calibration_mode: 0 = Raw values, 1 = Calibrated values
    pub fn osci_hr_calibr_mode_set(
        &mut self,
        osci_index: i32,
        calibration_mode: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.CalibrModeSet",
            &[
                NanonisValue::I32(osci_index),
                NanonisValue::U16(calibration_mode),
            ],
            &["i", "H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the calibration mode of the selected channel from the Oscilloscope High Resolution
    /// Returns: 0 = Raw values, 1 = Calibrated values
    pub fn osci_hr_calibr_mode_get(&mut self, osci_index: i32) -> Result<u16, NanonisError> {
        let result = self.quick_send(
            "OsciHR.CalibrModeGet",
            &[NanonisValue::I32(osci_index)],
            &["i"],
            &["H"],
        )?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No calibration mode returned".to_string(),
            )),
        }
    }

    /// Set the number of samples to acquire in the Oscilloscope High Resolution
    pub fn osci_hr_samples_set(&mut self, number_of_samples: i32) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.SamplesSet",
            &[NanonisValue::I32(number_of_samples)],
            &["i"],
            &[],
        )?;
        Ok(())
    }

    /// Get the number of samples to acquire in the Oscilloscope High Resolution
    pub fn osci_hr_samples_get(&mut self) -> Result<i32, NanonisError> {
        let result = self.quick_send("OsciHR.SamplesGet", &[], &[], &["i"])?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No sample count returned".to_string(),
            )),
        }
    }

    /// Set the Pre-Trigger Samples or Seconds in the Oscilloscope High Resolution
    pub fn osci_hr_pre_trig_set(
        &mut self,
        pre_trigger_samples: u32,
        pre_trigger_s: f64,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.PreTrigSet",
            &[
                NanonisValue::U32(pre_trigger_samples),
                NanonisValue::F64(pre_trigger_s),
            ],
            &["I", "d"],
            &[],
        )?;
        Ok(())
    }

    /// Get the Pre-Trigger Samples in the Oscilloscope High Resolution
    pub fn osci_hr_pre_trig_get(&mut self) -> Result<i32, NanonisError> {
        let result = self.quick_send("OsciHR.PreTrigGet", &[], &[], &["i"])?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No pre-trigger samples returned".to_string(),
            )),
        }
    }

    /// Start the Oscilloscope High Resolution module
    pub fn osci_hr_run(&mut self) -> Result<(), NanonisError> {
        self.quick_send("OsciHR.Run", &[], &[], &[])?;
        Ok(())
    }

    /// Get the graph data of the selected channel from the Oscilloscope High Resolution
    /// data_to_get: 0 = Current returns the currently displayed data, 1 = Next trigger waits for the next trigger
    /// Returns: (timestamp, time_delta, data_values, timeout_occurred)
    pub fn osci_hr_osci_data_get(
        &mut self,
        osci_index: i32,
        data_to_get: u16,
        timeout_s: f64,
    ) -> Result<(String, f64, Vec<f32>, bool), NanonisError> {
        let result = self.quick_send(
            "OsciHR.OsciDataGet",
            &[
                NanonisValue::I32(osci_index),
                NanonisValue::U16(data_to_get),
                NanonisValue::F64(timeout_s),
            ],
            &["i", "H", "d"],
            &["i", "*-c", "d", "i", "*f", "I"],
        )?;

        if result.len() >= 6 {
            let timestamp = result[1].as_string()?.to_string();
            let time_delta = result[2].as_f64()?;
            let data_values = result[4].as_f32_array()?.to_vec();
            let timeout_occurred = result[5].as_u32()? == 1;
            Ok((timestamp, time_delta, data_values, timeout_occurred))
        } else {
            Err(NanonisError::Protocol(
                "Invalid oscilloscope data response".to_string(),
            ))
        }
    }

    /// Set the trigger mode in the Oscilloscope High Resolution
    /// trigger_mode: 0 = Immediate, 1 = Level, 2 = Digital
    pub fn osci_hr_trig_mode_set(&mut self, trigger_mode: u16) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigModeSet",
            &[NanonisValue::U16(trigger_mode)],
            &["H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the trigger mode in the Oscilloscope High Resolution
    /// Returns: 0 = Immediate, 1 = Level, 2 = Digital
    pub fn osci_hr_trig_mode_get(&mut self) -> Result<u16, NanonisError> {
        let result = self.quick_send("OsciHR.TrigModeGet", &[], &[], &["H"])?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No trigger mode returned".to_string(),
            )),
        }
    }

    /// Set the Level Trigger Channel index in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_ch_set(
        &mut self,
        level_trigger_channel_index: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigLevChSet",
            &[NanonisValue::I32(level_trigger_channel_index)],
            &["i"],
            &[],
        )?;
        Ok(())
    }

    /// Get the Level Trigger Channel index in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_ch_get(&mut self) -> Result<i32, NanonisError> {
        let result = self.quick_send("OsciHR.TrigLevChGet", &[], &[], &["i"])?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No level trigger channel returned".to_string(),
            )),
        }
    }

    /// Set the Level Trigger value in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_val_set(
        &mut self,
        level_trigger_value: f64,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigLevValSet",
            &[NanonisValue::F64(level_trigger_value)],
            &["d"],
            &[],
        )?;
        Ok(())
    }

    /// Get the Level Trigger value in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_val_get(&mut self) -> Result<f64, NanonisError> {
        let result = self.quick_send("OsciHR.TrigLevValGet", &[], &[], &["d"])?;
        match result.first() {
            Some(value) => Ok(value.as_f64()?),
            None => Err(NanonisError::Protocol(
                "No level trigger value returned".to_string(),
            )),
        }
    }

    /// Set the Level Trigger Hysteresis in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_hyst_set(
        &mut self,
        level_trigger_hysteresis: f64,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigLevHystSet",
            &[NanonisValue::F64(level_trigger_hysteresis)],
            &["d"],
            &[],
        )?;
        Ok(())
    }

    /// Get the Level Trigger Hysteresis in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_hyst_get(&mut self) -> Result<f64, NanonisError> {
        let result = self.quick_send("OsciHR.TrigLevHystGet", &[], &[], &["d"])?;
        match result.first() {
            Some(value) => Ok(value.as_f64()?),
            None => Err(NanonisError::Protocol(
                "No level trigger hysteresis returned".to_string(),
            )),
        }
    }

    /// Set the Level Trigger Slope in the Oscilloscope High Resolution
    /// level_trigger_slope: 0 = Rising, 1 = Falling
    pub fn osci_hr_trig_lev_slope_set(
        &mut self,
        level_trigger_slope: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigLevSlopeSet",
            &[NanonisValue::U16(level_trigger_slope)],
            &["H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the Level Trigger Slope in the Oscilloscope High Resolution
    /// Returns: 0 = Rising, 1 = Falling
    pub fn osci_hr_trig_lev_slope_get(&mut self) -> Result<u16, NanonisError> {
        let result = self.quick_send("OsciHR.TrigLevSlopeGet", &[], &[], &["H"])?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No level trigger slope returned".to_string(),
            )),
        }
    }

    /// Set the Digital Trigger Channel in the Oscilloscope High Resolution
    /// channel_index: 0-35 (LS-DIO: 0-31, HS-DIO: 32-35)
    pub fn osci_hr_trig_dig_ch_set(
        &mut self,
        digital_trigger_channel_index: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigDigChSet",
            &[NanonisValue::I32(digital_trigger_channel_index)],
            &["i"],
            &[],
        )?;
        Ok(())
    }

    /// Get the Digital Trigger Channel in the Oscilloscope High Resolution
    /// Returns channel index: 0-35 (LS-DIO: 0-31, HS-DIO: 32-35)
    pub fn osci_hr_trig_dig_ch_get(&mut self) -> Result<i32, NanonisError> {
        let result = self.quick_send("OsciHR.TrigDigChGet", &[], &[], &["i"])?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No digital trigger channel returned".to_string(),
            )),
        }
    }

    /// Set the Trigger Arming Mode in the Oscilloscope High Resolution
    /// trigger_arming_mode: 0 = Single shot, 1 = Continuous
    pub fn osci_hr_trig_arm_mode_set(
        &mut self,
        trigger_arming_mode: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigArmModeSet",
            &[NanonisValue::U16(trigger_arming_mode)],
            &["H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the Trigger Arming Mode in the Oscilloscope High Resolution
    /// Returns: 0 = Single shot, 1 = Continuous
    pub fn osci_hr_trig_arm_mode_get(&mut self) -> Result<u16, NanonisError> {
        let result = self.quick_send("OsciHR.TrigArmModeGet", &[], &[], &["H"])?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No trigger arming mode returned".to_string(),
            )),
        }
    }

    /// Set the Digital Trigger Slope in the Oscilloscope High Resolution
    /// digital_trigger_slope: 0 = Rising, 1 = Falling
    pub fn osci_hr_trig_dig_slope_set(
        &mut self,
        digital_trigger_slope: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigDigSlopeSet",
            &[NanonisValue::U16(digital_trigger_slope)],
            &["H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the Digital Trigger Slope in the Oscilloscope High Resolution
    /// Returns: 0 = Rising, 1 = Falling
    pub fn osci_hr_trig_dig_slope_get(&mut self) -> Result<u16, NanonisError> {
        let result = self.quick_send("OsciHR.TrigDigSlopeGet", &[], &[], &["H"])?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No digital trigger slope returned".to_string(),
            )),
        }
    }

    /// Rearm the trigger in the Oscilloscope High Resolution module
    pub fn osci_hr_trig_rearm(&mut self) -> Result<(), NanonisError> {
        self.quick_send("OsciHR.TrigRearm", &[], &[], &[])?;
        Ok(())
    }

    /// Show or hide the PSD section of the Oscilloscope High Resolution
    /// show_psd_section: 0 = Hide, 1 = Show
    pub fn osci_hr_psd_show(&mut self, show_psd_section: u32) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.PSDShow",
            &[NanonisValue::U32(show_psd_section)],
            &["I"],
            &[],
        )?;
        Ok(())
    }

    /// Set the PSD Weighting in the Oscilloscope High Resolution
    /// psd_weighting: 0 = Linear, 1 = Exponential
    pub fn osci_hr_psd_weight_set(&mut self, psd_weighting: u16) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.PSDWeightSet",
            &[NanonisValue::U16(psd_weighting)],
            &["H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the PSD Weighting in the Oscilloscope High Resolution
    /// Returns: 0 = Linear, 1 = Exponential
    pub fn osci_hr_psd_weight_get(&mut self) -> Result<u16, NanonisError> {
        let result = self.quick_send("OsciHR.PSDWeightGet", &[], &[], &["H"])?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No PSD weighting returned".to_string(),
            )),
        }
    }

    /// Set the PSD Window Type in the Oscilloscope High Resolution
    /// psd_window_type: 0 = None, 1 = Hanning, 2 = Hamming, etc.
    pub fn osci_hr_psd_window_set(&mut self, psd_window_type: u16) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.PSDWindowSet",
            &[NanonisValue::U16(psd_window_type)],
            &["H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the PSD Window Type in the Oscilloscope High Resolution
    /// Returns: 0 = None, 1 = Hanning, 2 = Hamming, etc.
    pub fn osci_hr_psd_window_get(&mut self) -> Result<u16, NanonisError> {
        let result = self.quick_send("OsciHR.PSDWindowGet", &[], &[], &["H"])?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No PSD window type returned".to_string(),
            )),
        }
    }

    /// Set the PSD Averaging Type in the Oscilloscope High Resolution
    /// psd_averaging_type: 0 = None, 1 = Vector, 2 = RMS, 3 = Peak hold
    pub fn osci_hr_psd_avrg_type_set(
        &mut self,
        psd_averaging_type: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.PSDAvrgTypeSet",
            &[NanonisValue::U16(psd_averaging_type)],
            &["H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the PSD Averaging Type in the Oscilloscope High Resolution
    /// Returns: 0 = None, 1 = Vector, 2 = RMS, 3 = Peak hold
    pub fn osci_hr_psd_avrg_type_get(&mut self) -> Result<u16, NanonisError> {
        let result = self.quick_send("OsciHR.PSDAvrgTypeGet", &[], &[], &["H"])?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No PSD averaging type returned".to_string(),
            )),
        }
    }

    /// Set the PSD Averaging Count used by the RMS and Vector averaging types
    pub fn osci_hr_psd_avrg_count_set(
        &mut self,
        psd_averaging_count: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.PSDAvrgCountSet",
            &[NanonisValue::I32(psd_averaging_count)],
            &["i"],
            &[],
        )?;
        Ok(())
    }

    /// Get the PSD Averaging Count used by the RMS and Vector averaging types
    pub fn osci_hr_psd_avrg_count_get(&mut self) -> Result<i32, NanonisError> {
        let result = self.quick_send("OsciHR.PSDAvrgCountGet", &[], &[], &["i"])?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No PSD averaging count returned".to_string(),
            )),
        }
    }

    /// Restart the PSD averaging process in the Oscilloscope High Resolution module
    pub fn osci_hr_psd_avrg_restart(&mut self) -> Result<(), NanonisError> {
        self.quick_send("OsciHR.PSDAvrgRestart", &[], &[], &[])?;
        Ok(())
    }

    // ==================== Scan Functions ====================

    /// Start, stop, pause or resume a scan
    /// scan_action: 0=Start, 1=Stop, 2=Pause, 3=Resume, 4=Freeze, 5=Unfreeze, 6=Go to Center
    /// scan_direction: 1=Up, 0=Down
    pub fn scan_action(
        &mut self,
        scan_action: u16,
        scan_direction: u32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Scan.Action",
            &[
                NanonisValue::U16(scan_action),
                NanonisValue::U32(scan_direction),
            ],
            &["H", "I"],
            &[],
        )?;
        Ok(())
    }

    /// Get scan status (running or not)
    /// Returns: true if scan is running, false if not running
    pub fn scan_status_get(&mut self) -> Result<bool, NanonisError> {
        let result = self.quick_send("Scan.StatusGet", &[], &[], &["I"])?;
        match result.first() {
            Some(value) => Ok(value.as_u32()? == 1),
            None => Err(NanonisError::Protocol(
                "No scan status returned".to_string(),
            )),
        }
    }

    /// Wait for the End-of-Scan
    /// timeout_ms: timeout in milliseconds, use -1 for indefinite wait
    /// Returns: (timeout_occurred, file_path)
    pub fn scan_wait_end_of_scan(
        &mut self,
        timeout_ms: i32,
    ) -> Result<(bool, String), NanonisError> {
        let result = self.quick_send(
            "Scan.WaitEndOfScan",
            &[NanonisValue::I32(timeout_ms)],
            &["i"],
            &["I", "I", "*-c"],
        )?;

        if result.len() >= 3 {
            let timeout_occurred = result[0].as_u32()? == 1;
            let file_path = result[2].as_string()?.to_string();
            Ok((timeout_occurred, file_path))
        } else {
            Err(NanonisError::Protocol(
                "Invalid scan wait response".to_string(),
            ))
        }
    }

    /// Configure the scan frame parameters
    pub fn scan_frame_set(
        &mut self,
        center_x_m: f32,
        center_y_m: f32,
        width_m: f32,
        height_m: f32,
        angle_deg: f32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Scan.FrameSet",
            &[
                NanonisValue::F32(center_x_m),
                NanonisValue::F32(center_y_m),
                NanonisValue::F32(width_m),
                NanonisValue::F32(height_m),
                NanonisValue::F32(angle_deg),
            ],
            &["f", "f", "f", "f", "f"],
            &[],
        )?;
        Ok(())
    }

    /// Get the scan frame parameters
    /// Returns: (center_x_m, center_y_m, width_m, height_m, angle_deg)
    pub fn scan_frame_get(&mut self) -> Result<(f32, f32, f32, f32, f32), NanonisError> {
        let result = self.quick_send("Scan.FrameGet", &[], &[], &["f", "f", "f", "f", "f"])?;
        if result.len() >= 5 {
            let center_x = result[0].as_f32()?;
            let center_y = result[1].as_f32()?;
            let width = result[2].as_f32()?;
            let height = result[3].as_f32()?;
            let angle = result[4].as_f32()?;
            Ok((center_x, center_y, width, height, angle))
        } else {
            Err(NanonisError::Protocol(
                "Invalid scan frame response".to_string(),
            ))
        }
    }

    /// Configure the scan buffer parameters
    /// channel_indexes: array of channel indexes (0-23) for recorded channels
    /// pixels: number of pixels per line
    /// lines: number of scan lines
    pub fn scan_buffer_set(
        &mut self,
        channel_indexes: Vec<i32>,
        pixels: i32,
        lines: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Scan.BufferSet",
            &[
                NanonisValue::ArrayI32(channel_indexes),
                NanonisValue::I32(pixels),
                NanonisValue::I32(lines),
            ],
            &["+*i", "i", "i"],
            &[],
        )?;
        Ok(())
    }

    /// Get the scan buffer parameters
    /// Returns: (channel_indexes, pixels, lines)
    pub fn scan_buffer_get(&mut self) -> Result<(Vec<i32>, i32, i32), NanonisError> {
        let result = self.quick_send("Scan.BufferGet", &[], &[], &["i", "*i", "i", "i"])?;
        if result.len() >= 4 {
            let channel_indexes = result[1].as_i32_array()?.to_vec();
            let pixels = result[2].as_i32()?;
            let lines = result[3].as_i32()?;
            Ok((channel_indexes, pixels, lines))
        } else {
            Err(NanonisError::Protocol(
                "Invalid scan buffer response".to_string(),
            ))
        }
    }

    /// Configure scan properties
    /// continuous_scan: 0=no change, 1=On, 2=Off
    /// bouncy_scan: 0=no change, 1=On, 2=Off  
    /// autosave: 0=no change, 1=All, 2=Next, 3=Off
    /// autopaste: 0=no change, 1=All, 2=Next, 3=Off
    pub fn scan_props_set(
        &mut self,
        continuous_scan: u32,
        bouncy_scan: u32,
        autosave: u32,
        series_name: &str,
        comment: &str,
        modules_names: Vec<String>,
        autopaste: u32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Scan.PropsSet",
            &[
                NanonisValue::U32(continuous_scan),
                NanonisValue::U32(bouncy_scan),
                NanonisValue::U32(autosave),
                NanonisValue::String(series_name.to_string()),
                NanonisValue::String(comment.to_string()),
                NanonisValue::ArrayString(modules_names),
                NanonisValue::U32(autopaste),
            ],
            &["I", "I", "I", "+*c", "+*c", "+*c", "I"],
            &[],
        )?;
        Ok(())
    }

    /// Get scan properties - simplified version for core properties
    /// Returns: (continuous_scan, bouncy_scan, autosave, series_name, comment, autopaste)
    /// Note: Full implementation with 2D parameters array would be more complex
    pub fn scan_props_get(
        &mut self,
    ) -> Result<(u32, u32, u32, String, String, u32), NanonisError> {
        let result = self.quick_send(
            "Scan.PropsGet",
            &[],
            &[],
            &["I", "I", "I", "i", "*-c", "i", "*-c", "i", "i", "*+c", "i", "*+i", "i", "i", "*+c", "I"],
        )?;

        if result.len() >= 16 {
            let continuous_scan = result[0].as_u32()?;
            let bouncy_scan = result[1].as_u32()?;
            let autosave = result[2].as_u32()?;
            let series_name = result[4].as_string()?.to_string();
            let comment = result[6].as_string()?.to_string();
            let autopaste = result[15].as_u32()?;

            Ok((continuous_scan, bouncy_scan, autosave, series_name, comment, autopaste))
        } else {
            Err(NanonisError::Protocol(
                "Invalid scan properties response".to_string(),
            ))
        }
    }

    /// Configure scan speed parameters
    pub fn scan_speed_set(
        &mut self,
        forward_linear_speed_m_s: f32,
        backward_linear_speed_m_s: f32,
        forward_time_per_line_s: f32,
        backward_time_per_line_s: f32,
        keep_parameter_constant: u16,
        speed_ratio: f32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Scan.SpeedSet",
            &[
                NanonisValue::F32(forward_linear_speed_m_s),
                NanonisValue::F32(backward_linear_speed_m_s),
                NanonisValue::F32(forward_time_per_line_s),
                NanonisValue::F32(backward_time_per_line_s),
                NanonisValue::U16(keep_parameter_constant),
                NanonisValue::F32(speed_ratio),
            ],
            &["f", "f", "f", "f", "H", "f"],
            &[],
        )?;
        Ok(())
    }

    /// Get scan speed parameters
    /// Returns: (forward_linear_speed_m_s, backward_linear_speed_m_s, forward_time_per_line_s, 
    ///          backward_time_per_line_s, keep_parameter_constant, speed_ratio)
    pub fn scan_speed_get(&mut self) -> Result<(f32, f32, f32, f32, u16, f32), NanonisError> {
        let result = self.quick_send("Scan.SpeedGet", &[], &[], &["f", "f", "f", "f", "H", "f"])?;
        if result.len() >= 6 {
            let forward_speed = result[0].as_f32()?;
            let backward_speed = result[1].as_f32()?;
            let forward_time = result[2].as_f32()?;
            let backward_time = result[3].as_f32()?;
            let keep_param = result[4].as_u16()?;
            let speed_ratio = result[5].as_f32()?;
            Ok((
                forward_speed,
                backward_speed,
                forward_time,
                backward_time,
                keep_param,
                speed_ratio,
            ))
        } else {
            Err(NanonisError::Protocol(
                "Invalid scan speed response".to_string(),
            ))
        }
    }

    /// Get scan frame data for the selected channel
    /// channel_index: index of the channel to get data from
    /// data_direction: 1=forward, 0=backward
    /// Returns: (channel_name, rows, columns, scan_data, scan_direction_up)
    pub fn scan_frame_data_grab(
        &mut self,
        channel_index: u32,
        data_direction: u32,
    ) -> Result<(String, i32, i32, Vec<Vec<f32>>, bool), NanonisError> {
        let result = self.quick_send(
            "Scan.FrameDataGrab",
            &[
                NanonisValue::U32(channel_index),
                NanonisValue::U32(data_direction),
            ],
            &["I", "I"],
            &["i", "*-c", "i", "i", "2f", "I"],
        )?;

        if result.len() >= 6 {
            let channel_name = result[1].as_string()?.to_string();
            let rows = result[2].as_i32()?;
            let columns = result[3].as_i32()?;
            
            // Extract the 2D scan data from result[4]
            let scan_data = if let Ok(data_2d) = result[4].as_f32_2d_array() {
                data_2d.clone()
            } else {
                // If 2D array parsing fails, create empty array with correct dimensions
                vec![vec![0.0; columns as usize]; rows as usize]
            };
            
            let scan_direction_up = result[5].as_u32()? == 1;

            Ok((channel_name, rows, columns, scan_data, scan_direction_up))
        } else {
            Err(NanonisError::Protocol(
                "Invalid scan frame data response".to_string(),
            ))
        }
    }

    /// Get current scan X,Y position
    /// wait_newest_data: if true, waits for fresh data
    /// Returns: (x_m, y_m)
    pub fn scan_xy_pos_get(&mut self, wait_newest_data: bool) -> Result<(f32, f32), NanonisError> {
        let wait_flag = if wait_newest_data { 1u32 } else { 0u32 };
        let result = self.quick_send(
            "Scan.XYPosGet",
            &[NanonisValue::U32(wait_flag)],
            &["I"],
            &["f", "f"],
        )?;

        if result.len() >= 2 {
            let x = result[0].as_f32()?;
            let y = result[1].as_f32()?;
            Ok((x, y))
        } else {
            Err(NanonisError::Protocol(
                "Invalid scan XY position response".to_string(),
            ))
        }
    }

    /// Save current scan data to file
    /// wait_until_saved: if true, waits for save completion
    /// timeout_ms: timeout in milliseconds, -1 for indefinite
    /// Returns: true if timeout occurred
    pub fn scan_save(
        &mut self,
        wait_until_saved: bool,
        timeout_ms: i32,
    ) -> Result<bool, NanonisError> {
        let wait_flag = if wait_until_saved { 1u32 } else { 0u32 };
        let result = self.quick_send(
            "Scan.Save",
            &[NanonisValue::U32(wait_flag), NanonisValue::I32(timeout_ms)],
            &["I", "i"],
            &["I"],
        )?;

        match result.first() {
            Some(value) => Ok(value.as_u32()? == 1),
            None => Err(NanonisError::Protocol(
                "No scan save status returned".to_string(),
            )),
        }
    }

    /// Paste current scan data to background
    /// wait_until_pasted: if true, waits for paste completion
    /// timeout_ms: timeout in milliseconds, -1 for indefinite
    /// Returns: true if timeout occurred
    pub fn scan_background_paste(
        &mut self,
        wait_until_pasted: bool,
        timeout_ms: i32,
    ) -> Result<bool, NanonisError> {
        let wait_flag = if wait_until_pasted { 1u32 } else { 0u32 };
        let result = self.quick_send(
            "Scan.BackgroundPaste",
            &[NanonisValue::U32(wait_flag), NanonisValue::I32(timeout_ms)],
            &["I", "i"],
            &["I"],
        )?;

        match result.first() {
            Some(value) => Ok(value.as_u32()? == 1),
            None => Err(NanonisError::Protocol(
                "No scan paste status returned".to_string(),
            )),
        }
    }

    /// Delete background data
    /// wait_until_deleted: if true, waits for deletion completion
    /// timeout_ms: timeout in milliseconds, -1 for indefinite
    /// which_background: 0=latest background, 1=all backgrounds
    /// Returns: true if timeout occurred
    pub fn scan_background_delete(
        &mut self,
        wait_until_deleted: bool,
        timeout_ms: i32,
        which_background: u32,
    ) -> Result<bool, NanonisError> {
        let wait_flag = if wait_until_deleted { 1u32 } else { 0u32 };
        let result = self.quick_send(
            "Scan.BackgroundDelete",
            &[
                NanonisValue::U32(wait_flag),
                NanonisValue::I32(timeout_ms),
                NanonisValue::U32(which_background),
            ],
            &["I", "i", "I"],
            &["I"],
        )?;

        match result.first() {
            Some(value) => Ok(value.as_u32()? == 1),
            None => Err(NanonisError::Protocol(
                "No scan delete status returned".to_string(),
            )),
        }
    }

    /// Get the Power Spectral Density data from the Oscilloscope High Resolution
    /// data_to_get: 0 = Current returns the currently displayed data, 1 = Next trigger waits for the next trigger
    /// Returns: (f0, df, psd_data, timeout_occurred)
    pub fn osci_hr_psd_data_get(
        &mut self,
        data_to_get: u16,
        timeout_s: f64,
    ) -> Result<(f64, f64, Vec<f64>, bool), NanonisError> {
        let result = self.quick_send(
            "OsciHR.PSDDataGet",
            &[NanonisValue::U16(data_to_get), NanonisValue::F64(timeout_s)],
            &["H", "d"],
            &["d", "d", "i", "*d", "I"],
        )?;

        if result.len() >= 5 {
            let f0 = result[0].as_f64()?;
            let df = result[1].as_f64()?;
            let psd_data = result[3].as_f64_array()?.to_vec();
            let timeout_occurred = result[4].as_u32()? == 1;
            Ok((f0, df, psd_data, timeout_occurred))
        } else {
            Err(NanonisError::Protocol(
                "Invalid PSD data response".to_string(),
            ))
        }
    }

    // ==================== Osci1T (Oscilloscope 1-Channel) Functions ====================

    /// Set the channel to display in the Oscilloscope 1-Channel
    /// channel_index: 0-23, corresponds to signals assigned to the 24 slots in the Signals Manager
    pub fn osci1t_ch_set(&mut self, channel_index: i32) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci1T.ChSet",
            &[NanonisValue::I32(channel_index)],
            &["i"],
            &[],
        )?;
        Ok(())
    }

    /// Get the channel displayed in the Oscilloscope 1-Channel
    /// Returns: channel index (0-23)
    pub fn osci1t_ch_get(&mut self) -> Result<i32, NanonisError> {
        let result = self.quick_send("Osci1T.ChGet", &[], &[], &["i"])?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No channel index returned".to_string(),
            )),
        }
    }

    /// Set the timebase in the Oscilloscope 1-Channel
    /// Use osci1t_timebase_get() first to obtain available timebases, then use the index
    pub fn osci1t_timebase_set(&mut self, timebase_index: i32) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci1T.TimebaseSet",
            &[NanonisValue::I32(timebase_index)],
            &["i"],
            &[],
        )?;
        Ok(())
    }

    /// Get the timebase in the Oscilloscope 1-Channel
    /// Returns: (timebase_index, timebases_array)
    pub fn osci1t_timebase_get(&mut self) -> Result<(i32, Vec<f32>), NanonisError> {
        let result = self.quick_send("Osci1T.TimebaseGet", &[], &[], &["i", "i", "*f"])?;
        if result.len() >= 3 {
            let timebase_index = result[0].as_i32()?;
            let timebases = result[2].as_f32_array()?.to_vec();
            Ok((timebase_index, timebases))
        } else {
            Err(NanonisError::Protocol(
                "Invalid timebase response".to_string(),
            ))
        }
    }

    /// Set the trigger configuration in the Oscilloscope 1-Channel
    /// trigger_mode: 0 = Immediate, 1 = Level, 2 = Auto
    /// trigger_slope: 0 = Falling, 1 = Rising
    pub fn osci1t_trig_set(
        &mut self,
        trigger_mode: u16,
        trigger_slope: u16,
        trigger_level: f32,
        trigger_hysteresis: f32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci1T.TrigSet",
            &[
                NanonisValue::U16(trigger_mode),
                NanonisValue::U16(trigger_slope),
                NanonisValue::F32(trigger_level),
                NanonisValue::F32(trigger_hysteresis),
            ],
            &["H", "H", "f", "f"],
            &[],
        )?;
        Ok(())
    }

    /// Get the trigger configuration in the Oscilloscope 1-Channel
    /// Returns: (trigger_mode, trigger_slope, trigger_level, trigger_hysteresis)
    pub fn osci1t_trig_get(&mut self) -> Result<(u16, u16, f64, f64), NanonisError> {
        let result = self.quick_send("Osci1T.TrigGet", &[], &[], &["H", "H", "d", "d"])?;
        if result.len() >= 4 {
            let trigger_mode = result[0].as_u16()?;
            let trigger_slope = result[1].as_u16()?;
            let trigger_level = result[2].as_f64()?;
            let trigger_hysteresis = result[3].as_f64()?;
            Ok((
                trigger_mode,
                trigger_slope,
                trigger_level,
                trigger_hysteresis,
            ))
        } else {
            Err(NanonisError::Protocol(
                "Invalid trigger configuration response".to_string(),
            ))
        }
    }

    /// Start the Oscilloscope 1-Channel
    pub fn osci1t_run(&mut self) -> Result<(), NanonisError> {
        self.quick_send("Osci1T.Run", &[], &[], &[])?;
        Ok(())
    }

    /// Get the graph data from the Oscilloscope 1-Channel
    /// data_to_get: 0 = Current, 1 = Next trigger, 2 = Wait 2 triggers
    /// Returns: (t0, dt, data_values)
    pub fn osci1t_data_get(
        &mut self,
        data_to_get: u16,
    ) -> Result<(f64, f64, i32, Vec<f64>), NanonisError> {
        let result = self.quick_send(
            "Osci1T.DataGet",
            &[NanonisValue::U16(data_to_get)],
            &["H"],
            &["d", "d", "i", "*d"],
        )?;

        if result.len() >= 4 {
            let t0 = result[0].as_f64()?;
            let dt = result[1].as_f64()?;
            let size = result[2].as_i32()?;
            let data = result[3].as_f64_array()?.to_vec();
            Ok((t0, dt, size, data))
        } else {
            Err(NanonisError::Protocol(
                "Invalid oscilloscope 1T data response".to_string(),
            ))
        }
    }

    // ==================== Osci2T (Oscilloscope 2-Channels) Functions ====================

    /// Set the channels to display in the Oscilloscope 2-Channels
    /// channel_a_index: 0-23, channel A signal index  
    /// channel_b_index: 0-23, channel B signal index
    pub fn osci2t_ch_set(
        &mut self,
        channel_a_index: i32,
        channel_b_index: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci2T.ChSet",
            &[
                NanonisValue::I32(channel_a_index),
                NanonisValue::I32(channel_b_index),
            ],
            &["i", "i"],
            &[],
        )?;
        Ok(())
    }

    /// Get the channels displayed in the Oscilloscope 2-Channels
    /// Returns: (channel_a_index, channel_b_index)
    pub fn osci2t_ch_get(&mut self) -> Result<(i32, i32), NanonisError> {
        let result = self.quick_send("Osci2T.ChGet", &[], &[], &["i", "i"])?;
        if result.len() >= 2 {
            let channel_a = result[0].as_i32()?;
            let channel_b = result[1].as_i32()?;
            Ok((channel_a, channel_b))
        } else {
            Err(NanonisError::Protocol(
                "Invalid channel response".to_string(),
            ))
        }
    }

    /// Set the timebase in the Oscilloscope 2-Channels
    /// Use osci2t_timebase_get() first to obtain available timebases, then use the index
    pub fn osci2t_timebase_set(&mut self, timebase_index: u16) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci2T.TimebaseSet",
            &[NanonisValue::U16(timebase_index)],
            &["H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the timebase in the Oscilloscope 2-Channels
    /// Returns: (timebase_index, timebases_array)
    pub fn osci2t_timebase_get(&mut self) -> Result<(u16, Vec<f32>), NanonisError> {
        let result = self.quick_send("Osci2T.TimebaseGet", &[], &[], &["H", "i", "*f"])?;
        if result.len() >= 3 {
            let timebase_index = result[0].as_u16()?;
            let timebases = result[2].as_f32_array()?.to_vec();
            Ok((timebase_index, timebases))
        } else {
            Err(NanonisError::Protocol(
                "Invalid timebase response".to_string(),
            ))
        }
    }

    /// Set the oversampling in the Oscilloscope 2-Channels
    /// oversampling_index: 0=50 samples, 1=20, 2=10, 3=5, 4=2, 5=1 sample (no averaging)
    pub fn osci2t_oversampl_set(&mut self, oversampling_index: u16) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci2T.OversamplSet",
            &[NanonisValue::U16(oversampling_index)],
            &["H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the oversampling in the Oscilloscope 2-Channels
    /// Returns: oversampling index (0=50 samples, 1=20, 2=10, 3=5, 4=2, 5=1 sample)
    pub fn osci2t_oversampl_get(&mut self) -> Result<u16, NanonisError> {
        let result = self.quick_send("Osci2T.OversamplGet", &[], &[], &["H"])?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No oversampling index returned".to_string(),
            )),
        }
    }

    /// Set the trigger configuration in the Oscilloscope 2-Channels
    /// trigger_mode: 0 = Immediate, 1 = Level, 2 = Auto
    /// trig_channel: trigger channel
    /// trigger_slope: 0 = Falling, 1 = Rising
    pub fn osci2t_trig_set(
        &mut self,
        trigger_mode: u16,
        trig_channel: u16,
        trigger_slope: u16,
        trigger_level: f64,
        trigger_hysteresis: f64,
        trig_position: f64,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci2T.TrigSet",
            &[
                NanonisValue::U16(trigger_mode),
                NanonisValue::U16(trig_channel),
                NanonisValue::U16(trigger_slope),
                NanonisValue::F64(trigger_level),
                NanonisValue::F64(trigger_hysteresis),
                NanonisValue::F64(trig_position),
            ],
            &["H", "H", "H", "d", "d", "d"],
            &[],
        )?;
        Ok(())
    }

    /// Get the trigger configuration in the Oscilloscope 2-Channels
    /// Returns: (trigger_mode, trig_channel, trigger_slope, trigger_level, trigger_hysteresis, trig_position)
    pub fn osci2t_trig_get(&mut self) -> Result<(u16, u16, u16, f64, f64, f64), NanonisError> {
        let result =
            self.quick_send("Osci2T.TrigGet", &[], &[], &["H", "H", "H", "d", "d", "d"])?;
        if result.len() >= 6 {
            let trigger_mode = result[0].as_u16()?;
            let trig_channel = result[1].as_u16()?;
            let trigger_slope = result[2].as_u16()?;
            let trigger_level = result[3].as_f64()?;
            let trigger_hysteresis = result[4].as_f64()?;
            let trig_position = result[5].as_f64()?;
            Ok((
                trigger_mode,
                trig_channel,
                trigger_slope,
                trigger_level,
                trigger_hysteresis,
                trig_position,
            ))
        } else {
            Err(NanonisError::Protocol(
                "Invalid trigger configuration response".to_string(),
            ))
        }
    }

    /// Start the Oscilloscope 2-Channels
    pub fn osci2t_run(&mut self) -> Result<(), NanonisError> {
        self.quick_send("Osci2T.Run", &[], &[], &[])?;
        Ok(())
    }

    /// Get the graph data from the Oscilloscope 2-Channels
    /// data_to_get: 0 = Current, 1 = Next trigger, 2 = Wait 2 triggers
    /// Returns: (t0, dt, channel_a_data, channel_b_data)
    pub fn osci2t_data_get(
        &mut self,
        data_to_get: u16,
    ) -> Result<(f64, f64, Vec<f64>, Vec<f64>), NanonisError> {
        let result = self.quick_send(
            "Osci2T.DataGet",
            &[NanonisValue::U16(data_to_get)],
            &["H"],
            &["d", "d", "i", "*d", "i", "*d"],
        )?;

        if result.len() >= 6 {
            let t0 = result[0].as_f64()?;
            let dt = result[1].as_f64()?;
            let channel_a_data = result[3].as_f64_array()?.to_vec();
            let channel_b_data = result[5].as_f64_array()?.to_vec();
            Ok((t0, dt, channel_a_data, channel_b_data))
        } else {
            Err(NanonisError::Protocol(
                "Invalid oscilloscope 2T data response".to_string(),
            ))
        }
    }
}
