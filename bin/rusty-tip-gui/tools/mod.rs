//! What the workbench hosts.
//!
//! A tool owns its parameters and knows how to draw them, how to turn them
//! into a [`Job`] for the session, and what to show on top of the generic
//! run view while its job runs. The session never sees a tool and a tool
//! never sees a thread.
//!
//! Tip prep draws its setup from its config's JSON Schema
//! ([`crate::form`]); drift draws a small form by hand, since it greys
//! fields out by operation, and the schema form's `x-enabled-by` gates
//! only on a sibling boolean. A tool that needs no connection (the
//! handoff's `plan`) is not catered for yet.

pub mod controllers;
pub mod drift;
pub mod operating_points;
pub mod tip_prep;

use eframe::egui;
use serde::Serialize;
use serde::de::DeserializeOwned;

use rusty_tip::session::{Job, Readout};

use crate::connection::ConnectionSettings;
use crate::run_view::RunView;
use crate::samples::Samples;

/// What the Setup tab knows about the rest of the window.
///
/// Connection settings are the Connection page's. A tool whose config
/// file also carries them (tip prep's does, for the CLI) writes
/// `connection` into the file on save, and hands a file's settings back
/// through `import` on load, for the page to take when disconnected.
///
/// A tab that needs the controller (reading a loop's parameters, say)
/// puts a job in `run`; the window starts it and stays on Setup. `view`
/// is the current or last run, so the tab can pick its results up.
pub struct SetupCx<'a> {
    /// The page as it stands, or what is wrong with it.
    pub connection: Result<ConnectionSettings, String>,
    pub import: Option<ConnectionSettings>,
    /// Connected and idle: a job could start now.
    pub can_run: bool,
    /// A job the tab wants started.
    pub run: Option<Box<dyn Job>>,
    pub view: &'a RunView,
    /// The last seconds of the stream, for a live chart.
    pub samples: &'a Samples,
    /// What the connection knows about its signals, to find one by name.
    /// The session's idle readouts, each with the registry name it was
    /// asked for, so a tool finds a signal the way the session did.
    pub readouts: &'a [Readout],
}

pub trait Tool {
    /// Short identifier, stable across versions: the sidebar key and the
    /// preferences key.
    fn id(&self) -> &'static str;

    /// What the sidebar shows.
    fn label(&self) -> &str;

    /// The Setup tab.
    fn setup(&mut self, ui: &mut egui::Ui, cx: &mut SetupCx<'_>);

    /// Build the job from the current setup, or say what is wrong with it.
    fn job(&self) -> Result<Box<dyn Job>, String>;

    /// The tool's own view of a run, drawn above the generic panels.
    fn panel(&mut self, _ui: &mut egui::Ui, _view: &RunView) {}

    /// Whether the tool has a Run tab. One whose jobs all start from its
    /// Setup tab (reading or writing the controller) has none; the last
    /// job's [`Tool::panel`] shows under its setup instead.
    fn has_run_tab(&self) -> bool {
        true
    }

    /// Controls beside Start and Stop, for acting on the run in flight.
    fn run_controls(&mut self, _ui: &mut egui::Ui, _view: &RunView) {}

    /// Saved with the window's preferences, restored on the next start.
    fn prefs(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    fn restore(&mut self, _prefs: &serde_json::Value) {}
}

/// Read a TOML file into a `T`, the path in the error.
pub fn load_toml<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read {path}: {e}"))?;
    toml::from_str(&text).map_err(|e| e.to_string())
}

/// Write `value` to `path` as TOML, giving the name a `.toml` ending
/// first when it has none.
pub fn save_toml<T: Serialize>(path: &mut String, value: &T) -> Result<(), String> {
    if !path.to_lowercase().ends_with(".toml") {
        path.push_str(".toml");
    }
    let text = toml::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(&*path, text).map_err(|e| format!("Cannot write {path}: {e}"))
}

/// Every tool the workbench ships, in sidebar order.
pub fn all() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(tip_prep::TipPrepTool::default()),
        Box::new(drift::DriftTool::default()),
        Box::new(controllers::ControllersTool::default()),
    ]
}
