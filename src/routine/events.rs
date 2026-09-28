//! Events the harness writes on behalf of whichever routine it runs.

use schemars::JsonSchema;
use serde::Serialize;

use crate::experiment_log::{LogEvent, ToolSchema};
use crate::spm_controller::StreamSnapshot;

/// A `guarded` body failed and so did its cleanup. The body's error is what
/// the run reports; this is the only record of the cleanup's.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct CleanupFailedEvent {
    pub body_error: String,
    pub cleanup_error: String,
}

impl LogEvent for CleanupFailedEvent {
    const KIND: &'static str = "routine/cleanup_failed";
}

/// The routine panicked. The tip was withdrawn and the controller torn down
/// before the panic was re-raised.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct PanickedEvent {
    pub routine: String,
    pub message: String,
}

impl LogEvent for PanickedEvent {
    const KIND: &'static str = "routine/panicked";
}

/// The data stream's buffer at the end of the run, written after the final
/// withdraw and before teardown stops the stream. The default buffer holds
/// 10 000 frames: the last 20 s at 500 Hz, 10 s at 1 kHz, either way enough
/// to cover whatever ended the run.
#[derive(Serialize, serde::Deserialize, JsonSchema, Clone, Debug)]
pub struct StreamDumpEvent {
    pub stream: StreamSnapshot,
}

impl LogEvent for StreamDumpEvent {
    const KIND: &'static str = "routine/stream_dump";
}

/// A settings file was loaded into the controller during the run. The load
/// outlives the run: whatever it set stays set for the rest of the session.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct SettingsLoadedEvent {
    /// The path as the routine gave it.
    pub path: String,
}

impl LogEvent for SettingsLoadedEvent {
    const KIND: &'static str = "routine/settings_loaded";
}

/// A layout file was loaded into the controller during the run.
#[derive(Serialize, JsonSchema, Clone, Debug)]
pub struct LayoutLoadedEvent {
    /// The path as the routine gave it.
    pub path: String,
}

impl LogEvent for LayoutLoadedEvent {
    const KIND: &'static str = "routine/layout_loaded";
}

/// The kinds any routine's log can contain. A tool includes this in its own
/// schema with [`ToolSchema::including`].
pub fn log_schema() -> ToolSchema {
    ToolSchema::new("routine")
        .with::<CleanupFailedEvent>()
        .with::<PanickedEvent>()
        .with::<StreamDumpEvent>()
        .with::<SettingsLoadedEvent>()
        .with::<LayoutLoadedEvent>()
}
