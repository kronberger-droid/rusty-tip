use std::time::Duration;

use super::DataStore;
use crate::event::EventEmitter;
use crate::shutdown::ShutdownFlag;
use crate::spm_controller::SpmController;
use crate::spm_error::SpmError;

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
    /// Stop request, so an action that waits can be interrupted.
    pub shutdown: &'a ShutdownFlag,
}

impl ActionContext<'_> {
    /// Wait for `ms` milliseconds, waking early on a shutdown request.
    ///
    /// Actions must wait through this rather than `std::thread::sleep`: a bare
    /// sleep cannot be interrupted, so a stop request goes unheard until the
    /// wait ends. That matters most for the long waits, which are exactly the
    /// ones a user is most likely to want to abort.
    pub fn settle(&self, ms: u64) -> Result<(), SpmError> {
        match self.shutdown.wait_timeout(Duration::from_millis(ms)) {
            true => Err(SpmError::ShutdownRequested),
            false => Ok(()),
        }
    }

    /// Bail out with `ShutdownRequested` if a stop was requested.
    pub fn check_shutdown(&self) -> Result<(), SpmError> {
        match self.shutdown.is_requested() {
            true => Err(SpmError::ShutdownRequested),
            false => Ok(()),
        }
    }
}
