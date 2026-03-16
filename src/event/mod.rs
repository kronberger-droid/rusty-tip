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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    // -- Event construction tests --

    #[test]
    fn event_action_started_has_timestamp() {
        let event = Event::action_started("read_bias", serde_json::json!({}));
        match event {
            Event::ActionStarted { action, timestamp, .. } => {
                assert_eq!(action, "read_bias");
                assert!(timestamp.elapsed().unwrap().as_secs() < 1);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn event_action_completed_converts_output() {
        let output = ActionOutput::Value(3.14);
        let event = Event::action_completed("read_bias", &output, Duration::from_millis(50));
        match event {
            Event::ActionCompleted { output, duration, .. } => {
                assert!((output.as_f64().unwrap() - 3.14).abs() < 1e-10);
                assert!(duration.as_millis() == 50);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn event_action_completed_unit_is_null() {
        let event = Event::action_completed("wait", &ActionOutput::Unit, Duration::ZERO);
        match event {
            Event::ActionCompleted { output, .. } => {
                assert!(output.is_null());
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn event_action_failed() {
        let event = Event::action_failed("set_bias", "timeout", Duration::from_millis(100));
        match event {
            Event::ActionFailed { action, error, .. } => {
                assert_eq!(action, "set_bias");
                assert_eq!(error, "timeout");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn event_data_collected() {
        let event = Event::data_collected("z_height", serde_json::json!(1.23));
        match event {
            Event::DataCollected { label, value, .. } => {
                assert_eq!(label, "z_height");
                assert!((value.as_f64().unwrap() - 1.23).abs() < 1e-10);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn event_custom() {
        let event = Event::custom("workflow_started", serde_json::json!({"name": "prep"}));
        match event {
            Event::Custom { kind, data } => {
                assert_eq!(kind, "workflow_started");
                assert_eq!(data["name"], "prep");
            }
            _ => panic!("Wrong variant"),
        }
    }

    // -- Serialization tests --

    #[test]
    fn event_serializes_to_json() {
        let event = Event::action_started("read_bias", serde_json::json!({}));
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"action_started\""));
        assert!(json.contains("\"action\":\"read_bias\""));
        assert!(json.contains("\"timestamp\""));
    }

    #[test]
    fn duration_serializes_as_milliseconds() {
        let event = Event::action_completed(
            "test",
            &ActionOutput::Unit,
            Duration::from_millis(1234),
        );
        let json = serde_json::to_value(&event).unwrap();
        let dur_ms = json["duration"].as_f64().unwrap();
        assert!((dur_ms - 1234.0).abs() < 1.0);
    }

    #[test]
    fn timestamp_serializes_as_unix_epoch() {
        let event = Event::action_started("test", serde_json::json!({}));
        let json = serde_json::to_value(&event).unwrap();
        let ts = json["timestamp"].as_f64().unwrap();
        // Should be a reasonable Unix timestamp (after year 2020)
        assert!(ts > 1_577_836_800.0, "Timestamp {} too small", ts);
    }

    // -- EventBus tests --

    struct CountingObserver {
        count: Arc<AtomicUsize>,
    }

    impl Observer for CountingObserver {
        fn on_event(&self, _event: &Event) {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn event_bus_broadcasts_to_all_observers() {
        let count1 = Arc::new(AtomicUsize::new(0));
        let count2 = Arc::new(AtomicUsize::new(0));

        let mut bus = EventBus::new();
        bus.add_observer(Box::new(CountingObserver { count: count1.clone() }));
        bus.add_observer(Box::new(CountingObserver { count: count2.clone() }));

        bus.emit(Event::custom("test", serde_json::json!({})));
        bus.emit(Event::custom("test2", serde_json::json!({})));

        assert_eq!(count1.load(Ordering::SeqCst), 2);
        assert_eq!(count2.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn event_bus_empty_does_not_panic() {
        let bus = EventBus::new();
        bus.emit(Event::custom("test", serde_json::json!({})));
        // No panic = pass
    }

    #[test]
    fn null_emitter_does_not_panic() {
        let emitter = NullEmitter;
        emitter.emit(Event::custom("test", serde_json::json!({})));
    }
}
