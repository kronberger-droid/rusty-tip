pub mod bias;
mod context;
pub mod data_stream;
pub mod motor;
pub mod oscilloscope;
pub mod pll;
pub mod position;
pub mod scan;
pub mod signals;
mod store;
pub mod tip_shaper;
pub mod util;
pub mod z_controller;

pub use context::ActionContext;
pub use store::DataStore;

use serde::Serialize;

use crate::spm_controller::{Capability, SpmController};
use crate::spm_error::SpmError;

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
    /// What a successful execution produces: `()` for operations that only
    /// command the hardware, `f64` for a single reading, a struct or
    /// `serde_json::Value` for richer results.
    ///
    /// The bound exists so the execution layer can put the value into the
    /// event log without knowing the type.
    type Output: Serialize;

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
    fn execute(&self, ctx: &mut ActionContext) -> Result<Self::Output>;
}

/// Verify the controller supports every capability `action` requires.
///
/// Every execution path calls this before running an action, so a
/// capability violation surfaces as a clear `Unsupported` error before
/// any command reaches the hardware, instead of a mid-action failure.
pub fn check_capabilities<A: Action + ?Sized>(
    action: &A,
    controller: &dyn SpmController,
) -> Result<()> {
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
