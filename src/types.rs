use std::time::Duration;
use thiserror::Error;

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

    /// Extract u16 value with type checking
    pub fn as_u16(&self) -> Result<u16, NanonisError> {
        match self {
            NanonisValue::U16(v) => Ok(*v),
            _ => Err(NanonisError::Type(format!("Expected u16, got {self:?}"))),
        }
    }

    /// Extract u32 value with type checking
    pub fn as_u32(&self) -> Result<u32, NanonisError> {
        match self {
            NanonisValue::U32(v) => Ok(*v),
            _ => Err(NanonisError::Type(format!("Expected u32, got {self:?}"))),
        }
    }

    /// Extract u16 value with type checking
    pub fn as_i16(&self) -> Result<i16, NanonisError> {
        match self {
            NanonisValue::I16(v) => Ok(*v),
            _ => Err(NanonisError::Type(format!("Expected u16, got {self:?}"))),
        }
    }

    /// Extract u32 value with type checking
    pub fn as_i32(&self) -> Result<i32, NanonisError> {
        match self {
            NanonisValue::I32(v) => Ok(*v),
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
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(10),
            write_timeout: Duration::from_secs(5),
        }
    }
}
