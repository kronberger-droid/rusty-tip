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
                let _ = w.flush();
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
    events: Mutex<Vec<Event>>,
    max_events: usize,
}

impl EventAccumulator {
    pub fn new(max_events: usize) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            max_events,
        }
    }

    /// Return a clone of the most recent events.
    pub fn recent(&self, n: usize) -> Vec<Event> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        let start = events.len().saturating_sub(n);
        events[start..].to_vec()
    }

    /// Return all accumulated events.
    pub fn all(&self) -> Vec<Event> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.clone()
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
            events.remove(0);
        }
        events.push(event.clone());
    }
}

/// Prints human-readable event summaries to stderr.
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
