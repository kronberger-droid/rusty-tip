// Re-export nanonis-rs types from their respective submodules
pub use nanonis_rs::motor::{
    Amplitude, Frequency, MotorAxis, MotorDirection, MotorDisplacement, MotorGroup, MotorMovement,
    MovementMode, Position3D, StepCount,
};
pub use nanonis_rs::oscilloscope::{
    OsciData, OsciTriggerMode, OscilloscopeIndex, OversamplingIndex, SampleCount, TimebaseIndex,
    TriggerConfig, TriggerLevel, TriggerMode, TriggerSlope,
};

pub use nanonis_rs::Position;
pub use nanonis_rs::bias::PulseMode;
pub use nanonis_rs::scan::{ScanAction, ScanConfig, ScanDirection, ScanFrame};
pub use nanonis_rs::signals::SignalFrame;
pub use nanonis_rs::tcplog::TCPLogStatus;
pub use nanonis_rs::z_ctrl::ZControllerHold;

use std::time::{Duration, Instant};

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
