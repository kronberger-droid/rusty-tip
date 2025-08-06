pub mod classifier;
pub mod client;
pub mod controller;
pub mod error;
pub mod policy;
pub mod protocol;
pub mod signal_monitor;
pub mod types;

// Re-export main types for easy access
pub use classifier::{
    BoundaryClassifier,
    // State classification
    StateClassifier,
    TipState,
};
pub use client::{ConnectionConfig, NanonisClient, NanonisClientBuilder};
pub use controller::{Controller, SystemStats};
pub use error::NanonisError;
pub use policy::{
    // Expansion types for ML/transformer policies:
    ActionType,
    ExplainablePolicyEngine,
    LearningPolicyEngine,
    // Policy decisions
    PolicyDecision,
    PolicyEngine,
    RuleBasedPolicy,
};
pub use types::{BiasVoltage, MachineState, NanonisValue, Position};
pub use signal_monitor::{
    AsyncSignalMonitor, JsonDiskWriter, DiskWriter, DiskWriterConfig, DiskWriterFormat,
    SignalReceiver, MonitorStats,
};
