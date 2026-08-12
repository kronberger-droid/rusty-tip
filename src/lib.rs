// -- Hardware abstraction --
pub mod mock_controller;
pub mod nanonis_controller;
pub mod spm_controller;
pub mod spm_error;

// -- Actions and orchestration --
pub mod action;
pub mod routine;
pub mod tip_prep;

// -- Configuration and observability --
pub mod config;
pub mod controller_types;
pub mod event;
pub mod shutdown;
pub mod signal_registry;

// -- Analysis and display --
pub mod analyzer;
pub mod frame;
pub mod plotting;
pub mod types;

// -- Internal plumbing (not part of the public API) --
mod buffered_tcp_reader;
pub(crate) mod utils;

pub use controller_types::{
    BiasSweepPolarity, PolaritySign, PulseMethod, RandomPolaritySwitch, StabilityConfig,
};
pub use frame::{Frame, FrameGeometry, ToNpyPayload};
pub use plotting::{plot_values, plot_values_with_range};
pub use routine::{Outcome, Routine, Rt, run_routine};
pub use shutdown::ShutdownFlag;
pub use signal_registry::{Signal, SignalIndex, SignalRegistry};

// Re-export nanonis-rs core types
pub use nanonis_rs::{
    ConnectionConfig, NanonisClient, NanonisClientBuilder, NanonisError, NanonisValue, Position,
    TCPLoggerStream,
};

// Re-export nanonis-rs motor types
pub use nanonis_rs::motor::{
    Amplitude, Frequency, MotorAxis, MotorDirection, MotorGroup, MotorMovement, MovementMode,
    Position3D, StepCount,
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
pub use nanonis_rs::tcplog::TCPLogStatus;

// Re-export nanonis-rs tip recovery types
pub use nanonis_rs::tip_recovery::TipShaperConfig;

// Re-export nanonis-rs z spectroscopy types
pub use nanonis_rs::z_spectr::ZSpectroscopyResult;
