use std::time::{Duration, Instant};

use crate::action::{Action, ActionContext, DataStore};
use crate::event::{Event, EventBus, EventEmitter};
use crate::shutdown::ShutdownFlag;
use crate::spm_controller::{Capability, SpmController, ZControllerStatus};
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
    safe_tip_guard: bool,
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
            safe_tip_guard: false,
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

    /// Arm or disarm the safe-tip guard for subsequent approaches.
    ///
    /// Off unless a routine asks for it, either for its whole run via
    /// [`Routine::safe_tip_guard`](super::Routine::safe_tip_guard) or around a
    /// section with this setter. The guard is scoped to the calibrated
    /// approach rather than to every action, since that is the only stretch
    /// where the tip sits off the surface out of feedback and can drift into
    /// it. Everywhere else a routine runs in feedback, so the same check
    /// would mostly report misfires.
    pub fn set_safe_tip_guard(&mut self, enabled: bool) {
        self.safe_tip_guard = enabled;
    }

    /// Whether the safe-tip guard is currently armed.
    pub fn safe_tip_guard(&self) -> bool {
        self.safe_tip_guard
    }

    /// Bail out if the controller's safe-tip protection has tripped.
    ///
    /// The calibrated approach performs this check itself while the guard is
    /// armed; call it directly only to check at a point of your own choosing.
    ///
    /// A controller without [`Capability::SafeTip`] always passes, and a
    /// status read that itself fails is logged rather than treated as a trip,
    /// so a flaky read cannot abort an otherwise healthy run.
    pub fn check_safe_tip(&mut self) -> Result<(), SpmError> {
        if !self
            .controller
            .capabilities()
            .contains(&Capability::SafeTip)
        {
            return Ok(());
        }
        match self.controller.z_controller_status() {
            Ok(ZControllerStatus::SafeTip) => Err(SpmError::Routine(
                "safe-tip protection triggered, aborting".into(),
            )),
            Ok(_) => Ok(()),
            Err(e) => {
                log::warn!(
                    "Could not read z-controller status for safe-tip check: {}",
                    e
                );
                Ok(())
            }
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
    /// the body's error wins and a subsequent cleanup failure is logged and
    /// emitted as a `cleanup_failed` event (so a swallowed failure still
    /// reaches the event log); if the body succeeded, a cleanup failure is
    /// propagated. Cleanup that should be best-effort only (log, don't fail
    /// the run) handles its own errors and returns `Ok(())`.
    ///
    /// `cleanup` runs when `body` returns, not when it unwinds: a panic
    /// inside `body` skips it. [`run_routine`] catches panics at the top
    /// level so the tip is still withdrawn.
    ///
    /// [`run_routine`]: super::run_routine
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
                self.events.emit(Event::custom(
                    "cleanup_failed",
                    serde_json::json!({
                        "cleanup_error": cleanup_err.to_string(),
                        "body_error": body_err.to_string(),
                    }),
                ));
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

    /// Emit the same started/completed/failed triple as [`exec`] around a
    /// direct controller call, for mutations that have no [`Action`] behind
    /// them.
    ///
    /// Handle methods that *change* instrument state go through this or
    /// [`exec`], so every state change is in the event log; handle methods
    /// that only observe (`scan().status()` in a poll loop) call the
    /// controller directly and stay silent. Reads whose value is scientific
    /// data (`signals().read()`) are measurements, not observations, and go
    /// through [`exec`].
    ///
    /// [`exec`]: Self::exec
    pub(crate) fn logged<T>(
        &mut self,
        name: &str,
        params: serde_json::Value,
        op: impl FnOnce(&mut dyn SpmController) -> Result<T, SpmError>,
    ) -> Result<T, SpmError> {
        let start = Instant::now();
        self.events.emit(Event::action_started(name, params));
        match op(&mut *self.controller) {
            Ok(value) => {
                self.events.emit(Event::action_completed(
                    name,
                    serde_json::Value::Null,
                    start.elapsed(),
                ));
                Ok(value)
            }
            Err(e) => {
                self.events
                    .emit(Event::action_failed(name, &e.to_string(), start.elapsed()));
                Err(e)
            }
        }
    }

    /// Execute an action with capability checking and start/complete/fail
    /// events. All subsystem handle methods funnel through here.
    ///
    /// Returns the action's own output type, so a handle method that reads a
    /// voltage gets an `f64` rather than something it has to unwrap.
    ///
    /// The `Serialize` bound is what puts the action's parameters into the
    /// started event, so the log records a pulse's voltage and width rather
    /// than just that a pulse happened. An action that cannot be serialized
    /// (one holding a trait object, say) stays a valid [`Action`] but cannot
    /// go through here until it grows a `Serialize` impl.
    pub(crate) fn exec<A: Action + serde::Serialize>(
        &mut self,
        action: &A,
    ) -> Result<A::Output, SpmError> {
        let name = action.name().to_string();
        let start = Instant::now();
        let params = serde_json::to_value(action).unwrap_or_default();
        self.events.emit(Event::action_started(&name, params));
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
                // A value that will not serialize must not fail the action
                // that already ran; log it as null instead.
                let json = serde_json::to_value(&output).unwrap_or(serde_json::Value::Null);
                self.events
                    .emit(Event::action_completed(&name, json, start.elapsed()));
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
