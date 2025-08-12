use serde::{Deserialize, Serialize};

use crate::classifier::TipState;
use crate::error::NanonisError;
use std::collections::VecDeque;

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

// Conversion traits for NanonisValue
impl From<f32> for NanonisValue {
    fn from(value: f32) -> Self {
        NanonisValue::F32(value)
    }
}

impl From<f64> for NanonisValue {
    fn from(value: f64) -> Self {
        NanonisValue::F64(value)
    }
}

impl From<u16> for NanonisValue {
    fn from(value: u16) -> Self {
        NanonisValue::U16(value)
    }
}

impl From<u32> for NanonisValue {
    fn from(value: u32) -> Self {
        NanonisValue::U32(value)
    }
}

impl From<i16> for NanonisValue {
    fn from(value: i16) -> Self {
        NanonisValue::I16(value)
    }
}

impl From<i32> for NanonisValue {
    fn from(value: i32) -> Self {
        NanonisValue::I32(value)
    }
}

impl From<String> for NanonisValue {
    fn from(value: String) -> Self {
        NanonisValue::String(value)
    }
}

impl From<Vec<f32>> for NanonisValue {
    fn from(value: Vec<f32>) -> Self {
        NanonisValue::ArrayF32(value)
    }
}

impl From<Vec<String>> for NanonisValue {
    fn from(value: Vec<String>) -> Self {
        NanonisValue::ArrayString(value)
    }
}

impl From<Vec<i32>> for NanonisValue {
    fn from(value: Vec<i32>) -> Self {
        NanonisValue::ArrayI32(value)
    }
}

impl TryFrom<NanonisValue> for f32 {
    type Error = NanonisError;

    fn try_from(value: NanonisValue) -> Result<Self, Self::Error> {
        match value {
            NanonisValue::F32(v) => Ok(v),
            _ => Err(NanonisError::Type(format!("Expected f32, got {value:?}"))),
        }
    }
}

impl TryFrom<NanonisValue> for f64 {
    type Error = NanonisError;

    fn try_from(value: NanonisValue) -> Result<Self, Self::Error> {
        match value {
            NanonisValue::F64(v) => Ok(v),
            _ => Err(NanonisError::Type(format!("Expected f64, got {value:?}"))),
        }
    }
}

impl TryFrom<NanonisValue> for u16 {
    type Error = NanonisError;

    fn try_from(value: NanonisValue) -> Result<Self, Self::Error> {
        match value {
            NanonisValue::U16(v) => Ok(v),
            _ => Err(NanonisError::Type(format!("Expected u16, got {value:?}"))),
        }
    }
}

impl TryFrom<NanonisValue> for u32 {
    type Error = NanonisError;

    fn try_from(value: NanonisValue) -> Result<Self, Self::Error> {
        match value {
            NanonisValue::U32(v) => Ok(v),
            _ => Err(NanonisError::Type(format!("Expected u32, got {value:?}"))),
        }
    }
}

impl TryFrom<NanonisValue> for i16 {
    type Error = NanonisError;

    fn try_from(value: NanonisValue) -> Result<Self, Self::Error> {
        match value {
            NanonisValue::I16(v) => Ok(v),
            _ => Err(NanonisError::Type(format!("Expected i16, got {value:?}"))),
        }
    }
}

impl TryFrom<NanonisValue> for i32 {
    type Error = NanonisError;

    fn try_from(value: NanonisValue) -> Result<Self, Self::Error> {
        match value {
            NanonisValue::I32(v) => Ok(v),
            _ => Err(NanonisError::Type(format!("Expected i32, got {value:?}"))),
        }
    }
}

impl TryFrom<NanonisValue> for Vec<f32> {
    type Error = NanonisError;

    fn try_from(value: NanonisValue) -> Result<Self, Self::Error> {
        match value {
            NanonisValue::ArrayF32(v) => Ok(v),
            _ => Err(NanonisError::Type(format!(
                "Expected Vec<f32>, got {value:?}"
            ))),
        }
    }
}

impl TryFrom<NanonisValue> for Vec<String> {
    type Error = NanonisError;

    fn try_from(value: NanonisValue) -> Result<Self, Self::Error> {
        match value {
            NanonisValue::ArrayString(v) => Ok(v),
            _ => Err(NanonisError::Type(format!(
                "Expected Vec<String>, got {value:?}"
            ))),
        }
    }
}

impl TryFrom<NanonisValue> for Vec<i32> {
    type Error = NanonisError;

    fn try_from(value: NanonisValue) -> Result<Self, Self::Error> {
        match value {
            NanonisValue::ArrayI32(v) => Ok(v),
            _ => Err(NanonisError::Type(format!(
                "Expected Vec<i32>, got {value:?}"
            ))),
        }
    }
}

// Convenience methods (keeping these for backwards compatibility)
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

    /// Extract i16 value with type checking
    pub fn as_i16(&self) -> Result<i16, NanonisError> {
        match self {
            NanonisValue::I16(v) => Ok(*v),
            _ => Err(NanonisError::Type(format!("Expected i16, got {self:?}"))),
        }
    }

    /// Extract i32 value with type checking
    pub fn as_i32(&self) -> Result<i32, NanonisError> {
        match self {
            NanonisValue::I32(v) => Ok(*v),
            _ => Err(NanonisError::Type(format!("Expected i32, got {self:?}"))),
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

/// Session metadata - static information written once per monitoring session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub session_id: String,
    pub signal_names: Vec<String>,      // All signal names
    pub active_indices: Vec<usize>,     // Which signals are being monitored
    pub primary_signal_index: usize,    // Index of the primary signal
    pub session_start: f64,             // Session start timestamp
}

/// Comprehensive machine state for advanced policy engines
/// Expandable for transformer/ML models that need rich context
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MachineState {
    // Current signal readings
    pub all_signals: Option<Vec<f32>>, // All available signals for context
    
    // Runtime signal coordination (not saved to JSON - info is in SessionMetadata)
    #[serde(skip)]
    pub signal_indices: Option<Vec<i32>>, // Which signal indices all_signals contains [0,1,2,3,24,30,31]

    // Spatial context
    pub position: Option<(f64, f64)>, // Current XY position
    pub z_position: Option<f64>,      // Z height

    // Temporal context
    pub timestamp: f64, // When this state was captured
    #[serde(skip)]
    pub signal_history: VecDeque<f32>, // Historical signal values

    // System state
    pub approach_count: u32,         // Number of approaches performed
    pub last_action: Option<String>, // Last action executed
    pub system_parameters: Vec<f32>, // Configurable system params

    // Classification result
    pub classification: TipState, // How the classifier interpreted this state

                                  // For future ML/transformer expansion:
                                  // pub embedding: Option<Vec<f32>>,         // Learned state representation
                                  // pub attention_weights: Option<Vec<f32>>, // Transformer attention scores
                                  // pub confidence: f32,                     // Model confidence in decision
}
