use super::DataStore;
use crate::event::EventEmitter;
use crate::spm_controller::SpmController;

/// Context passed to every action during execution.
///
/// Provides access to the hardware controller, a shared data store
/// for inter-action communication, and an event emitter for observability.
pub struct ActionContext<'a> {
    /// The hardware controller (or mock/simulation)
    pub controller: &'a mut dyn SpmController,
    /// Shared key-value store for passing data between actions
    pub store: &'a mut DataStore,
    /// Event emitter for observability (logging, GUI updates, LLM context)
    pub events: &'a dyn EventEmitter,
}
