use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Mutex;

use super::Event;

/// Trait for consuming events.
pub trait Observer: Send + Sync {
    fn on_event(&self, event: &Event);
}

/// Writes events as JSONL (one JSON object per line) to a file.
pub struct FileLogger {
    writer: Mutex<BufWriter<File>>,
}

impl FileLogger {
    pub fn new(file: File) -> Self {
        Self {
            writer: Mutex::new(BufWriter::new(file)),
        }
    }
}

impl Observer for FileLogger {
    fn on_event(&self, event: &Event) {
        if let Ok(mut w) = self.writer.lock() {
            if let Ok(json) = serde_json::to_string(event) {
                let _ = writeln!(w, "{json}");
                // BufWriter flushes automatically when its buffer fills
                // or on drop -- no need to flush on every event.
            }
        }
    }
}

/// Forwards events over a crossbeam channel (for GUI or other threads).
pub struct ChannelForwarder {
    sender: crossbeam_channel::Sender<Event>,
}

impl ChannelForwarder {
    pub fn new(sender: crossbeam_channel::Sender<Event>) -> Self {
        Self { sender }
    }
}

impl Observer for ChannelForwarder {
    fn on_event(&self, event: &Event) {
        let _ = self.sender.try_send(event.clone());
    }
}

/// Accumulates events in memory with a bounded capacity.
///
/// Designed for LLM integration: the accumulated events can be serialized
/// into the LLM's context window so it can reason about recent history.
pub struct EventAccumulator {
    events: Mutex<VecDeque<Event>>,
    max_events: usize,
}

impl EventAccumulator {
    pub fn new(max_events: usize) -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
            max_events,
        }
    }

    /// Return a clone of the most recent events.
    pub fn recent(&self, n: usize) -> Vec<Event> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        let start = events.len().saturating_sub(n);
        events.iter().skip(start).cloned().collect()
    }

    /// Return all accumulated events.
    pub fn all(&self) -> Vec<Event> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.iter().cloned().collect()
    }

    /// Clear all accumulated events.
    pub fn clear(&self) {
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.clear();
    }
}

impl Observer for EventAccumulator {
    fn on_event(&self, event: &Event) {
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        if events.len() >= self.max_events {
            events.pop_front();
        }
        events.push_back(event.clone());
    }
}

/// Prints human-readable event summaries to stderr.
///
/// Uses the following format:
/// - `[action] starting: <name>`
/// - `[action] completed: <name> (<ms>ms)`
/// - `[action] FAILED: <name> (<ms>ms): <error>`
/// - `[data] collected: <label>`
/// - `[event] <kind>`
pub struct ConsoleLogger;

impl Observer for ConsoleLogger {
    fn on_event(&self, event: &Event) {
        match event {
            Event::ActionStarted { action, .. } => {
                eprintln!("[action] starting: {action}");
            }
            Event::ActionCompleted {
                action, duration, ..
            } => {
                eprintln!(
                    "[action] completed: {action} ({:.1}ms)",
                    duration.as_secs_f64() * 1000.0
                );
            }
            Event::ActionFailed {
                action,
                error,
                duration,
                ..
            } => {
                eprintln!(
                    "[action] FAILED: {action} ({:.1}ms): {error}",
                    duration.as_secs_f64() * 1000.0
                );
            }
            Event::DataCollected { label, .. } => {
                eprintln!("[data] collected: {label}");
            }
            Event::Custom { kind, .. } => {
                eprintln!("[event] {kind}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(kind: &str) -> Event {
        Event::Custom {
            kind: kind.into(),
            data: serde_json::json!({}),
        }
    }

    // -- EventAccumulator --

    #[test]
    fn accumulator_collects_events() {
        let acc = EventAccumulator::new(10);
        acc.on_event(&make_event("a"));
        acc.on_event(&make_event("b"));
        acc.on_event(&make_event("c"));
        assert_eq!(acc.all().len(), 3);
    }

    #[test]
    fn accumulator_respects_capacity() {
        let acc = EventAccumulator::new(3);
        for i in 0..5 {
            acc.on_event(&make_event(&format!("e{}", i)));
        }
        let all = acc.all();
        assert_eq!(all.len(), 3);
        // Should have the 3 most recent: e2, e3, e4
        match &all[0] {
            Event::Custom { kind, .. } => assert_eq!(kind, "e2"),
            _ => panic!("Wrong variant"),
        }
        match &all[2] {
            Event::Custom { kind, .. } => assert_eq!(kind, "e4"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn accumulator_recent_returns_tail() {
        let acc = EventAccumulator::new(10);
        for i in 0..5 {
            acc.on_event(&make_event(&format!("e{}", i)));
        }
        let recent = acc.recent(2);
        assert_eq!(recent.len(), 2);
        match &recent[0] {
            Event::Custom { kind, .. } => assert_eq!(kind, "e3"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn accumulator_recent_more_than_available() {
        let acc = EventAccumulator::new(10);
        acc.on_event(&make_event("a"));
        let recent = acc.recent(100);
        assert_eq!(recent.len(), 1);
    }

    #[test]
    fn accumulator_clear() {
        let acc = EventAccumulator::new(10);
        acc.on_event(&make_event("a"));
        acc.on_event(&make_event("b"));
        acc.clear();
        assert_eq!(acc.all().len(), 0);
    }

    // -- ChannelForwarder --

    #[test]
    fn channel_forwarder_sends_events() {
        let (tx, rx) = crossbeam_channel::bounded(10);
        let fwd = ChannelForwarder::new(tx);
        fwd.on_event(&make_event("hello"));
        fwd.on_event(&make_event("world"));

        let e1 = rx.try_recv().unwrap();
        let e2 = rx.try_recv().unwrap();
        match e1 {
            Event::Custom { kind, .. } => assert_eq!(kind, "hello"),
            _ => panic!("Wrong variant"),
        }
        match e2 {
            Event::Custom { kind, .. } => assert_eq!(kind, "world"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn channel_forwarder_drops_on_full_channel() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let fwd = ChannelForwarder::new(tx);
        fwd.on_event(&make_event("first"));
        fwd.on_event(&make_event("dropped")); // channel full, should not panic
        match rx.try_recv().unwrap() {
            Event::Custom { kind, .. } => assert_eq!(kind, "first"),
            _ => panic!("Wrong variant"),
        }
        assert!(rx.try_recv().is_err(), "Second event should have been dropped");
    }

    // -- FileLogger --

    #[test]
    fn file_logger_writes_jsonl() {
        let tmp = std::env::temp_dir().join("rusty_tip_test_file_logger.jsonl");
        {
            let file = std::fs::File::create(&tmp).unwrap();
            let logger = FileLogger::new(file);
            logger.on_event(&make_event("test_event"));
        }
        let content = std::fs::read_to_string(&tmp).unwrap();
        assert!(content.contains("test_event"));
        assert!(content.contains("\"type\":\"custom\""));
        let _ = std::fs::remove_file(&tmp);
    }
}
