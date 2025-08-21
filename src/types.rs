use serde::{Deserialize, Serialize};

use crate::classifier::TipState;
use crate::error::NanonisError;
use std::collections::VecDeque;
use std::time::Duration;

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

    /// Extract f64 array with type checking
    pub fn as_f64_array(&self) -> Result<&[f64], NanonisError> {
        match self {
            NanonisValue::ArrayF64(arr) => Ok(arr),
            _ => Err(NanonisError::Type(format!(
                "Expected f64 array, got {self:?}"
            ))),
        }
    }

    /// Extract i32 array with type checking
    pub fn as_i32_array(&self) -> Result<&[i32], NanonisError> {
        match self {
            NanonisValue::ArrayI32(arr) => Ok(arr),
            _ => Err(NanonisError::Type(format!(
                "Expected i32 array, got {self:?}"
            ))),
        }
    }

    /// Extract string value with type checking
    pub fn as_string(&self) -> Result<&str, NanonisError> {
        match self {
            NanonisValue::String(s) => Ok(s),
            _ => Err(NanonisError::Type(format!(
                "Expected string, got {self:?}"
            ))),
        }
    }

    /// Extract 2D f32 array with type checking
    pub fn as_f32_2d_array(&self) -> Result<&Vec<Vec<f32>>, NanonisError> {
        match self {
            NanonisValue::Array2DF32(arr) => Ok(arr),
            _ => Err(NanonisError::Type(format!(
                "Expected 2D f32 array, got {self:?}"
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

/// Signal and Channel Types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalIndex(pub i32);

impl SignalIndex {
    pub fn new(index: i32) -> Result<Self, crate::error::NanonisError> {
        if (0..=127).contains(&index) {
            Ok(SignalIndex(index))
        } else {
            Err(crate::error::NanonisError::InvalidCommand(
                format!("Signal index must be 0-127, got {}", index)
            ))
        }
    }
}

impl From<SignalIndex> for i32 {
    fn from(signal: SignalIndex) -> Self {
        signal.0
    }
}

impl From<i32> for SignalIndex {
    fn from(index: i32) -> Self {
        SignalIndex(index)
    }
}

impl From<usize> for SignalIndex {
    fn from(index: usize) -> Self {
        SignalIndex(index as i32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelIndex(pub i32);

impl ChannelIndex {
    pub fn new(index: i32) -> Result<Self, crate::error::NanonisError> {
        if (0..=23).contains(&index) {
            Ok(ChannelIndex(index))
        } else {
            Err(crate::error::NanonisError::InvalidCommand(
                format!("Channel index must be 0-23, got {}", index)
            ))
        }
    }
}

impl From<ChannelIndex> for i32 {
    fn from(channel: ChannelIndex) -> Self {
        channel.0
    }
}

impl From<i32> for ChannelIndex {
    fn from(index: i32) -> Self {
        ChannelIndex(index)
    }
}

impl From<usize> for ChannelIndex {
    fn from(index: usize) -> Self {
        ChannelIndex(index as i32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OscilloscopeIndex(pub i32);

impl From<OscilloscopeIndex> for i32 {
    fn from(osci: OscilloscopeIndex) -> Self {
        osci.0
    }
}

impl From<i32> for OscilloscopeIndex {
    fn from(index: i32) -> Self {
        OscilloscopeIndex(index)
    }
}

impl From<usize> for OscilloscopeIndex {
    fn from(index: usize) -> Self {
        OscilloscopeIndex(index as i32)
    }
}

/// Motor Control Types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotorDirection {
    XPlus = 0,
    XMinus = 1,
    YPlus = 2,
    YMinus = 3,
    ZPlus = 4,
    ZMinus = 5,
}

impl From<MotorDirection> for u32 {
    fn from(direction: MotorDirection) -> Self {
        direction as u32
    }
}

impl TryFrom<u32> for MotorDirection {
    type Error = crate::error::NanonisError;
    
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(MotorDirection::XPlus),
            1 => Ok(MotorDirection::XMinus),
            2 => Ok(MotorDirection::YPlus),
            3 => Ok(MotorDirection::YMinus),
            4 => Ok(MotorDirection::ZPlus),
            5 => Ok(MotorDirection::ZMinus),
            _ => Err(crate::error::NanonisError::InvalidCommand(
                format!("Invalid motor direction: {}", value)
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotorGroup {
    Group1 = 0,
    Group2 = 1,
    Group3 = 2,
    Group4 = 3,
    Group5 = 4,
    Group6 = 5,
}

impl From<MotorGroup> for u32 {
    fn from(group: MotorGroup) -> Self {
        group as u32
    }
}

impl TryFrom<u32> for MotorGroup {
    type Error = crate::error::NanonisError;
    
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(MotorGroup::Group1),
            1 => Ok(MotorGroup::Group2),
            2 => Ok(MotorGroup::Group3),
            3 => Ok(MotorGroup::Group4),
            4 => Ok(MotorGroup::Group5),
            5 => Ok(MotorGroup::Group6),
            _ => Err(crate::error::NanonisError::InvalidCommand(
                format!("Invalid motor group: {}", value)
            )),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StepCount(pub u16);

impl From<StepCount> for u16 {
    fn from(steps: StepCount) -> Self {
        steps.0
    }
}

impl From<u16> for StepCount {
    fn from(steps: u16) -> Self {
        StepCount(steps)
    }
}

impl From<u32> for StepCount {
    fn from(steps: u32) -> Self {
        StepCount(steps as u16)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Frequency(pub f32);

impl Frequency {
    pub fn hz(value: f32) -> Self {
        Self(value)
    }
}

impl From<Frequency> for f32 {
    fn from(freq: Frequency) -> Self {
        freq.0
    }
}

impl From<f32> for Frequency {
    fn from(freq: f32) -> Self {
        Frequency(freq)
    }
}

impl From<f64> for Frequency {
    fn from(freq: f64) -> Self {
        Frequency(freq as f32)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Amplitude(pub f32);

impl Amplitude {
    pub fn volts(value: f32) -> Self {
        Self(value)
    }
}

impl From<Amplitude> for f32 {
    fn from(amp: Amplitude) -> Self {
        amp.0
    }
}

impl From<f32> for Amplitude {
    fn from(amp: f32) -> Self {
        Amplitude(amp)
    }
}

impl From<f64> for Amplitude {
    fn from(amp: f64) -> Self {
        Amplitude(amp as f32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotorAxis {
    All = 0,
    X = 1,
    Y = 2,
    Z = 3,
}

impl From<MotorAxis> for u16 {
    fn from(axis: MotorAxis) -> Self {
        axis as u16
    }
}

// From implementations for common integer types for convenience
impl From<u16> for MotorAxis {
    fn from(value: u16) -> Self {
        match value {
            0 => MotorAxis::All,
            1 => MotorAxis::X,
            2 => MotorAxis::Y,
            3 => MotorAxis::Z,
            _ => MotorAxis::All, // Default fallback
        }
    }
}

impl From<i32> for MotorAxis {
    fn from(value: i32) -> Self {
        MotorAxis::from(value as u16)
    }
}

/// Position Extensions
#[derive(Debug, Clone, Copy)]
pub struct Position3D {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Position3D {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    
    pub fn meters(x: f64, y: f64, z: f64) -> Self {
        Self::new(x, y, z)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementMode {
    Relative = 0,
    Absolute = 1,
}

impl From<MovementMode> for u32 {
    fn from(mode: MovementMode) -> Self {
        mode as u32
    }
}

impl TryFrom<u32> for MovementMode {
    type Error = crate::error::NanonisError;
    
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(MovementMode::Relative),
            1 => Ok(MovementMode::Absolute),
            _ => Err(crate::error::NanonisError::InvalidCommand(
                format!("Invalid movement mode: {}", value)
            )),
        }
    }
}

/// Trigger and Timing Types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerMode {
    Immediate = 0,
    Level = 1,
    Digital = 2,
}

impl From<TriggerMode> for u16 {
    fn from(mode: TriggerMode) -> Self {
        mode as u16
    }
}

// From implementations for common integer types for convenience
impl From<u16> for TriggerMode {
    fn from(value: u16) -> Self {
        match value {
            0 => TriggerMode::Immediate,
            1 => TriggerMode::Level,
            2 => TriggerMode::Digital,
            _ => TriggerMode::Immediate, // Default fallback
        }
    }
}

impl From<i32> for TriggerMode {
    fn from(value: i32) -> Self {
        TriggerMode::from(value as u16)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerSlope {
    Falling = 0,
    Rising = 1,
}

impl From<TriggerSlope> for u16 {
    fn from(slope: TriggerSlope) -> Self {
        slope as u16
    }
}

impl TryFrom<u16> for TriggerSlope {
    type Error = crate::error::NanonisError;
    
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(TriggerSlope::Falling),
            1 => Ok(TriggerSlope::Rising),
            _ => Err(crate::error::NanonisError::InvalidCommand(
                format!("Invalid trigger slope: {}", value)
            )),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TriggerLevel(pub f64);

impl From<TriggerLevel> for f64 {
    fn from(level: TriggerLevel) -> Self {
        level.0
    }
}

impl From<f64> for TriggerLevel {
    fn from(level: f64) -> Self {
        TriggerLevel(level)
    }
}

impl From<f32> for TriggerLevel {
    fn from(level: f32) -> Self {
        TriggerLevel(level as f64)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SampleCount(pub i32);

impl SampleCount {
    pub fn new(count: i32) -> Self {
        Self(count)
    }
}

impl From<SampleCount> for i32 {
    fn from(samples: SampleCount) -> Self {
        samples.0
    }
}

impl From<i32> for SampleCount {
    fn from(count: i32) -> Self {
        SampleCount(count)
    }
}

impl From<u32> for SampleCount {
    fn from(count: u32) -> Self {
        SampleCount(count as i32)
    }
}

impl From<usize> for SampleCount {
    fn from(count: usize) -> Self {
        SampleCount(count as i32)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TimeoutMs(pub i32);

impl TimeoutMs {
    pub fn milliseconds(ms: i32) -> Self {
        Self(ms)
    }
    
    pub fn indefinite() -> Self {
        Self(-1)
    }
}

impl From<TimeoutMs> for i32 {
    fn from(timeout: TimeoutMs) -> Self {
        timeout.0
    }
}

impl From<i32> for TimeoutMs {
    fn from(timeout: i32) -> Self {
        TimeoutMs(timeout)
    }
}

impl From<u32> for TimeoutMs {
    fn from(timeout: u32) -> Self {
        TimeoutMs(timeout as i32)
    }
}

impl From<Duration> for TimeoutMs {
    fn from(duration: Duration) -> Self {
        TimeoutMs(duration.as_millis() as i32)
    }
}

/// Scan Types
#[derive(Debug, Clone, Copy)]
pub struct ScanFrame {
    pub center: Position,
    pub width_m: f32,
    pub height_m: f32,
    pub angle_deg: f32,
}

impl ScanFrame {
    pub fn new(center: Position, width_m: f32, height_m: f32, angle_deg: f32) -> Self {
        Self {
            center,
            width_m,
            height_m,
            angle_deg,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanAction {
    Start = 0,
    Stop = 1,
    Pause = 2,
    Resume = 3,
    Freeze = 4,
    Unfreeze = 5,
    GoToCenter = 6,
}

impl From<ScanAction> for u16 {
    fn from(action: ScanAction) -> Self {
        action as u16
    }
}

impl TryFrom<u16> for ScanAction {
    type Error = crate::error::NanonisError;
    
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ScanAction::Start),
            1 => Ok(ScanAction::Stop),
            2 => Ok(ScanAction::Pause),
            3 => Ok(ScanAction::Resume),
            4 => Ok(ScanAction::Freeze),
            5 => Ok(ScanAction::Unfreeze),
            6 => Ok(ScanAction::GoToCenter),
            _ => Err(crate::error::NanonisError::InvalidCommand(
                format!("Invalid scan action: {}", value)
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanDirection {
    Down = 0,
    Up = 1,
}

impl From<ScanDirection> for u32 {
    fn from(direction: ScanDirection) -> Self {
        direction as u32
    }
}

impl TryFrom<u32> for ScanDirection {
    type Error = crate::error::NanonisError;
    
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ScanDirection::Down),
            1 => Ok(ScanDirection::Up),
            _ => Err(crate::error::NanonisError::InvalidCommand(
                format!("Invalid scan direction: {}", value)
            )),
        }
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
    #[serde(skip)]
    pub decision_value_history: VecDeque<f32>, // History of processed values that led to each decision

    // System state
    pub last_action: Option<String>, // Last action executed
    pub system_parameters: Vec<f32>, // Configurable system params

    // Classification result
    pub classification: TipState, // How the classifier interpreted this state

                                  // For future ML/transformer expansion:
                                  // pub embedding: Option<Vec<f32>>,         // Learned state representation
                                  // pub attention_weights: Option<Vec<f32>>, // Transformer attention scores
                                  // pub confidence: f32,                     // Model confidence in decision
}
