use super::{Action, ActionOutput, DataStore, run_action};
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
    /// Nesting of the action running in this context: 0 for one a routine
    /// ran directly, 1 for one run through [`ActionContext::run`] by that
    /// action, and so on. Stamped on the events the action emits.
    pub depth: usize,
}

impl ActionContext<'_> {
    /// Run `action` as a child of the current one.
    ///
    /// Composite actions call their steps through this rather than calling
    /// `execute` directly, so each step is logged as its own action one
    /// level deeper and a log reader can rebuild the tree.
    pub fn run<A: Action + serde::Serialize>(
        &mut self,
        action: &A,
    ) -> Result<ActionOutput, SpmError> {
        let mut child = ActionContext {
            controller: &mut *self.controller,
            store: &mut *self.store,
            events: self.events,
            shutdown: self.shutdown,
            depth: self.depth + 1,
        };
        run_action(&mut child, action)
    }

    /// Wait for `ms` milliseconds, waking early on a shutdown request.
    ///
    /// Actions must wait through this rather than `std::thread::sleep`: a bare
    /// sleep cannot be interrupted, so a stop request goes unheard until the
    /// wait ends. That matters most for the long waits, which are exactly the
    /// ones a user is most likely to want to abort.
    pub fn settle(&self, ms: u64) -> Result<(), SpmError> {
        self.shutdown.settle(ms)
    }

    /// Bail out with `ShutdownRequested` if a stop was requested.
    pub fn check_shutdown(&self) -> Result<(), SpmError> {
        self.shutdown.check()
    }
}
