use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::spm_error::SpmError;

/// What an action returns after execution.
///
/// Designed to cover the common return shapes of SPM operations
/// while keeping everything serializable for logging and LLM inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ActionOutput {
    /// Single numeric value (e.g. bias voltage, signal reading)
    Value(f64),
    /// Multiple labeled values (e.g. multi-signal read)
    Values(Vec<(String, f64)>),
    /// Structured data for complex returns (oscilloscope data, tip state, etc.)
    Data(serde_json::Value),
    /// Action completed with no meaningful return value
    Unit,
}

impl ActionOutput {
    /// Wrap an action's result struct as [`ActionOutput::Data`].
    pub fn data<T: Serialize>(action: &str, result: &T) -> Result<Self, SpmError> {
        serde_json::to_value(result)
            .map(Self::Data)
            .map_err(|e| SpmError::Workflow(format!("{action}: result does not serialize: {e}")))
    }

    /// Take back the result struct an action returned as [`ActionOutput::Data`].
    pub fn into_data<T: DeserializeOwned>(self, action: &str) -> Result<T, SpmError> {
        match self {
            Self::Data(value) => serde_json::from_value(value)
                .map_err(|e| SpmError::Workflow(format!("{action}: unexpected result shape: {e}"))),
            other => Err(SpmError::Workflow(format!(
                "{action}: expected structured data, got {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::f64;

    use super::*;

    #[test]
    fn value_serializes_tagged() {
        let output = ActionOutput::Value(f64::consts::PI);
        let v = serde_json::to_value(&output).unwrap();
        assert_eq!(v["type"], "value");
        assert_eq!(v["data"], f64::consts::PI);
    }

    #[test]
    fn values_serializes_tagged() {
        let output = ActionOutput::Values(vec![("x".into(), 1.0), ("y".into(), 2.0)]);
        let v = serde_json::to_value(&output).unwrap();
        assert_eq!(v["type"], "values");
        assert!(v["data"].is_array());
    }

    #[test]
    fn data_serializes_tagged() {
        let output = ActionOutput::Data(serde_json::json!({"key": "val"}));
        let v = serde_json::to_value(&output).unwrap();
        assert_eq!(v["type"], "data");
        assert_eq!(v["data"]["key"], "val");
    }

    #[test]
    fn unit_serializes_tagged() {
        let output = ActionOutput::Unit;
        let v = serde_json::to_value(&output).unwrap();
        assert_eq!(v["type"], "unit");
    }

    #[test]
    fn all_variants_are_clone() {
        let v = ActionOutput::Value(1.0);
        let v2 = v.clone();
        let s1 = serde_json::to_value(&v).unwrap();
        let s2 = serde_json::to_value(&v2).unwrap();
        assert_eq!(s1, s2);
    }

    #[test]
    fn debug_format_works() {
        let v = ActionOutput::Value(1.0);
        let s = format!("{:?}", v);
        assert!(s.contains("Value"));
    }
}
