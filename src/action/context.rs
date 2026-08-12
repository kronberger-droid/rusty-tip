use crate::event::EventEmitter;
use crate::spm_controller::SpmController;

/// Context passed to every action during execution.
///
/// Provides access to the hardware controller and an event emitter for
/// observability. Actions return their results as typed values
/// ([`Action::Output`](super::Action::Output)); data flows between steps
/// through the routine that runs them, not through shared state.
pub struct ActionContext<'a> {
    /// The hardware controller (or mock/simulation)
    pub controller: &'a mut dyn SpmController,
    /// Event emitter for observability (logging, GUI updates, LLM context)
    pub events: &'a dyn EventEmitter,
}
