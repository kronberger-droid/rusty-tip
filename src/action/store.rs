use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;

use crate::spm_error::SpmError;

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
    pub fn set<T: Serialize>(
        &mut self,
        key: &str,
        value: &T,
    ) -> Result<(), SpmError> {
        let json = serde_json::to_value(value).map_err(|e| {
            SpmError::Protocol(format!(
                "Failed to serialize value for key '{}': {}",
                key, e
            ))
        })?;
        self.values.insert(key.to_string(), json);
        Ok(())
    }

    /// Retrieve and deserialize a value by key.
    /// Returns None if the key doesn't exist or deserialization fails.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let val = self.values.get(key)?;
        serde_json::from_value(val.clone()).ok()
    }

    /// Get the raw JSON value by key without deserialization.
    pub fn get_raw(&self, key: &str) -> Option<&serde_json::Value> {
        self.values.get(key)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[test]
    fn store_and_retrieve_f64() {
        let mut store = DataStore::new();
        store.set("bias", &1.5f64).unwrap();
        let val: f64 = store.get("bias").unwrap();
        assert!((val - 1.5).abs() < 1e-10);
    }

    #[test]
    fn store_and_retrieve_string() {
        let mut store = DataStore::new();
        store.set("channel", &"Z".to_string()).unwrap();
        let val: String = store.get("channel").unwrap();
        assert_eq!(val, "Z");
    }

    #[test]
    fn store_and_retrieve_struct() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct Point {
            x: f64,
            y: f64,
        }

        let mut store = DataStore::new();
        store.set("pos", &Point { x: 1.0, y: 2.0 }).unwrap();
        let val: Point = store.get("pos").unwrap();
        assert_eq!(val, Point { x: 1.0, y: 2.0 });
    }

    #[test]
    fn get_missing_key_returns_none() {
        let store = DataStore::new();
        let val: Option<f64> = store.get("nope");
        assert!(val.is_none());
    }

    #[test]
    fn get_wrong_type_returns_none() {
        let mut store = DataStore::new();
        store.set("bias", &1.5f64).unwrap();
        let val: Option<Vec<String>> = store.get("bias");
        assert!(val.is_none());
    }

    #[test]
    fn overwrite_key() {
        let mut store = DataStore::new();
        store.set("v", &1.0f64).unwrap();
        store.set("v", &2.0f64).unwrap();
        let val: f64 = store.get("v").unwrap();
        assert!((val - 2.0).abs() < 1e-10);
    }

    #[test]
    fn get_raw() {
        let mut store = DataStore::new();
        store.set("x", &42i32).unwrap();
        let raw = store.get_raw("x").unwrap();
        assert_eq!(raw.as_i64(), Some(42));
    }

    #[test]
    fn remove_returns_value() {
        let mut store = DataStore::new();
        store.set("k", &"hello").unwrap();
        let removed = store.remove("k");
        assert!(removed.is_some());
        assert!(!store.contains("k"));
    }

    #[test]
    fn remove_missing_returns_none() {
        let mut store = DataStore::new();
        assert!(store.remove("nope").is_none());
    }

    #[test]
    fn contains() {
        let mut store = DataStore::new();
        assert!(!store.contains("k"));
        store.set("k", &1).unwrap();
        assert!(store.contains("k"));
    }

    #[test]
    fn snapshot_reflects_state() {
        let mut store = DataStore::new();
        store.set("a", &1).unwrap();
        store.set("b", &2).unwrap();
        let snap = store.snapshot();
        assert_eq!(snap.len(), 2);
        assert!(snap.contains_key("a"));
        assert!(snap.contains_key("b"));
    }

    #[test]
    fn clear_empties_store() {
        let mut store = DataStore::new();
        store.set("a", &1).unwrap();
        store.set("b", &2).unwrap();
        store.clear();
        assert!(!store.contains("a"));
        assert_eq!(store.snapshot().len(), 0);
    }

    #[test]
    fn store_json_value_directly() {
        let mut store = DataStore::new();
        let val = serde_json::json!({"nested": [1, 2, 3]});
        store.set("complex", &val).unwrap();
        let raw = store.get_raw("complex").unwrap();
        assert_eq!(raw["nested"][1], 2);
    }
}
