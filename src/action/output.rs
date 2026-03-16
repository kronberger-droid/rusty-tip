use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_serializes_tagged() {
        let output = ActionOutput::Value(3.14);
        let v = serde_json::to_value(&output).unwrap();
        assert_eq!(v["type"], "value");
        assert_eq!(v["data"], 3.14);
    }

    #[test]
    fn values_serializes_tagged() {
        let output = ActionOutput::Values(vec![
            ("x".into(), 1.0),
            ("y".into(), 2.0),
        ]);
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
