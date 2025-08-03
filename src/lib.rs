pub mod afm_controller;
pub mod client;
pub mod error;
pub mod policy;
pub mod protocol;
pub mod types;

// Re-export main types for easy access
pub use afm_controller::{AFMController, SystemStats};
pub use client::{ConnectionConfig, NanonisClient, NanonisClientBuilder};
pub use error::NanonisError;
pub use policy::{
    PolicyDecision, PolicyEngine, RuleBasedPolicy,
    // Expansion types for ML/transformer policies:
    TipState, ActionType, LearningPolicyEngine, ExplainablePolicyEngine
};
pub use types::{BiasVoltage, NanonisValue, Position};
