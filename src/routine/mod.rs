//! The routine harness: the interface automation routines are written against.
//!
//! A routine is a plain Rust struct implementing [`Routine`]. Its `run`
//! method receives an [`Rt`] (the routine runtime), which hands out the
//! controller's subsystems and absorbs the cross-cutting scaffolding every
//! routine otherwise reimplements:
//!
//! - **Subsystem access**: `rt.bias()?`, `rt.z()?`, `rt.signals()?`,
//!   `rt.motor()?`, `rt.scan()?`. Each accessor checks the controller's
//!   capabilities, so running a routine against hardware that lacks a
//!   subsystem fails with a clear `Unsupported` error at the call site.
//!   Every operation is logged to the [`EventBus`] automatically.
//! - **Cancellation**: `rt.settle(ms)` sleeps interruptibly and
//!   `rt.check_shutdown()?` bails between steps; both surface a stop request
//!   as [`SpmError::ShutdownRequested`], which the harness translates to
//!   [`Outcome::StoppedByUser`].
//! - **Budgets**: [`Rt::cycles`] drives the main loop and turns cycle and
//!   time limits into [`Outcome`]s instead of hand-rolled checks.
//! - **Cleanup**: [`Rt::guarded`] runs a body with a cleanup that executes
//!   no matter how the body ends, for hardware that must be restored
//!   (a running scan, a modified scan speed) even when a sweep fails.
//!
//! [`run_routine`] owns the controller life cycle around a routine: it calls
//! `prepare()`, runs the routine, withdraws the tip, and calls `teardown()`,
//! whatever the outcome. The shipped tip-prep routine
//! ([`crate::tip_prep::TipPrep`]) is the reference implementation.
//!
//! ```no_run
//! use rusty_tip::event::EventBus;
//! use rusty_tip::routine::{Outcome, Routine, Rt, run_routine};
//! use rusty_tip::spm_error::SpmError;
//! use rusty_tip::ShutdownFlag;
//!
//! struct BiasCheck {
//!     target_v: f64,
//! }
//!
//! impl Routine for BiasCheck {
//!     fn name(&self) -> &str {
//!         "bias_check"
//!     }
//!
//!     fn run(&mut self, rt: &mut Rt) -> Result<Outcome, SpmError> {
//!         rt.bias()?.set(self.target_v)?;
//!         rt.settle(500)?;
//!         let read_back = rt.bias()?.get()?;
//!         log::info!("bias now {read_back} V");
//!         Ok(Outcome::Completed)
//!     }
//! }
//!
//! # fn main() -> Result<(), SpmError> {
//! # let controller: Box<dyn rusty_tip::spm_controller::SpmController> = unimplemented!();
//! let events = EventBus::new();
//! let shutdown = ShutdownFlag::new();
//! let outcome = run_routine(controller, &events, &shutdown, &mut BiasCheck { target_v: -0.5 })?;
//! # Ok(())
//! # }
//! ```

mod rt;
mod subsystems;

pub use rt::{Cycles, Rt};
pub use subsystems::{Bias, Motor, RepositionSpec, Scan, Signals, StableReadSpec, ZCtrl};

use std::panic::{self, AssertUnwindSafe};
use std::time::Duration;

use crate::event::{Event, EventBus, EventEmitter};
use crate::shutdown::ShutdownFlag;
use crate::spm_controller::SpmController;
use crate::spm_error::SpmError;

/// How a routine run ended.
///
/// Everything here is an expected ending, not an error: a stop request or an
/// exhausted budget is a normal way for lab automation to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The routine reached its goal.
    Completed,
    /// A shutdown was requested (Ctrl+C, GUI stop button).
    StoppedByUser,
    /// The cycle budget ran out first.
    CycleLimit(usize),
    /// The time budget ran out first.
    TimedOut(Duration),
}

/// An automation routine, runnable via [`run_routine`].
///
/// Implementations hold their own configuration and mutable state; all
/// hardware access goes through the [`Rt`] passed to `run`.
pub trait Routine {
    /// Short identifier used in logs and events.
    fn name(&self) -> &str;

    /// Whether calibrated approaches abort when safe-tip protection trips,
    /// for the whole run. Off by default.
    ///
    /// Turn it on for a routine whose approaches are the risky part, so a
    /// trip aborts instead of the run continuing over a crashed tip. Within
    /// `run`, [`Rt::set_safe_tip_guard`] overrides this for a section.
    fn safe_tip_guard(&self) -> bool {
        false
    }

    /// Execute the routine to one of its endings.
    ///
    /// Return `Err(SpmError::ShutdownRequested)` freely from anywhere inside
    /// (it is what `rt.settle` and `rt.check_shutdown` produce); the harness
    /// converts it to `Ok(Outcome::StoppedByUser)`.
    fn run(&mut self, rt: &mut Rt) -> Result<Outcome, SpmError>;
}

/// Run a routine, owning the controller life cycle around it.
///
/// Calls `prepare()` first. Afterwards, regardless of how the routine ended,
/// withdraws the tip (best effort, logged on failure) and calls `teardown()`,
/// so an error mid-routine never leaves the tip engaged on the surface. A
/// shutdown request surfaces as `Ok(Outcome::StoppedByUser)`, never as an
/// error.
///
/// "Regardless" includes panics: the routine runs inside
/// [`catch_unwind`](panic::catch_unwind), so a panicking routine is withdrawn
/// and torn down before the panic is re-raised unchanged. This relies on the
/// unwinding panic strategy; under `panic = "abort"` no cleanup can run.
///
/// # Limitation: one routine per controller
///
/// This takes the controller by value and drops it, so routines cannot yet be
/// composed — you cannot prepare a tip with one routine and measure with the
/// next against the same connection. Sequencing work today means writing it as
/// a single `Routine`. Lifting this needs a borrowing signature and a way to
/// opt out of the unconditional withdraw (re-approaching between stages loses
/// the spot and costs an approach cycle); both are deferred until a second
/// routine exists to design them against.
pub fn run_routine(
    mut controller: Box<dyn SpmController>,
    events: &EventBus,
    shutdown: &ShutdownFlag,
    routine: &mut dyn Routine,
) -> Result<Outcome, SpmError> {
    controller.prepare()?;

    let mut rt = Rt::new(&mut *controller, events, shutdown);
    rt.set_safe_tip_guard(routine.safe_tip_guard());
    // AssertUnwindSafe is honest here: nothing observes `rt` or `routine`
    // after a panic except the cleanup below, which only restores hardware
    // before re-raising.
    let caught = panic::catch_unwind(AssertUnwindSafe(|| routine.run(&mut rt)));

    if let Err(payload) = &caught {
        let message = panic_message(&**payload);
        log::error!("Routine '{}' panicked: {}", routine.name(), message);
        events.emit(Event::custom(
            "routine_panicked",
            serde_json::json!({ "routine": routine.name(), "message": message }),
        ));
    }

    log::info!("Cleanup starting...");
    match rt.z() {
        Ok(mut z) => {
            if let Err(e) = z.withdraw() {
                log::warn!("Cleanup withdrawal failed: {}", e);
            }
        }
        Err(e) => log::warn!("Cleanup withdrawal skipped: {}", e),
    }
    drop(rt);
    controller.teardown();
    log::info!("Cleanup complete");

    match caught {
        Ok(Err(SpmError::ShutdownRequested)) => Ok(Outcome::StoppedByUser),
        Ok(other) => other,
        // Hardware is restored; hand the panic back to the caller untouched.
        Err(payload) => panic::resume_unwind(payload),
    }
}

/// Best-effort rendering of a caught panic payload, which is a `&str` for
/// `panic!("literal")` and a `String` for formatted messages.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use super::*;
    use crate::event::Observer;
    use crate::mock_controller::MockController;
    use crate::spm_controller::ZControllerStatus;

    #[derive(Clone, Default)]
    struct Recorder {
        events: Arc<StdMutex<Vec<Event>>>,
    }

    impl Observer for Recorder {
        fn on_event(&self, event: &Event) {
            self.events.lock().unwrap().push(event.clone());
        }
    }

    fn recording_bus() -> (EventBus, Arc<StdMutex<Vec<Event>>>) {
        let recorder = Recorder::default();
        let handle = Arc::clone(&recorder.events);
        let mut bus = EventBus::new();
        bus.add_observer(Box::new(recorder));
        (bus, handle)
    }

    fn custom_event(events: &[Event], wanted: &str) -> Option<serde_json::Value> {
        events.iter().find_map(|e| match e {
            Event::Custom { kind, data } if kind == wanted => Some(data.clone()),
            _ => None,
        })
    }

    fn started_params(events: &[Event], wanted: &str) -> Option<serde_json::Value> {
        events.iter().find_map(|e| match e {
            Event::ActionStarted { action, params, .. } if action == wanted => Some(params.clone()),
            _ => None,
        })
    }

    struct Panicker;

    impl Routine for Panicker {
        fn name(&self) -> &str {
            "panicker"
        }

        fn run(&mut self, rt: &mut Rt) -> Result<Outcome, SpmError> {
            rt.bias()?.set(-1.0)?;
            panic!("routine blew up");
        }
    }

    #[test]
    fn a_panicking_routine_is_still_withdrawn_and_the_panic_re_raised() {
        let mock = MockController::builder().build();
        let obs = mock.observations();
        let (bus, events) = recording_bus();

        let caught = panic::catch_unwind(AssertUnwindSafe(|| {
            run_routine(Box::new(mock), &bus, &ShutdownFlag::new(), &mut Panicker)
        }));

        let payload = caught.expect_err("the panic must be re-raised, not swallowed");
        assert_eq!(panic_message(&*payload), "routine blew up");

        let obs = obs.lock();
        assert!(
            obs.withdraw_count >= 1,
            "cleanup must withdraw even when the routine panicked"
        );
        assert!(
            obs.torn_down,
            "teardown must run even when the routine panicked"
        );

        let events = events.lock().unwrap();
        let data =
            custom_event(&events, "routine_panicked").expect("the panic must reach the event log");
        assert_eq!(data["routine"], "panicker");
        assert_eq!(data["message"], "routine blew up");
    }

    #[test]
    fn scan_speed_changes_are_logged_but_scan_reads_are_not() {
        let mut mock = MockController::builder().build();
        let (bus, events) = recording_bus();
        let shutdown = ShutdownFlag::new();
        let mut rt = Rt::new(&mut mock, &bus, &shutdown);

        let mut config = rt.scan().unwrap().speed_get().unwrap();
        config.forward_linear_speed_m_s = 5e-9;
        rt.scan().unwrap().speed_set(config).unwrap();
        rt.scan().unwrap().status().unwrap();

        let events = events.lock().unwrap();
        let params = started_params(&events, "scan_speed_set")
            .expect("a scan speed change must reach the event log");
        // ScanConfig speeds are f32 on the wire, so the logged value is the
        // widened f32, not the f64 literal.
        assert_eq!(
            params["forward_linear_speed_m_s"].as_f64().unwrap(),
            5e-9f32 as f64
        );
        assert!(
            started_params(&events, "scan_speed_get").is_none(),
            "reads must stay silent: speed_get is not a state change"
        );
        assert!(
            started_params(&events, "scan_status").is_none(),
            "reads must stay silent: status is polled in a loop"
        );
    }

    #[test]
    fn an_actions_parameters_reach_the_event_log() {
        let mut mock = MockController::builder().build();
        let (bus, events) = recording_bus();
        let shutdown = ShutdownFlag::new();
        let mut rt = Rt::new(&mut mock, &bus, &shutdown);

        rt.bias().unwrap().pulse(4.0, 50).unwrap();

        let events = events.lock().unwrap();
        let params =
            started_params(&events, "bias_pulse").expect("the pulse must reach the event log");
        assert_eq!(params["voltage"], 4.0);
        assert_eq!(params["duration_ms"], 50);
    }

    /// A calibrated approach with the guard armed must abort when the
    /// controller reports safe-tip protection has tripped.
    #[test]
    fn an_armed_guard_aborts_the_approach_on_a_safe_tip_trip() {
        let mut mock = MockController::builder()
            .z_controller_status(ZControllerStatus::SafeTip)
            .build();
        let (bus, _events) = recording_bus();
        let shutdown = ShutdownFlag::new();
        let mut rt = Rt::new(&mut mock, &bus, &shutdown);
        rt.set_safe_tip_guard(true);

        let err = rt
            .z()
            .unwrap()
            .calibrated_approach()
            .expect_err("an armed guard must abort on a trip");
        assert!(
            err.to_string().contains("safe-tip protection triggered"),
            "unexpected error: {err}"
        );
    }

    /// The guard is off unless asked for, so the same trip is ignored.
    #[test]
    fn a_disarmed_guard_ignores_a_safe_tip_trip() {
        let mut mock = MockController::builder()
            .z_controller_status(ZControllerStatus::SafeTip)
            .build();
        let (bus, _events) = recording_bus();
        let shutdown = ShutdownFlag::new();
        let mut rt = Rt::new(&mut mock, &bus, &shutdown);

        assert!(!rt.safe_tip_guard(), "the guard must default to off");
        rt.z()
            .unwrap()
            .calibrated_approach()
            .expect("a disarmed guard must not abort");
    }

    /// `run_routine` applies the routine's whole-run setting.
    #[test]
    fn run_routine_applies_the_routines_safe_tip_setting() {
        struct Guarded;
        impl Routine for Guarded {
            fn name(&self) -> &str {
                "guarded"
            }
            fn safe_tip_guard(&self) -> bool {
                true
            }
            fn run(&mut self, rt: &mut Rt) -> Result<Outcome, SpmError> {
                assert!(rt.safe_tip_guard(), "run_routine must arm the guard");
                Ok(Outcome::Completed)
            }
        }

        let mock = MockController::builder().build();
        let (bus, _events) = recording_bus();
        let outcome = run_routine(Box::new(mock), &bus, &ShutdownFlag::new(), &mut Guarded)
            .expect("routine should not error");
        assert_eq!(outcome, Outcome::Completed);
    }

    #[test]
    fn guarded_emits_the_cleanup_error_it_swallows() {
        let mut mock = MockController::builder().build();
        let (bus, events) = recording_bus();
        let shutdown = ShutdownFlag::new();
        let mut rt = Rt::new(&mut mock, &bus, &shutdown);

        let result: Result<(), SpmError> = rt.guarded(
            |_| Err(SpmError::Workflow("body failed".into())),
            |_| Err(SpmError::Workflow("cleanup failed".into())),
        );

        assert_eq!(
            result.expect_err("the body error wins").to_string(),
            "body failed"
        );

        let events = events.lock().unwrap();
        let data = custom_event(&events, "cleanup_failed")
            .expect("a swallowed cleanup failure must still reach the event log");
        assert_eq!(data["body_error"], "body failed");
        assert_eq!(data["cleanup_error"], "cleanup failed");
    }
}
