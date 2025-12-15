use crate::error::NanonisError;
use crate::types::NanonisValue;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use log::debug;
use std::io::Read;

// Protocol constants
pub const COMMAND_SIZE: usize = 32;
pub const HEADER_SIZE: usize = 40;
pub const ERROR_INFO_SIZE: usize = 8;
pub const MAX_RETRY_COUNT: usize = 1000;
pub const MAX_RESPONSE_SIZE: usize = 100 * 1024 * 1024; // 100MB
pub const RESPONSE_FLAG: u16 = 1;
pub const ZERO_BUFFER: u16 = 0;

#[derive(Debug, Clone)]
struct MessageHeader {
    command: [u8; COMMAND_SIZE],
    body_size: u32,
    send_response: u16,
    _padding: u16,
}

impl MessageHeader {
    fn new(command: &str, body_size: u32) -> Self {
        let mut cmd_bytes = [0u8; COMMAND_SIZE];
        let cmd_str = command.as_bytes();
        let len = cmd_str.len().min(COMMAND_SIZE);
        cmd_bytes[..len].copy_from_slice(&cmd_str[..len]);

        Self {
            command: cmd_bytes,
            body_size,
            send_response: RESPONSE_FLAG, // Always request response for error info
            _padding: ZERO_BUFFER,
        }
    }

    // Safe serialization without unsafe code
    fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..32].copy_from_slice(&self.command);
        buf[32..36].copy_from_slice(&self.body_size.to_be_bytes());
        buf[36..38].copy_from_slice(&self.send_response.to_be_bytes());
        buf[38..40].copy_from_slice(&self._padding.to_be_bytes());
        buf
    }
}

/// Low-level protocol handling
pub struct Protocol;

impl Protocol {
    /// Parse error information from the end of a response body using safe slice operations
    pub fn parse_error_info(
        body: &[u8],
        data_end_cursor: usize,
    ) -> Result<(), NanonisError> {
        // Get error section safely
        let error_section = match body.get(data_end_cursor..) {
            Some(section) if section.len() >= ERROR_INFO_SIZE => section,
            _ => return Ok(()), // No error info available
        };

        // Use safe slice splitting instead of manual indexing
        let (status_bytes, rest) = error_section.split_at(4);
        let (size_bytes, message_bytes) = rest.split_at(4);

        let error_status =
            i32::from_be_bytes(status_bytes.try_into().map_err(|_| {
                NanonisError::Protocol("Invalid error status format".into())
            })?);

        let error_desc_size =
            i32::from_be_bytes(size_bytes.try_into().map_err(|_| {
                NanonisError::Protocol("Invalid error size format".into())
            })?) as usize;

        if error_desc_size > 0 {
            // Safe message extraction with bounds checking
            let message_slice =
                message_bytes.get(..error_desc_size).ok_or_else(|| {
                    NanonisError::Protocol("Error message truncated".into())
                })?;

            // Use from_utf8 for better error handling
            let error_msg = std::str::from_utf8(message_slice).map_err(|_| {
                NanonisError::Protocol("Invalid UTF-8 in error message".into())
            })?;

            let trimmed_msg = error_msg.trim();
            if !trimmed_msg.is_empty() {
                return Err(NanonisError::ServerError {
                    code: error_status,
                    message: trimmed_msg.to_string(),
                });
            }
        }

        Ok(())
    }

    /// Helper for reading exact byte counts with better error messages
    pub fn read_exact_bytes<const N: usize>(
        reader: &mut dyn Read,
    ) -> Result<[u8; N], NanonisError> {
        debug!("Attempting to read exactly {} bytes", N);
        let mut buf = [0u8; N];

        match reader.read_exact(&mut buf) {
            Ok(()) => {
                debug!(
                    "Successfully read {} bytes: {:02x?}",
                    N,
                    if N <= 20 { &buf[..] } else { &buf[..20] }
                );
                Ok(buf)
            }
            Err(e) => {
                debug!("Failed to read {} bytes: {} (kind: {:?})", N, e, e.kind());
                Err(NanonisError::Io {
                    source: e,
                    context: format!("Failed to read {} bytes from Nanonis", N),
                })
            }
        }
    }

    /// Helper for reading variable-length data with size validation
    pub fn read_variable_bytes(
        reader: &mut dyn Read,
        size: usize,
    ) -> Result<Vec<u8>, NanonisError> {
        debug!("Attempting to read {} variable bytes", size);

        // Reasonable size limit to prevent memory attacks
        if size > MAX_RESPONSE_SIZE {
            debug!("Size {} exceeds maximum {}", size, MAX_RESPONSE_SIZE);
            return Err(NanonisError::Protocol(format!(
                "Response size {} exceeds maximum {}",
                size, MAX_RESPONSE_SIZE
            )));
        }

        let mut body = vec![0u8; size];
        match reader.read_exact(&mut body) {
            Ok(()) => {
                debug!(
                    "Successfully read {} variable bytes: {:02x?}",
                    size,
                    if size <= 50 { &body[..] } else { &body[..50] }
                );
                Ok(body)
            }
            Err(e) => {
                debug!(
                    "Failed to read {} variable bytes: {} (kind: {:?})",
                    size,
                    e,
                    e.kind()
                );
                // Try to read whatever we can to diagnose the issue
                let mut partial_buf = Vec::new();
                if let Ok(bytes_read) = reader.read_to_end(&mut partial_buf) {
                    debug!(
                        "Partial read got {} bytes: {:02x?}",
                        bytes_read,
                        if bytes_read <= 50 {
                            &partial_buf[..]
                        } else {
                            &partial_buf[..50]
                        }
                    );
                }
                Err(NanonisError::Io {
                    source: e,
                    context: format!("Failed to read {} byte response body", size),
                })
            }
        }
    }

    /// Parse response with error checking - returns (values, cursor_position)
    pub fn parse_response_with_error_check(
        response: &[u8],
        response_types: &[&str],
    ) -> Result<Vec<NanonisValue>, NanonisError> {
        // Parse normal response data first
        let values = Self::parse_response(response, response_types)?;

        // Calculate cursor position after parsing all response data
        let cursor = Self::calculate_cursor_position(response, response_types)?;

        // Check for errors at the end
        Self::parse_error_info(response, cursor)?;

        Ok(values)
    }

    /// Calculate cursor position after parsing response data
    fn calculate_cursor_position(
        response: &[u8],
        response_types: &[&str],
    ) -> Result<usize, NanonisError> {
        let mut cursor = std::io::Cursor::new(response);
        let mut result = Vec::with_capacity(response_types.len());

        // This is essentially the same parsing logic as parse_response,
        // but we only track the cursor position without storing values
        for &response_type in response_types {
            match response_type {
                "H" => {
                    cursor.read_u16::<BigEndian>()?;
                }
                "h" => {
                    cursor.read_i16::<BigEndian>()?;
                }
                "I" => {
                    let val = cursor.read_u32::<BigEndian>()?;
                    result.push(val);
                }
                "i" => {
                    let val = cursor.read_i32::<BigEndian>()? as u32;
                    result.push(val);
                }
                "f" => {
                    cursor.read_f32::<BigEndian>()?;
                }
                "d" => {
                    cursor.read_f64::<BigEndian>()?;
                }

                t if t.contains("*f") => {
                    let len = if t.starts_with("+") {
                        cursor.read_u32::<BigEndian>()? as usize
                    } else if let Some(&prev_val) = result.last() {
                        prev_val as usize
                    } else {
                        return Err(NanonisError::Protocol(
                            "Array length not specified".to_string(),
                        ));
                    };

                    for _ in 0..len {
                        cursor.read_f32::<BigEndian>()?;
                    }
                }

                t if t.contains("*d") => {
                    let len = if t.starts_with("+") {
                        cursor.read_u32::<BigEndian>()? as usize
                    } else if let Some(&prev_val) = result.last() {
                        prev_val as usize
                    } else {
                        return Err(NanonisError::Protocol(
                            "Array length not specified".to_string(),
                        ));
                    };

                    for _ in 0..len {
                        cursor.read_f64::<BigEndian>()?;
                    }
                }

                t if t.contains("*i") => {
                    let len = if t.starts_with("+") {
                        cursor.read_u32::<BigEndian>()? as usize
                    } else if let Some(&prev_val) = result.last() {
                        prev_val as usize
                    } else {
                        return Err(NanonisError::Protocol(
                            "Array length not specified".to_string(),
                        ));
                    };

                    for _ in 0..len {
                        cursor.read_i32::<BigEndian>()?;
                    }
                }

                "+*c" => {
                    let _total_size = cursor.read_u32::<BigEndian>()?;
                    let num_strings = cursor.read_u32::<BigEndian>()? as usize;

                    for _ in 0..num_strings {
                        let string_len = cursor.read_u32::<BigEndian>()? as usize;
                        let mut string_bytes = vec![0u8; string_len];
                        cursor.read_exact(&mut string_bytes)?;
                    }
                }

                "*+c" => {
                    let num_strings = if let Some(&prev_val) = result.last() {
                        prev_val as usize
                    } else {
                        return Err(NanonisError::Protocol(
                            "String count not found for *+c type".to_string(),
                        ));
                    };

                    for _ in 0..num_strings {
                        let string_len = cursor.read_u32::<BigEndian>()? as usize;
                        let mut string_bytes = vec![0u8; string_len];
                        cursor.read_exact(&mut string_bytes)?;
                    }
                }

                "*-c" => {
                    let string_length = result.last().ok_or_else(|| {
                        NanonisError::Protocol(
                            "String length not found for *-c type".to_string(),
                        )
                    })?;

                    let mut string_bytes = vec![0u8; *string_length as usize];
                    cursor.read_exact(&mut string_bytes)?;
                }

                "2f" => {
                    if result.len() < 2 {
                        return Err(NanonisError::Protocol(
                            "2D array dimensions not found".to_string(),
                        ));
                    }

                    let rows = result[result.len() - 2] as usize;
                    let cols = result[result.len() - 1] as usize;

                    for _ in 0..(rows * cols) {
                        cursor.read_f32::<BigEndian>()?;
                    }
                }

                _ => {
                    return Err(NanonisError::Type(format!(
                        "Unsupported response type: {response_type}"
                    )));
                }
            };
        }

        Ok(cursor.position() as usize)
    }

    /// Serialize a value according to its type specification
    pub fn serialize_value(
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

            (NanonisValue::ArrayString(arr), "*+c") => {
                // Don't write count - it comes from previous variable
                for s in arr {
                    let bytes = s.as_bytes();
                    buffer.write_u32::<BigEndian>(bytes.len() as u32)?;
                    buffer.extend_from_slice(bytes);
                }
            }

            (NanonisValue::ArrayI32(arr), t) if t.contains("*i") => {
                if t.starts_with("+") {
                    buffer.write_u32::<BigEndian>(arr.len() as u32)?;
                }
                for &val in arr {
                    buffer.write_i32::<BigEndian>(val)?;
                }
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

    /// Parse response data according to type specifications
    pub fn parse_response(
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
                            NanonisValue::I32(len) => *len as usize,
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
                            NanonisValue::I32(len) => *len as usize,
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

                t if t.contains("*i") => {
                    let len = if t.starts_with("+") {
                        cursor.read_u32::<BigEndian>()? as usize
                    } else if let Some(prev_val) = result.last() {
                        match prev_val {
                            NanonisValue::U32(len) => *len as usize,
                            NanonisValue::I32(len) => *len as usize,
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
                        arr.push(cursor.read_i32::<BigEndian>()?);
                    }
                    NanonisValue::ArrayI32(arr)
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
                        let string =
                            String::from_utf8_lossy(&string_bytes).to_string();
                        strings.push(string);
                    }

                    NanonisValue::ArrayString(strings)
                }

                // Handle string arrays with count from previous variable
                "*+c" => {
                    // Get string count from previous variable (should be an integer)
                    let num_strings = match result.last() {
                        Some(NanonisValue::I32(count)) => *count as usize,
                        Some(NanonisValue::U32(count)) => *count as usize,
                        _ => {
                            return Err(NanonisError::Protocol(
                                "String count not found for *+c type".to_string(),
                            ))
                        }
                    };

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

                // Handle dynamic strings (*-c) where length comes from previous variable
                "*-c" => {
                    // Get string length from previous variable (should be an integer)
                    let string_length = match result.last() {
                        Some(NanonisValue::I32(len)) => *len as usize,
                        Some(NanonisValue::U32(len)) => *len as usize,
                        _ => {
                            return Err(NanonisError::Protocol(
                                "String length not found for *-c type".to_string(),
                            ))
                        }
                    };

                    // Read string bytes
                    let mut string_bytes = vec![0u8; string_length];
                    cursor.read_exact(&mut string_bytes)?;
                    let string = String::from_utf8_lossy(&string_bytes).to_string();

                    NanonisValue::String(string)
                }

                "2f" => {
                    // 2D float array - dimensions should be in the two preceding i32 values
                    if result.len() < 2 {
                        return Err(NanonisError::Protocol(
                            "2D array dimensions not found".to_string(),
                        ));
                    }

                    let rows = match result[result.len() - 2] {
                        NanonisValue::I32(r) => r as usize,
                        _ => {
                            return Err(NanonisError::Protocol(
                                "Invalid row count for 2D array".to_string(),
                            ))
                        }
                    };

                    let cols = match result[result.len() - 1] {
                        NanonisValue::I32(c) => c as usize,
                        _ => {
                            return Err(NanonisError::Protocol(
                                "Invalid column count for 2D array".to_string(),
                            ))
                        }
                    };

                    // Read the flat array data
                    let mut data_2d = Vec::with_capacity(rows);

                    for _ in 0..rows {
                        let mut row_data = Vec::with_capacity(cols);
                        for _ in 0..cols {
                            row_data.push(cursor.read_f32::<BigEndian>()?);
                        }
                        data_2d.push(row_data);
                    }

                    NanonisValue::Array2DF32(data_2d)
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

    /// Create command header with proper padding using safe serialization
    pub fn create_command_header(command: &str, body_size: u32) -> Vec<u8> {
        let header = MessageHeader::new(command, body_size);
        header.to_bytes().to_vec()
    }

    /// Validate command response header
    pub fn validate_response_header(
        header: &[u8; HEADER_SIZE],
        expected_command: &str,
    ) -> Result<u32, NanonisError> {
        // Extract body size from header (bytes 32-36)
        let response_body_size =
            u32::from_be_bytes([header[32], header[33], header[34], header[35]]);

        // Verify command matches (bytes 0-32 of header)
        let received_command = String::from_utf8_lossy(&header[0..COMMAND_SIZE])
            .trim_end_matches('\0')
            .trim_end_matches('0')
            .to_string();

        if received_command == expected_command {
            Ok(response_body_size)
        } else {
            Err(NanonisError::CommandMismatch {
                expected: expected_command.to_string(),
                actual: received_command,
            })
        }
    }
}
