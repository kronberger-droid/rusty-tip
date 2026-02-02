pub mod action_driver;
pub mod actions;
pub mod buffered_tcp_reader;
pub mod logger;
pub mod plotting;
pub mod signal_registry;
pub mod types;
pub mod utils;

pub use action_driver::{
    stability, ActionDriver, ActionDriverBuilder, ExecutionResult,
    ExecutionStats, TCPReaderConfig,
};
pub use actions::{Action, ActionChain, ActionLogEntry, ActionResult};
pub use logger::Logger;
pub use plotting::{plot_values, plot_values_with_range};
pub use signal_registry::{Signal, SignalRegistry};
pub use types::{
    ChainExperimentData, ExperimentData, SessionMetadata,
    TimestampedSignalFrame, TipShape,
};
pub use utils::{poll_until, poll_with_timeout, PollError};

// Re-export nanonis-rs core types
pub use nanonis_rs::{
    ConnectionConfig, NanonisClient, NanonisClientBuilder, NanonisError,
    NanonisValue, Position, TCPLoggerStream,
};

// Re-export nanonis-rs motor types
pub use nanonis_rs::motor::{
    Amplitude, Frequency, MotorAxis, MotorDirection, MotorGroup,
    MotorMovement, MovementMode, Position3D, StepCount,
};

// Re-export nanonis-rs scan types
pub use nanonis_rs::scan::{ScanAction, ScanConfig, ScanDirection, ScanFrame, ScanPropsBuilder};

// Re-export nanonis-rs oscilloscope types
pub use nanonis_rs::oscilloscope::{
    OscilloscopeIndex, SampleCount, TriggerLevel, TriggerMode, TriggerSlope,
};

// Re-export nanonis-rs bias types
pub use nanonis_rs::bias::PulseMode;

// Re-export nanonis-rs z_ctrl types
pub use nanonis_rs::z_ctrl::ZControllerHold;

// Re-export nanonis-rs signals types
pub use nanonis_rs::signals::SignalFrame;

// Re-export nanonis-rs tcplog types
pub use nanonis_rs::tcplog::{TCPLogStatus, TCPLoggerData};

// Re-export nanonis-rs tip recovery types
pub use nanonis_rs::tip_recovery::TipShaperConfig;

// Re-export nanonis-rs z spectroscopy types
pub use nanonis_rs::z_spectr::ZSpectroscopyResult;
