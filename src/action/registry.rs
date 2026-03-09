use std::collections::HashMap;

use serde::de::DeserializeOwned;

use super::{Action, Result};
use crate::spm_error::SpmError;

/// Factory function that creates an action instance from JSON parameters.
pub type ActionFactory = Box<dyn Fn(serde_json::Value) -> Result<Box<dyn Action>> + Send + Sync>;

/// Metadata about a registered action.
#[derive(Debug, Clone)]
pub struct ActionInfo {
    pub name: String,
    pub description: String,
}

/// Registry of available actions.
///
/// Actions are registered by type, and can be instantiated from a name
/// and JSON parameters. This enables runtime discovery (for LLMs) and
/// deserialization of workflows from config files.
pub struct ActionRegistry {
    factories: HashMap<String, ActionFactory>,
    descriptions: HashMap<String, String>,
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            descriptions: HashMap::new(),
        }
    }

    /// Register an action type.
    ///
    /// The action must implement `Action + DeserializeOwned` so it can
    /// be constructed from JSON parameters. A temporary instance is created
    /// to extract the name and description for the registry.
    pub fn register<A>(&mut self)
    where
        A: Action + DeserializeOwned + 'static,
        A: Default,
    {
        let sample = A::default();
        let name = sample.name().to_string();
        let desc = sample.description().to_string();

        self.factories.insert(
            name.clone(),
            Box::new(|params: serde_json::Value| {
                let action: A = serde_json::from_value(params).map_err(|e| {
                    SpmError::Protocol(format!("Failed to deserialize action: {}", e))
                })?;
                Ok(Box::new(action) as Box<dyn Action>)
            }),
        );
        self.descriptions.insert(name, desc);
    }

    /// List all registered actions with their descriptions.
    pub fn list(&self) -> Vec<ActionInfo> {
        self.descriptions
            .iter()
            .map(|(name, desc)| ActionInfo {
                name: name.clone(),
                description: desc.clone(),
            })
            .collect()
    }

    /// Create an action instance from a name and JSON parameters.
    pub fn create(&self, name: &str, params: serde_json::Value) -> Result<Box<dyn Action>> {
        let factory = self.factories.get(name).ok_or_else(|| {
            SpmError::Protocol(format!("Unknown action: {}", name))
        })?;
        factory(params)
    }

    /// Check if an action is registered.
    pub fn has(&self, name: &str) -> bool {
        self.factories.contains_key(name)
    }
}

impl Default for ActionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
