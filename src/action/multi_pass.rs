//! Multi-pass scanning actions.
//!
//! The Nanonis controller cannot be handed a multi-pass configuration over
//! TCP; it can only be told to load one from a file it can reach. So the
//! interesting action here is [`ApplyMultiPass`], which writes the file and
//! loads it as one unit. See [`crate::multi_pass`] for the file format.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::multi_pass::MultiPassConfig;
use crate::spm_controller::Capability;
use crate::spm_error::SpmError;

/// Load a `.mpas` file the controller can reach.
///
/// An empty `host_path` loads whatever the session settings file holds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoadMultiPass {
    pub host_path: String,
}

impl Action for LoadMultiPass {
    fn name(&self) -> &str {
        "load_multi_pass"
    }
    fn description(&self) -> &str {
        "Load a multi-pass configuration file on the controller"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::MultiPass]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller.multi_pass_load(&self.host_path)?;
        Ok(ActionOutput::Unit)
    }
}

/// Save the controller's active configuration to a path it can reach.
///
/// Useful as a read-back check: what comes out is what the controller actually
/// parsed. Expect float fields to differ slightly, since the real-time system
/// stores them as `f32`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SaveMultiPass {
    pub host_path: String,
}

impl Action for SaveMultiPass {
    fn name(&self) -> &str {
        "save_multi_pass"
    }
    fn description(&self) -> &str {
        "Save the active multi-pass configuration to a file"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::MultiPass]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller.multi_pass_save(&self.host_path)?;
        Ok(ActionOutput::Unit)
    }
}

/// Switch multi-pass scanning on or off.
///
/// Activating stops a running scan, so this belongs before a scan starts, not
/// during one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivateMultiPass {
    pub on: bool,
}

impl Action for ActivateMultiPass {
    fn name(&self) -> &str {
        "activate_multi_pass"
    }
    fn description(&self) -> &str {
        "Enable or disable multi-pass scanning"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::MultiPass]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller.multi_pass_activate(self.on)?;
        Ok(ActionOutput::Unit)
    }
}

/// Write a configuration, load it on the controller, and switch multi-pass on.
///
/// The two paths are the same file seen from two machines: `local_path` is
/// where we write it, `host_path` is where the controller looks for it. They
/// are the same string only when the controller runs on this machine; under
/// Wine, or on a real instrument, `host_path` is a mapped drive or a share.
///
/// Any signal the configuration records is also added to the scan buffer.
/// Multi-pass records and plays through buffers of its own, which say nothing
/// about what reaches the saved frames, so without this the run produces a
/// `[P2]` pass whose signal was never acquired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyMultiPass {
    pub config: MultiPassConfig,
    pub local_path: PathBuf,
    pub host_path: String,
}

impl Action for ApplyMultiPass {
    fn name(&self) -> &str {
        "apply_multi_pass"
    }
    fn description(&self) -> &str {
        "Write a multi-pass configuration, load it, and activate multi-pass"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::MultiPass, Capability::Scanning]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        self.config
            .write(&self.local_path)
            .map_err(|e| SpmError::Io {
                source: e,
                context: format!("writing multi-pass config to {}", self.local_path.display()),
            })?;

        let recorded: Vec<_> = self
            .config
            .passes
            .iter()
            .filter_map(|p| p.recorded())
            .collect();
        ctx.controller.scan_buffer_ensure(&recorded)?;

        ctx.controller.multi_pass_load(&self.host_path)?;
        ctx.controller.multi_pass_activate(true)?;
        Ok(ActionOutput::Unit)
    }
}
