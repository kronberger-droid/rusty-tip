//! What the workbench hosts.
//!
//! A tool owns its parameters and knows how to draw them, how to turn them
//! into a [`Job`] for the session, and what to show on top of the generic
//! run view while its job runs. The session never sees a tool and a tool
//! never sees a thread.
//!
//! This is the step-2 shape: `setup` is a hand-drawn form. Step 3 of the
//! handoff replaces it with a schema-driven form over the tool's parameter
//! type, at which point `setup` goes and `schema`/`defaults` arrive.

pub mod drift;
pub mod tip_prep;

use eframe::egui;
use rusty_tip::session::Job;

use crate::connection::ConnectionSettings;
use crate::run_view::RunView;

/// What the Setup tab knows about the rest of the window.
///
/// Connection settings are the Connection page's. A tool whose config
/// file also carries them (tip prep's does, for the CLI) writes
/// `connection` into the file on save, and hands a file's settings back
/// through `import` on load, for the page to take when disconnected.
pub struct SetupCx {
    pub connection: ConnectionSettings,
    pub import: Option<ConnectionSettings>,
}

pub trait Tool {
    /// Short identifier, stable across versions: the sidebar key and the
    /// preferences key.
    fn id(&self) -> &'static str;

    /// What the sidebar shows.
    fn label(&self) -> &str;

    /// Whether the job needs the session's controller. A tool that says no
    /// runs on a worker thread of its own, so it works while disconnected
    /// and while another job holds the session.
    fn needs_connection(&self) -> bool {
        true
    }

    /// The Setup tab.
    fn setup(&mut self, ui: &mut egui::Ui, cx: &mut SetupCx);

    /// Build the job from the current setup, or say what is wrong with it.
    fn job(&self) -> Result<Box<dyn Job>, String>;

    /// The tool's own view of a run, drawn above the generic panels.
    fn panel(&mut self, _ui: &mut egui::Ui, _view: &RunView) {}

    /// Saved with the window's preferences, restored on the next start.
    fn prefs(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    fn restore(&mut self, _prefs: &serde_json::Value) {}
}

/// Every tool the workbench ships, in sidebar order.
pub fn all() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(tip_prep::TipPrepTool::default()),
        Box::new(drift::DriftTool::default()),
    ]
}
