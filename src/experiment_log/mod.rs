//! What a run writes about itself, and how a tool declares the events it
//! logs.
//!
//! An experiment log is a JSONL file: one JSON object per line, each line an
//! [`Event`](crate::event::Event) wrapped in an envelope that adds a sequence
//! number. The first line is a `run_started` event carrying a [`RunHeader`],
//! the last is `run_finished`. Everything a reader needs to interpret the
//! lines in between is in the header, including the schema of every custom
//! event the tool can emit, so a log is self-describing and a reader written
//! against the header works for any tool.
//!
//! # Declaring events
//!
//! A tool's custom events are plain structs. Each implements [`LogEvent`]
//! with a kind name namespaced by tool (`tip_prep/cycle`), and the tool lists
//! them once in a [`ToolSchema`]:
//!
//! ```
//! use rusty_tip::experiment_log::{LogEvent, ToolSchema};
//! use schemars::JsonSchema;
//! use serde::Serialize;
//!
//! #[derive(Serialize, JsonSchema)]
//! struct Cycle { cycle: usize, freq_shift_hz: f64 }
//!
//! impl LogEvent for Cycle {
//!     const KIND: &'static str = "demo/cycle";
//! }
//!
//! let schema = ToolSchema::new("demo").with::<Cycle>();
//! assert_eq!(schema.kinds[0].kind, "demo/cycle");
//! ```
//!
//! The JSON Schema for each kind is generated from the struct, so the
//! declaration cannot drift from what is written. A snapshot test per tool
//! pins the generated schema in the repository; changing a field changes the
//! snapshot, which is the point.

use schemars::{JsonSchema, schema_for};
use serde::Serialize;

use crate::signal_registry::SignalRegistry;
use crate::spm_controller::SpmController;

/// Version of the line format and the built-in event variants. Bump when a
/// reader of the previous version would misread a log.
pub const ENVELOPE_VERSION: u32 = 1;

/// A custom event a tool can write. `KIND` is `tool/name`.
pub trait LogEvent: Serialize + JsonSchema {
    const KIND: &'static str;
}

/// The declared schema of one custom event kind.
#[derive(Serialize, Clone, Debug)]
pub struct EventKindSchema {
    pub kind: String,
    /// JSON Schema of the event's `data`, generated from the Rust type.
    pub schema: serde_json::Value,
}

/// Every custom event kind a tool can emit, with its schema.
#[derive(Serialize, Clone, Debug)]
pub struct ToolSchema {
    pub tool: String,
    pub kinds: Vec<EventKindSchema>,
}

impl ToolSchema {
    pub fn new(tool: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            kinds: Vec::new(),
        }
    }

    /// Declare one event type.
    pub fn with<E: LogEvent>(mut self) -> Self {
        let schema =
            serde_json::to_value(schema_for!(E)).expect("a generated JSON Schema serializes");
        self.kinds.push(EventKindSchema {
            kind: E::KIND.to_string(),
            schema,
        });
        self
    }

    /// Add the kinds of another schema, for events a shared layer emits on
    /// the tool's behalf (the routine harness, say).
    pub fn including(mut self, other: ToolSchema) -> Self {
        self.kinds.extend(other.kinds);
        self
    }
}

/// The first line of a log: what produced it and how to read it.
#[derive(Serialize, Clone, Debug)]
pub struct RunHeader {
    pub tool: String,
    /// Crate version.
    pub version: String,
    /// Short git commit the binary was built from, when built in a checkout.
    pub git_commit: Option<String>,
    pub envelope_version: u32,
    /// The configuration as loaded, after defaults were applied.
    pub config: serde_json::Value,
    /// What was learned about the controller at startup.
    pub controller: serde_json::Value,
    pub schema: ToolSchema,
}

impl RunHeader {
    /// `config` and `controller` are whatever the tool has; `serde_json::Value::Null`
    /// is fine for a tool without a config file.
    pub fn new(schema: ToolSchema, config: impl Serialize, controller: impl Serialize) -> Self {
        let commit = env!("RUSTY_TIP_GIT_COMMIT");
        Self {
            tool: schema.tool.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            git_commit: (!commit.is_empty()).then(|| commit.to_string()),
            envelope_version: ENVELOPE_VERSION,
            config: serde_json::to_value(config).unwrap_or(serde_json::Value::Null),
            controller: serde_json::to_value(controller).unwrap_or(serde_json::Value::Null),
            schema,
        }
    }
}

/// One signal as the controller names it.
#[derive(Serialize, serde::Deserialize, Clone, Debug)]
pub struct SignalFact {
    pub index: u8,
    pub name: String,
    /// TCP logger channel the signal streams on, if it is streamed.
    pub tcp_channel: Option<u8>,
}

/// What a tool learned about the controller at startup, for the header.
#[derive(Serialize, Clone, Debug, Default)]
pub struct ControllerFacts {
    /// Signals the run resolved, by name. Not the controller's whole list:
    /// only what the registry holds, which is what the run can refer to.
    pub signals: Vec<SignalFact>,
    /// Delivered stream rate, if a stream was started.
    pub stream_rate_hz: Option<f64>,
}

impl ControllerFacts {
    pub fn gather(controller: &mut dyn SpmController, registry: Option<&SignalRegistry>) -> Self {
        // Aliases share one `Signal`, so the same entry shows up under
        // several names; sort and dedup collapse them.
        let mut signals: Vec<SignalFact> = registry
            .map(|r| {
                r.values()
                    .map(|s| SignalFact {
                        index: s.index,
                        name: s.name.clone(),
                        tcp_channel: s.tcp_channel,
                    })
                    .collect()
            })
            .unwrap_or_default();
        signals.sort_by_key(|s| (s.index, s.name.clone()));
        signals.dedup_by(|a, b| a.index == b.index && a.name == b.name);
        Self {
            signals,
            stream_rate_hz: controller.stream_rate_hz(),
        }
    }
}

/// Reading a log back.
pub mod reader;
