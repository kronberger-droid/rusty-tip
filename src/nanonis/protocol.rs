use crate::error::NanonisError;
use crate::types::NanonisValue;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::Read;

// Protocol constants
pub const COMMAND_SIZE: usize = 32;
pub const HEADER_SIZE: usize = 40;
pub const MAX_RETRY_COUNT: usize = 1000;
pub const RESPONSE_FLAG: u16 = 1;
pub const ZERO_BUFFER: u16 = 0;

/// Low-level protocol handling
pub struct Protocol;

impl Protocol {
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
                        _ => return Err(NanonisError::Protocol(
                            "Invalid row count for 2D array".to_string(),
                        )),
                    };
                    
                    let cols = match result[result.len() - 1] {
                        NanonisValue::I32(c) => c as usize,
                        _ => return Err(NanonisError::Protocol(
                            "Invalid column count for 2D array".to_string(),
                        )),
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

    /// Create command header with proper padding
    pub fn create_command_header(command: &str, body_size: u32) -> Vec<u8> {
        let mut message = Vec::with_capacity(COMMAND_SIZE + 8);

        // Command: 32 bytes, padded with null bytes
        let mut command_bytes = [0u8; COMMAND_SIZE];
        let cmd_bytes = command.as_bytes();
        let len = cmd_bytes.len().min(COMMAND_SIZE);
        command_bytes[..len].copy_from_slice(&cmd_bytes[..len]);
        message.extend_from_slice(&command_bytes);

        // Body size: 4 bytes big-endian
        message.extend_from_slice(&body_size.to_be_bytes());

        // Send response back flag: 2 bytes big-endian
        message.extend_from_slice(&RESPONSE_FLAG.to_be_bytes());

        // Zero buffer: 2 bytes
        message.extend_from_slice(&ZERO_BUFFER.to_be_bytes());

        message
    }

    /// Validate command response header
    pub fn validate_response_header(header: &[u8; HEADER_SIZE], expected_command: &str) -> Result<u32, NanonisError> {
        // Extract body size from header (bytes 32-36)
        let response_body_size = u32::from_be_bytes([header[32], header[33], header[34], header[35]]);

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
