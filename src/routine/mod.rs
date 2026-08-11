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

use std::time::Duration;

use crate::event::EventBus;
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
pub fn run_routine(
    mut controller: Box<dyn SpmController>,
    events: &EventBus,
    shutdown: &ShutdownFlag,
    routine: &mut dyn Routine,
) -> Result<Outcome, SpmError> {
    controller.prepare()?;

    let mut rt = Rt::new(&mut *controller, events, shutdown);
    let result = routine.run(&mut rt);

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

    match result {
        Err(SpmError::ShutdownRequested) => Ok(Outcome::StoppedByUser),
        other => other,
    }
}
