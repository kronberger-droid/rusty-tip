use std::collections::HashMap;

use serde::de::DeserializeOwned;

use super::{Action, Result};
use crate::machine_state::{ActionKind, StateField};
use crate::spm_error::SpmError;

/// Factory function that creates an action instance from JSON parameters.
pub type ActionFactory =
    Box<dyn Fn(serde_json::Value) -> Result<Box<dyn Action>> + Send + Sync>;

/// Factory function that creates a default-parameter resolver action
/// without going through JSON deserialization. Used by the executor's
/// auto-resolve path where the action must be constructed from a
/// `StateField` lookup, not a name + JSON.
pub type ResolverFactory = Box<dyn Fn() -> Box<dyn Action> + Send + Sync>;

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
///
/// Query actions that declare `resolves()` are also indexed by
/// `StateField` so the executor can auto-insert them to resolve unknown
/// state before running a gated action.
pub struct ActionRegistry {
    factories: HashMap<String, ActionFactory>,
    descriptions: HashMap<String, String>,
    resolvers: HashMap<StateField, ResolverFactory>,
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            descriptions: HashMap::new(),
            resolvers: HashMap::new(),
        }
    }

    /// Register an action type.
    ///
    /// The action must implement `Action + DeserializeOwned` so it can
    /// be constructed from JSON parameters. A temporary instance is created
    /// to extract the name and description for the registry.
    ///
    /// If the action is a `Query` that declares any `resolves()` fields,
    /// it is also indexed as the resolver for those fields.
    pub fn register<A>(&mut self)
    where
        A: Action + DeserializeOwned + 'static,
        A: Default,
    {
        let sample = A::default();
        let name = sample.name().to_string();
        let desc = sample.description().to_string();
        let kind = sample.kind();
        let resolves = sample.resolves();

        if self.factories.contains_key(&name) {
            log::warn!(
                "ActionRegistry: overwriting existing action '{}'",
                name
            );
        }

        self.factories.insert(
            name.clone(),
            Box::new(|params: serde_json::Value| {
                let action: A =
                    serde_json::from_value(params).map_err(|e| {
                        SpmError::Protocol(format!(
                            "Failed to deserialize action: {}",
                            e
                        ))
                    })?;
                Ok(Box::new(action) as Box<dyn Action>)
            }),
        );
        self.descriptions.insert(name.clone(), desc);

        if kind == ActionKind::Query {
            for field in resolves {
                if self.resolvers.contains_key(&field) {
                    log::warn!(
                        "ActionRegistry: overwriting resolver for {:?} with '{}'",
                        field,
                        name
                    );
                }
                self.resolvers.insert(
                    field,
                    Box::new(|| Box::new(A::default()) as Box<dyn Action>),
                );
            }
        }
    }

    /// Instantiate the resolver (default-parameter Query action) for a state
    /// field, if one is registered. Returns `None` if no Query action was
    /// registered with this field in its `resolves()`.
    pub fn resolver_for(&self, field: StateField) -> Option<Box<dyn Action>> {
        self.resolvers.get(&field).map(|factory| factory())
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
    pub fn create(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> Result<Box<dyn Action>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, ActionContext, ActionOutput};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    struct DummyAction {
        #[serde(default)]
        value: f64,
    }

    impl Action for DummyAction {
        fn name(&self) -> &str {
            "dummy"
        }
        fn description(&self) -> &str {
            "A test action"
        }
        fn execute(
            &self,
            _ctx: &mut ActionContext,
        ) -> crate::action::Result<ActionOutput> {
            Ok(ActionOutput::Value(self.value))
        }
    }

    #[test]
    fn register_and_create() {
        let mut reg = ActionRegistry::new();
        reg.register::<DummyAction>();
        assert!(reg.has("dummy"));

        let action = reg
            .create("dummy", serde_json::json!({"value": 42.0}))
            .unwrap();
        assert_eq!(action.name(), "dummy");
    }

    #[test]
    fn create_unknown_action_fails() {
        let reg = ActionRegistry::new();
        let result = reg.create("nonexistent", serde_json::json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn create_with_bad_params_fails() {
        let mut reg = ActionRegistry::new();
        reg.register::<DummyAction>();
        // "value" should be f64, not a string -- but serde is lenient here
        // Use a truly incompatible type: array instead of object
        let result = reg.create("dummy", serde_json::json!([1, 2, 3]));
        assert!(result.is_err());
    }

    #[test]
    fn list_shows_registered_actions() {
        let mut reg = ActionRegistry::new();
        reg.register::<DummyAction>();
        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "dummy");
        assert_eq!(list[0].description, "A test action");
    }

    #[test]
    fn has_returns_false_for_unregistered() {
        let reg = ActionRegistry::new();
        assert!(!reg.has("dummy"));
    }

    #[test]
    fn builtin_registry_has_expected_actions() {
        let reg = crate::action::builtin_registry();
        assert!(reg.has("read_bias"));
        assert!(reg.has("set_bias"));
        assert!(reg.has("bias_pulse"));
        assert!(reg.has("read_signal"));
        assert!(reg.has("withdraw"));
        assert!(reg.has("auto_approach"));
        assert!(reg.has("scan_control"));
        assert!(reg.has("grab_scan_frame"));
        assert!(reg.has("wait"));
        assert!(reg.has("tip_shape"));
        assert!(reg.has("center_freq_shift"));

        let list = reg.list();
        assert!(
            list.len() >= 20,
            "Should have 20+ built-in actions, got {}",
            list.len()
        );
    }

    #[test]
    fn builtin_actions_all_have_descriptions() {
        let reg = crate::action::builtin_registry();
        for info in reg.list() {
            assert!(
                !info.description.is_empty(),
                "Action '{}' should have a description",
                info.name
            );
        }
    }

    #[test]
    fn create_with_default_params() {
        let mut reg = ActionRegistry::new();
        reg.register::<DummyAction>();
        // Empty object should use defaults
        let action = reg.create("dummy", serde_json::json!({})).unwrap();
        assert_eq!(action.name(), "dummy");
    }
}
