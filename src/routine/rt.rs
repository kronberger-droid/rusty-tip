use std::time::{Duration, Instant};

use crate::action::{Action, ActionContext, ActionOutput, DataStore};
use crate::event::{Event, EventBus, EventEmitter};
use crate::shutdown::ShutdownFlag;
use crate::spm_controller::{Capability, SpmController};
use crate::spm_error::SpmError;

use super::Outcome;
use super::subsystems::{Bias, Motor, Scan, Signals, ZCtrl};

/// The routine runtime: what a [`super::Routine`] runs against.
///
/// Hands out capability-checked subsystem handles and provides the
/// cross-cutting pieces (interruptible waits, cycle budgets, guaranteed
/// cleanup, event emission). Subsystem handles are cheap and short-lived by
/// design: fetch one per statement (`rt.bias()?.set(v)?`) rather than
/// binding it across other `rt` calls, so borrows never overlap.
pub struct Rt<'a> {
    controller: &'a mut dyn SpmController,
    events: &'a EventBus,
    shutdown: &'a ShutdownFlag,
    store: DataStore,
}

impl<'a> Rt<'a> {
    pub fn new(
        controller: &'a mut dyn SpmController,
        events: &'a EventBus,
        shutdown: &'a ShutdownFlag,
    ) -> Self {
        Self {
            controller,
            events,
            shutdown,
            store: DataStore::new(),
        }
    }

    // -- Subsystems --

    /// Bias voltage control. Errors if the controller lacks [`Capability::Bias`].
    pub fn bias(&mut self) -> Result<Bias<'_, 'a>, SpmError> {
        self.require(Capability::Bias)?;
        Ok(Bias { rt: self })
    }

    /// Z-controller operations. Errors if the controller lacks [`Capability::ZController`].
    pub fn z(&mut self) -> Result<ZCtrl<'_, 'a>, SpmError> {
        self.require(Capability::ZController)?;
        Ok(ZCtrl { rt: self })
    }

    /// Signal reading. Errors if the controller lacks [`Capability::Signals`].
    pub fn signals(&mut self) -> Result<Signals<'_, 'a>, SpmError> {
        self.require(Capability::Signals)?;
        Ok(Signals { rt: self })
    }

    /// Coarse motor positioning. Errors if the controller lacks [`Capability::Motor`].
    pub fn motor(&mut self) -> Result<Motor<'_, 'a>, SpmError> {
        self.require(Capability::Motor)?;
        Ok(Motor { rt: self })
    }

    /// Scan control. Errors if the controller lacks [`Capability::Scanning`].
    pub fn scan(&mut self) -> Result<Scan<'_, 'a>, SpmError> {
        self.require(Capability::Scanning)?;
        Ok(Scan { rt: self })
    }

    /// Escape hatch: the bare controller, for operations the subsystem
    /// handles don't cover. Calls made through this bypass event logging.
    pub fn controller(&mut self) -> &mut dyn SpmController {
        self.controller
    }

    // -- Cross-cutting --

    /// Wait for `ms` milliseconds, waking early on a shutdown request
    /// (which surfaces as `Err(SpmError::ShutdownRequested)`).
    pub fn settle(&self, ms: u64) -> Result<(), SpmError> {
        if self.shutdown.wait_timeout(Duration::from_millis(ms)) {
            Err(SpmError::ShutdownRequested)
        } else {
            Ok(())
        }
    }

    /// Bail out with `Err(SpmError::ShutdownRequested)` if a stop was requested.
    pub fn check_shutdown(&self) -> Result<(), SpmError> {
        if self.shutdown.is_requested() {
            Err(SpmError::ShutdownRequested)
        } else {
            Ok(())
        }
    }

    /// The shutdown flag itself, e.g. for handing to a spawned thread.
    pub fn shutdown(&self) -> &ShutdownFlag {
        self.shutdown
    }

    /// Emit an event to all observers (GUI, JSONL log, console).
    pub fn emit(&self, event: Event) {
        self.events.emit(event);
    }

    /// Start a budgeted main loop. `None` means unlimited.
    ///
    /// The returned [`Cycles`] is independent of `rt` (it snapshots the
    /// shutdown flag), so the loop body can use `rt` freely:
    ///
    /// ```ignore
    /// let mut cycles = rt.cycles(cfg.max_cycles, cfg.max_duration);
    /// while let Some(cycle) = cycles.next() {
    ///     // ... may `return Ok(Outcome::Completed)` early ...
    /// }
    /// Ok(cycles.outcome())
    /// ```
    pub fn cycles(&self, max_cycles: Option<usize>, max_duration: Option<Duration>) -> Cycles {
        Cycles {
            shutdown: self.shutdown.clone(),
            started: Instant::now(),
            max_cycles,
            max_duration,
            completed: 0,
            ending: None,
        }
    }

    /// Run `body`, then always run `cleanup`, whichever way `body` ended.
    ///
    /// Use this wherever hardware must be restored (stop a scan, restore a
    /// speed, withdraw) even if the work in between fails. Error precedence:
    /// the body's error wins and a subsequent cleanup failure is logged; if
    /// the body succeeded, a cleanup failure is propagated. Cleanup that
    /// should be best-effort only (log, don't fail the run) handles its own
    /// errors and returns `Ok(())`.
    pub fn guarded<T>(
        &mut self,
        body: impl FnOnce(&mut Rt<'a>) -> Result<T, SpmError>,
        cleanup: impl FnOnce(&mut Rt<'a>) -> Result<(), SpmError>,
    ) -> Result<T, SpmError> {
        let result = body(self);
        match (result, cleanup(self)) {
            (Ok(value), Ok(())) => Ok(value),
            (Ok(_), Err(cleanup_err)) => Err(cleanup_err),
            (Err(body_err), Ok(())) => Err(body_err),
            (Err(body_err), Err(cleanup_err)) => {
                log::error!("Cleanup after a failure also failed: {}", cleanup_err);
                Err(body_err)
            }
        }
    }

    // -- Internal --

    fn require(&self, cap: Capability) -> Result<(), SpmError> {
        if self.controller.capabilities().contains(&cap) {
            Ok(())
        } else {
            Err(SpmError::Unsupported(format!(
                "controller does not support the {:?} subsystem",
                cap
            )))
        }
    }

    /// Execute an action with capability checking and start/complete/fail
    /// events. All subsystem handle methods funnel through here.
    pub(crate) fn exec(&mut self, action: &dyn Action) -> Result<ActionOutput, SpmError> {
        let name = action.name().to_string();
        let start = Instant::now();
        self.events
            .emit(Event::action_started(&name, serde_json::json!({})));
        let mut ctx = ActionContext {
            controller: self.controller,
            store: &mut self.store,
            events: self.events,
        };
        let result = match crate::action::check_capabilities(action, ctx.controller) {
            Ok(()) => action.execute(&mut ctx),
            Err(e) => Err(e),
        };
        match result {
            Ok(output) => {
                self.events
                    .emit(Event::action_completed(&name, &output, start.elapsed()));
                Ok(output)
            }
            Err(e) => {
                self.events
                    .emit(Event::action_failed(&name, &e.to_string(), start.elapsed()));
                Err(e)
            }
        }
    }
}

/// Budget-aware driver for a routine's main loop, created by [`Rt::cycles`].
///
/// `next()` returns the 1-based cycle number until a stop condition is hit
/// (shutdown request, time budget, cycle budget), after which [`outcome`]
/// says which one it was.
///
/// [`outcome`]: Cycles::outcome
pub struct Cycles {
    shutdown: ShutdownFlag,
    started: Instant,
    max_cycles: Option<usize>,
    max_duration: Option<Duration>,
    completed: usize,
    ending: Option<Outcome>,
}

impl Cycles {
    /// The next cycle number, or `None` once a stop condition is reached.
    #[allow(clippy::should_implement_trait)] // deliberate: not an Iterator, so
    // `for` can't consume it and `outcome()` stays reachable after the loop
    pub fn next(&mut self) -> Option<usize> {
        if self.ending.is_some() {
            return None;
        }
        if self.shutdown.is_requested() {
            self.ending = Some(Outcome::StoppedByUser);
            return None;
        }
        if let Some(max) = self.max_duration
            && self.started.elapsed() > max
        {
            self.ending = Some(Outcome::TimedOut(max));
            return None;
        }
        if let Some(max) = self.max_cycles
            && self.completed >= max
        {
            self.ending = Some(Outcome::CycleLimit(max));
            return None;
        }
        self.completed += 1;
        Some(self.completed)
    }

    /// Time since the loop started.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Why the loop stopped. Call after `next()` has returned `None`.
    ///
    /// # Panics
    ///
    /// Panics if the loop has not ended yet (i.e. `next()` never returned
    /// `None`), since there is no outcome to report at that point.
    pub fn outcome(&self) -> Outcome {
        self.ending
            .expect("Cycles::outcome() called before next() returned None")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_counts_up_to_the_limit() {
        let shutdown = ShutdownFlag::new();
        let mut cycles = Cycles {
            shutdown,
            started: Instant::now(),
            max_cycles: Some(3),
            max_duration: None,
            completed: 0,
            ending: None,
        };
        let mut seen = Vec::new();
        while let Some(c) = cycles.next() {
            seen.push(c);
        }
        assert_eq!(seen, vec![1, 2, 3]);
        assert_eq!(cycles.outcome(), Outcome::CycleLimit(3));
    }

    #[test]
    fn cycles_stops_on_shutdown() {
        let shutdown = ShutdownFlag::new();
        let mut cycles = Cycles {
            shutdown: shutdown.clone(),
            started: Instant::now(),
            max_cycles: None,
            max_duration: None,
            completed: 0,
            ending: None,
        };
        assert_eq!(cycles.next(), Some(1));
        shutdown.request();
        assert_eq!(cycles.next(), None);
        assert_eq!(cycles.outcome(), Outcome::StoppedByUser);
    }

    #[test]
    fn cycles_times_out() {
        let shutdown = ShutdownFlag::new();
        let mut cycles = Cycles {
            shutdown,
            started: Instant::now() - Duration::from_secs(10),
            max_cycles: None,
            max_duration: Some(Duration::from_secs(1)),
            completed: 0,
            ending: None,
        };
        assert_eq!(cycles.next(), None);
        assert_eq!(cycles.outcome(), Outcome::TimedOut(Duration::from_secs(1)));
    }

    #[test]
    #[should_panic(expected = "before next() returned None")]
    fn outcome_before_the_loop_ends_panics() {
        let shutdown = ShutdownFlag::new();
        let cycles = Cycles {
            shutdown,
            started: Instant::now(),
            max_cycles: Some(3),
            max_duration: None,
            completed: 0,
            ending: None,
        };
        let _ = cycles.outcome();
    }
}
