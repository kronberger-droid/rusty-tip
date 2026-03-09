use crate::spm_controller::SpmController;
use super::DataStore;

/// Context passed to every action during execution.
///
/// Provides access to the hardware controller and a shared data store
/// for inter-action communication. Future phases will add event emission
/// and cancellation support here.
pub struct ActionContext<'a> {
    /// The hardware controller (or mock/simulation)
    pub controller: &'a mut dyn SpmController,
    /// Shared key-value store for passing data between actions
    pub store: &'a mut DataStore,
}
