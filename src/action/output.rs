use serde::Serialize;

/// What an action returns after execution.
///
/// Designed to cover the common return shapes of SPM operations
/// while keeping everything serializable for logging and LLM inspection.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
