use serde::{Deserialize, Serialize};

// Re-export nanonis-rs types from their respective submodules
pub use nanonis_rs::motor::{
    Amplitude, Frequency, MotorAxis, MotorDirection, MotorDisplacement,
    MotorGroup, MotorMovement, MovementMode, Position3D, StepCount,
};
pub use nanonis_rs::oscilloscope::{
    OsciData, OsciTriggerMode, OscilloscopeIndex, OversamplingIndex,
    SampleCount, TimebaseIndex, TriggerConfig, TriggerLevel, TriggerMode,
    TriggerSlope,
};

/// Signal stability statistics for oscilloscope data analysis.
///
/// Previously lived in nanonis-rs but is application-level analysis,
/// not protocol data.
#[derive(Debug, Clone)]
pub struct SignalStats {
    pub mean: f64,
    pub std_dev: f64,
    pub relative_std: f64,
    pub window_size: usize,
    pub stability_method: String,
}

/// Extension trait adding stability analysis fields to `OsciData`.
///
/// The base `OsciData` in nanonis-rs is now a pure protocol type.
/// Stability tracking is application-level and lives here.
#[derive(Debug, Clone)]
pub struct StableOsciData {
    pub osci: OsciData,
    pub signal_stats: Option<SignalStats>,
    pub is_stable: bool,
    pub fallback_value: Option<f64>,
}

impl StableOsciData {
    pub fn stable(osci: OsciData) -> Self {
        Self {
            osci,
            signal_stats: None,
            is_stable: true,
            fallback_value: None,
        }
    }

    pub fn with_stats(osci: OsciData, stats: SignalStats) -> Self {
        Self {
            osci,
            signal_stats: Some(stats),
            is_stable: true,
            fallback_value: None,
        }
    }

    pub fn unstable_with_fallback(osci: OsciData, fallback: f64) -> Self {
        Self {
            osci,
            signal_stats: None,
            is_stable: false,
            fallback_value: Some(fallback),
        }
    }
}
pub use nanonis_rs::Position;
pub use nanonis_rs::bias::PulseMode;
pub use nanonis_rs::scan::{ScanAction, ScanConfig, ScanDirection, ScanFrame};
pub use nanonis_rs::signals::SignalFrame;
pub use nanonis_rs::tcplog::{TCPLogStatus, TCPLoggerData};
pub use nanonis_rs::z_ctrl::ZControllerHold;
// DataToGet is extended locally with Stable variant

use std::time::{Duration, Instant};

/// Simple tip shape - matches original controller
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum TipShape {
    #[default]
    Blunt,
    Sharp,
    Stable,
}

/// Session metadata for signal tracking
#[derive(Debug, Clone)]
pub struct SessionMetadata {
    pub session_id: String,
    pub signal_names: Vec<String>, // All signal names
    pub active_indices: Vec<usize>, // Which signals are being monitored
    pub primary_signal_index: usize, // Index of the primary signal
    pub session_start: f64,        // Session start timestamp
}

/// Extended DataToGet with application-specific Stable variant
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DataToGet {
    Current,
    NextTrigger,
    Wait2Triggers,
    Stable { readings: u32, timeout: Duration },
}

/// Timestamped version of SignalFrame for efficient buffering
#[derive(Debug, Clone)]
pub struct TimestampedSignalFrame {
    /// The lightweight signal frame
    pub signal_frame: SignalFrame,
    /// High-resolution timestamp when frame was received
    pub timestamp: Instant,
    /// Time relative to collection start
    pub relative_time: Duration,
}

impl TimestampedSignalFrame {
    /// Create a new timestamped signal frame from lightweight signal frame
    /// Just adds high-resolution timestamp to existing SignalFrame
    pub fn new(signal_frame: SignalFrame, start_time: Instant) -> Self {
        let timestamp = Instant::now();
        Self {
            signal_frame,
            timestamp,
            relative_time: timestamp.duration_since(start_time),
        }
    }
}

/// Result of an auto-approach operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoApproachResult {
    /// Auto-approach completed successfully
    Success,
    /// Auto-approach timed out before completion
    Timeout,
    /// Auto-approach failed (e.g., hardware error, abnormal termination)
    Failed(String),
    /// Auto-approach was already running when attempted to start
    AlreadyRunning,
    /// Auto-approach was cancelled/stopped externally
    Cancelled,
}

impl AutoApproachResult {
    /// Check if the result represents a successful operation
    pub fn is_success(&self) -> bool {
        matches!(self, AutoApproachResult::Success)
    }

    /// Check if the result represents a failure
    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            AutoApproachResult::Failed(_) | AutoApproachResult::Timeout
        )
    }

    /// Get error description if this is a failure
    pub fn error_message(&self) -> Option<&str> {
        match self {
            AutoApproachResult::Failed(msg) => Some(msg),
            AutoApproachResult::Timeout => Some("Auto-approach timed out"),
            AutoApproachResult::AlreadyRunning => {
                Some("Auto-approach already running")
            }
            AutoApproachResult::Cancelled => {
                Some("Auto-approach was cancelled")
            }
            AutoApproachResult::Success => None,
        }
    }
}

/// Status information for auto-approach operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoApproachStatus {
    /// Auto-approach is not running
    Idle,
    /// Auto-approach is currently running
    Running,
    /// Auto-approach status is unknown (e.g., communication error)
    Unknown,
}
