pub mod classifier;
pub mod client;
pub mod controller;
pub mod error;
pub mod policy;
pub mod protocol;
pub mod types;

// Re-export main types for easy access
pub use controller::{Controller, SystemStats};
pub use client::{ConnectionConfig, NanonisClient, NanonisClientBuilder};
pub use error::NanonisError;
pub use classifier::{
    // State classification
    StateClassifier, BoundaryClassifier, TipState,
};
pub use policy::{
    // Policy decisions  
    PolicyDecision, PolicyEngine, RuleBasedPolicy,
    // Expansion types for ML/transformer policies:
    ActionType, LearningPolicyEngine, ExplainablePolicyEngine
};
pub use types::{BiasVoltage, NanonisValue, Position, MachineState};
