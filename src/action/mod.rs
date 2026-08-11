pub mod bias;
mod context;
pub mod data_stream;
pub mod motor;
pub mod oscilloscope;
mod output;
pub mod pll;
pub mod position;
pub mod scan;
pub mod signals;
mod store;
pub mod tip_shaper;
pub mod util;
pub mod z_controller;

pub use context::ActionContext;
pub use output::ActionOutput;
pub use store::DataStore;

use crate::spm_controller::{Capability, SpmController};
use crate::spm_error::SpmError;

/// Shared serde default for boolean fields that should default to `true`.
pub(crate) fn default_true() -> bool {
    true
}

type Result<T> = std::result::Result<T, SpmError>;

/// Every SPM operation implements this trait.
///
/// Actions are self-contained units of work that execute against an
/// `SpmController` via the `ActionContext`. Each action struct holds its own
/// parameters and knows how to execute itself.
///
/// Actions are an implementation detail of the routine harness, not the
/// authoring API: routines reach them through the subsystem handles
/// [`Rt`](crate::routine::Rt) hands out, which is where capability checks and
/// event emission happen.
pub trait Action: Send + Sync {
    /// Unique identifier, e.g. "read_signal", "bias_pulse"
    fn name(&self) -> &str;

    /// Human-readable description for documentation and LLM context
    fn description(&self) -> &str;

    /// Which hardware capabilities this action needs.
    /// The execution layer checks these against `SpmController::capabilities()`
    /// before running the action. Returns empty by default (no requirements).
    fn requires(&self) -> Vec<Capability> {
        vec![]
    }

    /// Execute this action against the provided context
    fn execute(&self, ctx: &mut ActionContext) -> Result<ActionOutput>;
}

/// Verify the controller supports every capability `action` requires.
///
/// Every execution path calls this before running an action, so a
/// capability violation surfaces as a clear `Unsupported` error before
/// any command reaches the hardware, instead of a mid-action failure.
pub fn check_capabilities(action: &dyn Action, controller: &dyn SpmController) -> Result<()> {
    let required = action.requires();
    if required.is_empty() {
        return Ok(());
    }
    let caps = controller.capabilities();
    for cap in &required {
        if !caps.contains(cap) {
            return Err(SpmError::Unsupported(format!(
                "Action '{}' requires {:?}, which the controller does not support",
                action.name(),
                cap,
            )));
        }
    }
    Ok(())
}
