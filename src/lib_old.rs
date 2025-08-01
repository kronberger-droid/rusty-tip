use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use log::{debug, trace, warn};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use thiserror::Error;

pub mod policy;
pub mod afm_controller;

// Protocol constants
const COMMAND_SIZE: usize = 32;
const HEADER_SIZE: usize = 40;
const MAX_RETRY_COUNT: usize = 1000;
const RESPONSE_FLAG: u16 = 1;
const ZERO_BUFFER: u16 = 0;

// Default timeouts
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Error, Debug)]
pub enum NanonisError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Connection timeout")]
    Timeout,
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("Type error: {0}")]
    Type(String),
    #[error("Command mismatch: expected {expected}, got {actual}")]
    CommandMismatch { expected: String, actual: String },
    #[error("Invalid command: {0}")]
    InvalidCommand(String),
    #[error("Server error: {0}")]
    ServerError(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
}

#[derive(Debug, Clone)]
pub enum NanonisValue {
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    F32(f32),
    F64(f64),
    String(String),
    ArrayU16(Vec<u16>),
    ArrayI16(Vec<i16>),
    ArrayU32(Vec<u32>),
    ArrayI32(Vec<i32>),
    ArrayF32(Vec<f32>),
    ArrayF64(Vec<f64>),
    ArrayString(Vec<String>),
    Array2DF32(Vec<Vec<f32>>),
}

impl NanonisValue {
    /// Extract f32 value with type checking
    pub fn as_f32(&self) -> Result<f32, NanonisError> {
        match self {
            NanonisValue::F32(v) => Ok(*v),
            _ => Err(NanonisError::Type(format!("Expected f32, got {self:?}"))),
        }
    }

    /// Extract f64 value with type checking
    pub fn as_f64(&self) -> Result<f64, NanonisError> {
        match self {
            NanonisValue::F64(v) => Ok(*v),
            _ => Err(NanonisError::Type(format!("Expected f64, got {self:?}"))),
        }
    }

    /// Extract u32 value with type checking
    pub fn as_u32(&self) -> Result<u32, NanonisError> {
        match self {
            NanonisValue::U32(v) => Ok(*v),
            _ => Err(NanonisError::Type(format!("Expected u32, got {self:?}"))),
        }
    }

    /// Extract string array with type checking
    pub fn as_string_array(&self) -> Result<&[String], NanonisError> {
        match self {
            NanonisValue::ArrayString(arr) => Ok(arr),
            _ => Err(NanonisError::Type(format!(
                "Expected string array, got {self:?}"
            ))),
        }
    }

    /// Extract f32 array with type checking
    pub fn as_f32_array(&self) -> Result<&[f32], NanonisError> {
        match self {
            NanonisValue::ArrayF32(arr) => Ok(arr),
            _ => Err(NanonisError::Type(format!(
                "Expected f32 array, got {self:?}"
            ))),
        }
    }
}

/// Type-safe wrappers for common Nanonis values
#[derive(Debug, Clone, Copy)]
pub struct BiasVoltage(pub f32);

#[derive(Debug, Clone, Copy)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

impl Position {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Connection configuration
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            read_timeout: DEFAULT_READ_TIMEOUT,
            write_timeout: DEFAULT_WRITE_TIMEOUT,
        }
    }
}

pub struct NanonisClient {
    stream: TcpStream,
    debug: bool,
    config: ConnectionConfig,
}

impl NanonisClient {
    /// Create a new client with default configuration
    pub fn new(addr: &str) -> Result<Self, NanonisError> {
        Self::with_config(addr, ConnectionConfig::default())
    }

    /// Create a new client with custom configuration
    pub fn with_config(addr: &str, config: ConnectionConfig) -> Result<Self, NanonisError> {
        let socket_addr: SocketAddr = addr
            .parse()
            .map_err(|_| NanonisError::InvalidAddress(addr.to_string()))?;

        debug!("Connecting to Nanonis at {addr}");

        let stream =
            TcpStream::connect_timeout(&socket_addr, config.connect_timeout).map_err(|e| {
                warn!("Failed to connect to {addr}: {e}");
                if e.kind() == std::io::ErrorKind::TimedOut {
                    NanonisError::Timeout
                } else {
                    NanonisError::Io(e)
                }
            })?;

        // Set socket timeouts
        stream.set_read_timeout(Some(config.read_timeout))?;
        stream.set_write_timeout(Some(config.write_timeout))?;

        debug!("Successfully connected to Nanonis");

        Ok(Self {
            stream,
            debug: false,
            config,
        })
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
            let response_data = self.parse_response(&response, response_types)?;
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
            self.serialize_value(value, body_type, &mut body_part)?;
        }

        let body_size = body_part.len() as u32;

        // Create message following Python format with optimized allocation
        let mut message = Vec::with_capacity(COMMAND_SIZE + 8 + body_part.len());

        // Command: 32 bytes, padded with null bytes (optimized)
        let mut command_bytes = [0u8; COMMAND_SIZE];
        let cmd_bytes = command.as_bytes();
        let len = cmd_bytes.len().min(COMMAND_SIZE);
        command_bytes[..len].copy_from_slice(&cmd_bytes[..len]);
        message.extend_from_slice(&command_bytes);

        // Body size: 4 bytes big-endian
        message.write_u32::<BigEndian>(body_size)?;

        // Send response back flag: 2 bytes big-endian
        message.write_u16::<BigEndian>(RESPONSE_FLAG)?;

        // Zero buffer: 2 bytes
        message.write_u16::<BigEndian>(ZERO_BUFFER)?;

        // Body
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

        // Extract body size from header (bytes 32-36)
        let response_body_size =
            u32::from_be_bytes([header[32], header[33], header[34], header[35]]) as usize;

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

        // Verify command matches (bytes 0-32 of header)
        let received_command = String::from_utf8_lossy(&header[0..COMMAND_SIZE])
            .trim_end_matches('\0')
            .trim_end_matches('0')
            .to_string();

        if received_command == command {
            trace!("Command verification successful");
            Ok(response_body)
        } else {
            Err(NanonisError::CommandMismatch {
                expected: command.to_string(),
                actual: received_command,
            })
        }
    }

    fn serialize_value(
        &self,
        value: &NanonisValue,
        body_type: &str,
        buffer: &mut Vec<u8>,
    ) -> Result<(), NanonisError> {
        match (value, body_type) {
            (NanonisValue::U16(v), "H") => buffer.write_u16::<BigEndian>(*v)?,
            (NanonisValue::I16(v), "h") => buffer.write_i16::<BigEndian>(*v)?,
            (NanonisValue::U32(v), "I") => buffer.write_u32::<BigEndian>(*v)?,
            (NanonisValue::I32(v), "i") => buffer.write_i32::<BigEndian>(*v)?,
            (NanonisValue::F32(v), "f") => buffer.write_f32::<BigEndian>(*v)?,
            (NanonisValue::F64(v), "d") => buffer.write_f64::<BigEndian>(*v)?,

            (NanonisValue::String(s), t) if t.contains("*c") => {
                let bytes = s.as_bytes();
                if t.starts_with("+") {
                    buffer.write_u32::<BigEndian>(bytes.len() as u32)?;
                }
                buffer.extend_from_slice(bytes);
            }

            (NanonisValue::ArrayU32(arr), t) if t.contains("*I") => {
                if t.starts_with("+") {
                    buffer.write_u32::<BigEndian>(arr.len() as u32)?;
                }
                for &val in arr {
                    buffer.write_u32::<BigEndian>(val)?;
                }
            }

            (NanonisValue::ArrayF32(arr), t) if t.contains("*f") => {
                if t.starts_with("+") {
                    buffer.write_u32::<BigEndian>(arr.len() as u32)?;
                }
                for &val in arr {
                    buffer.write_f32::<BigEndian>(val)?;
                }
            }

            (NanonisValue::ArrayF64(arr), t) if t.contains("*d") => {
                if t.starts_with("+") {
                    buffer.write_u32::<BigEndian>(arr.len() as u32)?;
                }
                for &val in arr {
                    buffer.write_f64::<BigEndian>(val)?;
                }
            }

            _ => {
                return Err(NanonisError::Type(format!(
                    "Unsupported type combination: {value:?} with {body_type}"
                )))
            }
        }
        Ok(())
    }

    fn parse_response(
        &self,
        response: &[u8],
        response_types: &[&str],
    ) -> Result<Vec<NanonisValue>, NanonisError> {
        let mut cursor = std::io::Cursor::new(response);
        let mut result = Vec::with_capacity(response_types.len());

        for &response_type in response_types {
            let value = match response_type {
                "H" => NanonisValue::U16(cursor.read_u16::<BigEndian>()?),
                "h" => NanonisValue::I16(cursor.read_i16::<BigEndian>()?),
                "I" => NanonisValue::U32(cursor.read_u32::<BigEndian>()?),
                "i" => NanonisValue::I32(cursor.read_i32::<BigEndian>()?),
                "f" => NanonisValue::F32(cursor.read_f32::<BigEndian>()?),
                "d" => NanonisValue::F64(cursor.read_f64::<BigEndian>()?),

                t if t.contains("*f") => {
                    let len = if t.starts_with("+") {
                        cursor.read_u32::<BigEndian>()? as usize
                    } else if let Some(prev_val) = result.last() {
                        match prev_val {
                            NanonisValue::U32(len) => *len as usize,
                            _ => {
                                return Err(NanonisError::Protocol(
                                    "Array length not found".to_string(),
                                ))
                            }
                        }
                    } else {
                        return Err(NanonisError::Protocol(
                            "Array length not specified".to_string(),
                        ));
                    };

                    let mut arr = Vec::with_capacity(len);
                    for _ in 0..len {
                        arr.push(cursor.read_f32::<BigEndian>()?);
                    }
                    NanonisValue::ArrayF32(arr)
                }

                t if t.contains("*d") => {
                    let len = if t.starts_with("+") {
                        cursor.read_u32::<BigEndian>()? as usize
                    } else if let Some(prev_val) = result.last() {
                        match prev_val {
                            NanonisValue::U32(len) => *len as usize,
                            _ => {
                                return Err(NanonisError::Protocol(
                                    "Array length not found".to_string(),
                                ))
                            }
                        }
                    } else {
                        return Err(NanonisError::Protocol(
                            "Array length not specified".to_string(),
                        ));
                    };

                    let mut arr = Vec::with_capacity(len);
                    for _ in 0..len {
                        arr.push(cursor.read_f64::<BigEndian>()?);
                    }
                    NanonisValue::ArrayF64(arr)
                }

                // Handle string arrays with prepended length
                "+*c" => {
                    // First read total byte size (we don't use this, but it's in the protocol)
                    let _total_size = cursor.read_u32::<BigEndian>()?;
                    // Then read number of strings
                    let num_strings = cursor.read_u32::<BigEndian>()? as usize;
                    let mut strings = Vec::with_capacity(num_strings);

                    for _ in 0..num_strings {
                        let string_len = cursor.read_u32::<BigEndian>()? as usize;
                        let mut string_bytes = vec![0u8; string_len];
                        cursor.read_exact(&mut string_bytes)?;
                        let string = String::from_utf8_lossy(&string_bytes).to_string();
                        strings.push(string);
                    }

                    NanonisValue::ArrayString(strings)
                }

                _ => {
                    return Err(NanonisError::Type(format!(
                        "Unsupported response type: {response_type}"
                    )))
                }
            };

            result.push(value);
        }

        Ok(result)
    }

    // Type-safe method implementations

    /// Set bias voltage with type safety
    pub fn set_bias(&mut self, voltage: BiasVoltage) -> Result<(), NanonisError> {
        self.quick_send("Bias.Set", &[NanonisValue::F32(voltage.0)], &["f"], &[])?;
        Ok(())
    }

    /// Get bias voltage with type safety
    pub fn get_bias(&mut self) -> Result<BiasVoltage, NanonisError> {
        let result = self.quick_send("Bias.Get", &[], &[], &["f"])?;
        match result.first() {
            Some(value) => Ok(BiasVoltage(value.as_f32()?)),
            None => Err(NanonisError::Protocol("No bias value returned".to_string())),
        }
    }

    /// Set XY position with type safety
    pub fn set_xy_position(
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

    /// Get available signal names
    pub fn get_signal_names(&mut self) -> Result<Vec<String>, NanonisError> {
        let result = self.quick_send("Signals.NamesGet", &[], &[], &["+*c"])?;
        match result.first() {
            Some(value) => Ok(value.as_string_array()?.to_vec()),
            None => Err(NanonisError::Protocol(
                "No signal names returned".to_string(),
            )),
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

    /// Get current value of a signal by index
    pub fn signals_val_get(
        &mut self,
        signal_index: i32,
        wait_for_newest_data: bool,
    ) -> Result<f32, NanonisError> {
        let wait_flag = if wait_for_newest_data { 1u32 } else { 0u32 };
        let result = self.quick_send(
            "Signals.ValGet",
            &[
                NanonisValue::I32(signal_index),
                NanonisValue::U32(wait_flag),
            ],
            &["i", "I"],
            &["f"],
        )?;
        match result.first() {
            Some(value) => Ok(value.as_f32()?),
            None => Err(NanonisError::Protocol(
                "No signal value returned".to_string(),
            )),
        }
    }

    /// Get current values of multiple signals by indexes
    pub fn signals_vals_get(
        &mut self,
        signal_indexes: &[i32],
        wait_for_newest_data: bool,
    ) -> Result<Vec<f32>, NanonisError> {
        let wait_flag = if wait_for_newest_data { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "Signals.ValsGet",
            &[
                NanonisValue::ArrayI32(signal_indexes.to_vec()),
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
        let signals = self.get_signal_names()?;
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
            Some(index) => self.signals_val_get(index as i32, wait_for_newest),
            None => Err(NanonisError::InvalidCommand(format!(
                "Signal '{signal_name}' not found"
            ))),
        }
    }

    // Legacy methods for backward compatibility
    pub fn bias_set(&mut self, bias_value: f32) -> Result<Vec<NanonisValue>, NanonisError> {
        self.quick_send("Bias.Set", &[NanonisValue::F32(bias_value)], &["f"], &[])
    }

    pub fn bias_get(&mut self) -> Result<Vec<NanonisValue>, NanonisError> {
        self.quick_send("Bias.Get", &[], &[], &["f"])
    }

    pub fn folme_xy_pos_set(
        &mut self,
        x_m: f64,
        y_m: f64,
        wait_end: u32,
    ) -> Result<Vec<NanonisValue>, NanonisError> {
        self.quick_send(
            "FolMe.XYPosSet",
            &[
                NanonisValue::F64(x_m),
                NanonisValue::F64(y_m),
                NanonisValue::U32(wait_end),
            ],
            &["d", "d", "I"],
            &[],
        )
    }

    pub fn signals_names_get(&mut self) -> Result<Vec<NanonisValue>, NanonisError> {
        self.quick_send("Signals.NamesGet", &[], &[], &["+*c"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_serialization() {
        let client = NanonisClient::new("127.0.0.1:6501").unwrap();
        let mut buffer = Vec::new();

        client
            .serialize_value(&NanonisValue::F32(std::f32::consts::PI), "f", &mut buffer)
            .unwrap();
        assert_eq!(buffer.len(), 4);
    }

    #[test]
    fn test_bias_voltage() {
        let voltage = BiasVoltage(1.5);
        assert_eq!(voltage.0, 1.5);
    }

    #[test]
    fn test_position() {
        let pos = Position::new(1e-9, 2e-9);
        assert_eq!(pos.x, 1e-9);
        assert_eq!(pos.y, 2e-9);
    }

    #[test]
    fn test_value_type_safety() {
        let value = NanonisValue::F32(std::f32::consts::PI);
        assert!(value.as_f32().is_ok());
        assert!(value.as_f64().is_err());
        assert!(value.as_u32().is_err());
    }

    #[test]
    fn test_connection_config() {
        let config = ConnectionConfig::default();
        assert_eq!(config.connect_timeout, DEFAULT_CONNECT_TIMEOUT);
        assert_eq!(config.read_timeout, DEFAULT_READ_TIMEOUT);
        assert_eq!(config.write_timeout, DEFAULT_WRITE_TIMEOUT);
    }
}
