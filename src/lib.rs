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

// Re-export nanonis-rs types
pub use nanonis_rs::{
    Amplitude, ConnectionConfig, Frequency, MotorAxis, MotorDirection,
    MotorGroup, MotorMovement, MovementMode, NanonisClient,
    NanonisClientBuilder, NanonisError, NanonisValue, OscilloscopeIndex,
    Position, Position3D, PulseMode, SampleCount, ScanAction, ScanDirection,
    ScanFrame, SignalFrame, StepCount, TCPLogStatus, TCPLoggerData,
    TCPLoggerStream, TipShaperConfig, TipShaperProps, TriggerLevel,
    TriggerMode, TriggerSlope, ZControllerHold, ZSpectroscopyResult,
};
