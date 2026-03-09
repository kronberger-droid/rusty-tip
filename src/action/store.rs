use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;

/// Typed key-value store for inter-action communication.
///
/// Uses `serde_json::Value` internally so any serializable type can be
/// stored and retrieved. The serialization boundary means store contents
/// can be logged, inspected by LLMs, and persisted to disk.
#[derive(Debug, Default)]
pub struct DataStore {
    values: HashMap<String, serde_json::Value>,
}

impl DataStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a serializable value under the given key.
    /// Overwrites any existing value at that key.
    pub fn set<T: Serialize>(&mut self, key: &str, value: &T) -> Option<()> {
        let json = serde_json::to_value(value).ok()?;
        self.values.insert(key.to_string(), json);
        Some(())
    }

    /// Retrieve and deserialize a value by key.
    /// Returns None if the key doesn't exist or deserialization fails.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let val = self.values.get(key)?;
        serde_json::from_value(val.clone()).ok()
    }

    /// Remove a value by key, returning the raw JSON if it existed.
    pub fn remove(&mut self, key: &str) -> Option<serde_json::Value> {
        self.values.remove(key)
    }

    /// Check if a key exists in the store.
    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// Get a snapshot of all keys and their raw JSON values.
    /// Useful for logging and LLM inspection.
    pub fn snapshot(&self) -> &HashMap<String, serde_json::Value> {
        &self.values
    }

    /// Clear all stored values.
    pub fn clear(&mut self) {
        self.values.clear();
    }
}
