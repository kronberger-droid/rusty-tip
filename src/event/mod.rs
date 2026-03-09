mod observer;

pub use observer::{ChannelForwarder, ConsoleLogger, EventAccumulator, FileLogger, Observer};

use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::action::ActionOutput;

/// Structured event emitted during execution.
///
/// All variants are `Clone + Serialize` so they can be broadcast to multiple
/// observers, written to JSONL logs, and accumulated for LLM context windows.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Emitted by the executor before running an action.
    ActionStarted {
        action: String,
        params: serde_json::Value,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
    /// Emitted by the executor after an action completes successfully.
    ActionCompleted {
        action: String,
        output: serde_json::Value,
        #[serde(with = "duration_ms_serde")]
        duration: Duration,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
    /// Emitted by the executor when an action fails.
    ActionFailed {
        action: String,
        error: String,
        #[serde(with = "duration_ms_serde")]
        duration: Duration,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
    /// Emitted by actions that produce measurements.
    DataCollected {
        label: String,
        value: serde_json::Value,
        #[serde(with = "system_time_serde")]
        timestamp: SystemTime,
    },
    /// Escape hatch for domain-specific events from user-defined actions.
    Custom {
        kind: String,
        data: serde_json::Value,
    },
}

impl Event {
    pub fn action_started(action: &str, params: serde_json::Value) -> Self {
        Event::ActionStarted {
            action: action.into(),
            params,
            timestamp: SystemTime::now(),
        }
    }

    pub fn action_completed(action: &str, output: &ActionOutput, duration: Duration) -> Self {
        let output_json = match output {
            ActionOutput::Value(v) => serde_json::json!(v),
            ActionOutput::Values(vs) => serde_json::json!(vs),
            ActionOutput::Data(d) => d.clone(),
            ActionOutput::Unit => serde_json::Value::Null,
        };
        Event::ActionCompleted {
            action: action.into(),
            output: output_json,
            duration,
            timestamp: SystemTime::now(),
        }
    }

    pub fn action_failed(action: &str, error: &str, duration: Duration) -> Self {
        Event::ActionFailed {
            action: action.into(),
            error: error.into(),
            duration,
            timestamp: SystemTime::now(),
        }
    }

    pub fn data_collected(label: &str, value: serde_json::Value) -> Self {
        Event::DataCollected {
            label: label.into(),
            value,
            timestamp: SystemTime::now(),
        }
    }

    pub fn custom(kind: &str, data: serde_json::Value) -> Self {
        Event::Custom {
            kind: kind.into(),
            data,
        }
    }
}

/// Trait for broadcasting events. Passed into `ActionContext`.
pub trait EventEmitter: Send + Sync {
    fn emit(&self, event: Event);
}

/// Broadcasts events to multiple observers.
pub struct EventBus {
    observers: Vec<Box<dyn Observer>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            observers: Vec::new(),
        }
    }

    pub fn add_observer(&mut self, observer: Box<dyn Observer>) {
        self.observers.push(observer);
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventEmitter for EventBus {
    fn emit(&self, event: Event) {
        for observer in &self.observers {
            observer.on_event(&event);
        }
    }
}

/// No-op emitter for contexts that don't need events (e.g. tests).
pub struct NullEmitter;

impl EventEmitter for NullEmitter {
    fn emit(&self, _event: Event) {}
}

// -- Serde helpers --

mod system_time_serde {
    use serde::Serializer;
    use std::time::SystemTime;

    pub fn serialize<S: Serializer>(time: &SystemTime, ser: S) -> Result<S::Ok, S::Error> {
        let duration = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        ser.serialize_f64(duration.as_secs_f64())
    }
}

mod duration_ms_serde {
    use serde::Serializer;
    use std::time::Duration;

    pub fn serialize<S: Serializer>(dur: &Duration, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_f64(dur.as_secs_f64() * 1000.0)
    }
}
