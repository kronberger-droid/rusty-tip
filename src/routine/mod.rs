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
//! [`run_routine`] owns the run around a routine: it calls `prepare()`,
//! applies the routine's [`RunSetup`] (Z home, safe-tip), runs the routine,
//! leaves the tip as the routine's [`ExitPolicy`] says, puts safe-tip back
//! and calls `teardown()`, whatever the outcome. It borrows the controller,
//! so one connection can run routine after routine. The shipped tip-prep
//! routine ([`crate::tip_prep::TipPrep`]) is the reference implementation.
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
//! # let mut controller: Box<dyn rusty_tip::spm_controller::SpmController> = unimplemented!();
//! let events = EventBus::new();
//! let shutdown = ShutdownFlag::new();
//! let outcome = run_routine(&mut *controller, &events, &shutdown, &mut BiasCheck { target_v: -0.5 })?;
//! # Ok(())
//! # }
//! ```

mod events;
mod rt;
mod subsystems;

pub use events::{
    CleanupFailedEvent, LayoutLoadedEvent, PanickedEvent, SettingsLoadedEvent, StreamDumpEvent,
    log_schema,
};
pub use rt::{Cycles, Rt};
pub use subsystems::{Bias, Motor, Presets, RepositionSpec, Scan, Signals, StableReadSpec, ZCtrl};

use std::panic::{self, AssertUnwindSafe};
use std::time::{Duration, Instant};

use crate::event::{Event, EventBus, EventEmitter};
use crate::shutdown::ShutdownFlag;
use crate::spm_controller::{SpmController, ZHomeMode};
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

/// How the harness leaves the tip when a routine ends, whatever the outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitPolicy {
    /// Withdraw, then back the coarse motor off this many steps.
    ///
    /// A withdraw only parks the tip at the top of the piezo range. Routines
    /// that leave the tip behind for good (tip prep) want real distance
    /// behind it; routines that expect to come back to the same spot keep
    /// the steps at zero.
    Withdraw { retract_steps: u16 },
    /// Leave Z and the coarse motor exactly as the routine left them. For
    /// work done *between* passes with the tip engaged: reconfiguring the
    /// controller, measuring or compensating drift.
    LeaveInPlace,
}

impl Default for ExitPolicy {
    /// Withdraw with no coarse retract, the safe choice for a routine that
    /// has not thought about it.
    fn default() -> Self {
        ExitPolicy::Withdraw { retract_steps: 0 }
    }
}

/// Z-controller home settings a run wants in place.
///
/// The calibrated approach "homes" the tip to get a small distance from the
/// surface before centring the frequency shift. That only makes sense as a
/// move *relative* to wherever the tip is: in [`ZHomeMode::Absolute`] the
/// same call drives Z to a fixed coordinate, which, depending on where the
/// surface sits in the Z range, can be straight into it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZHome {
    pub mode: ZHomeMode,
    /// Home position in metres. With `Relative` mode this is how far the tip
    /// backs off from the surface when homed.
    pub position_m: f64,
}

/// Safe-tip handling for a run.
///
/// The harness records the safe-tip state before touching it and puts it
/// back when the run ends, however it ends, so the operator's own setting
/// survives every run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SafeTipSetup {
    /// Current threshold in amperes. Applied with auto-recovery off and
    /// auto-pause-scan on, the combination tip prep has always run with.
    pub threshold_a: f64,
    /// Switch safe-tip off for the run. Tip prep does: a pulse is a current
    /// spike by design, and safe-tip would retract on every one.
    pub disable: bool,
}

/// What the harness sets on the controller before a routine runs.
///
/// The default sets a relative 50 nm Z home and leaves safe-tip alone.
/// Z home has a default since every calibrated approach homes the tip, and
/// a routine that forgot to say so would otherwise inherit whatever mode
/// the operator's controller happens to be in. Safe-tip has none since a
/// routine that works between passes should leave it as the operator set
/// it. What is set is put back on exit where that makes sense (safe-tip);
/// Z home is a setting with no meaningful "before", so it stays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunSetup {
    pub z_home: Option<ZHome>,
    pub safe_tip: Option<SafeTipSetup>,
}

impl RunSetup {
    /// Touch nothing at all, for a routine that must not move Z home either.
    pub const NONE: RunSetup = RunSetup {
        z_home: None,
        safe_tip: None,
    };
}

impl Default for RunSetup {
    fn default() -> Self {
        RunSetup {
            z_home: Some(ZHome {
                mode: ZHomeMode::Relative,
                position_m: 50e-9,
            }),
            safe_tip: None,
        }
    }
}

impl Outcome {
    /// The outcome as the log spells it (`completed`, `stopped_by_user`,
    /// `cycle_limit`, `timed_out`), with the detail a budget outcome carries.
    pub fn log_name(&self) -> (&'static str, Option<String>) {
        match self {
            Outcome::Completed => ("completed", None),
            Outcome::StoppedByUser => ("stopped_by_user", None),
            Outcome::CycleLimit(n) => ("cycle_limit", Some(n.to_string())),
            Outcome::TimedOut(d) => ("timed_out", Some(format!("{:.0}s", d.as_secs_f64()))),
        }
    }
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

    /// What the harness sets on the controller before `run`, and restores
    /// afterwards. The default sets a relative 50 nm Z home and leaves
    /// safe-tip alone; [`RunSetup::NONE`] touches nothing.
    fn run_setup(&self) -> RunSetup {
        RunSetup::default()
    }

    /// How the harness leaves the tip once `run` returns. The default
    /// withdraws without a coarse retract.
    fn exit_policy(&self) -> ExitPolicy {
        ExitPolicy::default()
    }
}

/// Run a routine, owning the run around it.
///
/// Calls `prepare()`, then applies the routine's [`RunSetup`]. Afterwards,
/// regardless of how the routine ended, leaves the tip as the routine's
/// [`ExitPolicy`] says (best effort, logged on failure), dumps the stream
/// buffer into the log, restores safe-tip and calls `teardown()`, so an
/// error mid-routine never leaves the tip engaged on the surface. A shutdown
/// request surfaces as `Ok(Outcome::StoppedByUser)`, never as an error.
///
/// "Regardless" includes panics: the routine runs inside
/// [`catch_unwind`](panic::catch_unwind), so a panicking routine is cleaned
/// up before the panic is re-raised unchanged. This relies on the unwinding
/// panic strategy; under `panic = "abort"` no cleanup can run.
///
/// The controller is borrowed, not consumed: the data stream and the
/// connection outlive the run, and the next routine can start on the same
/// controller straight away. Ending the session is the owner's job, through
/// [`SpmController::disconnect`] or by dropping the controller.
pub fn run_routine(
    controller: &mut dyn SpmController,
    events: &EventBus,
    shutdown: &ShutdownFlag,
    routine: &mut dyn Routine,
) -> Result<Outcome, SpmError> {
    let started = Instant::now();
    controller.prepare()?;

    let setup = routine.run_setup();
    let mut rt = Rt::new(&mut *controller, events, shutdown);
    let mut safe_tip_before = None;
    if let Err(e) = apply_run_setup(&mut rt, &setup, &mut safe_tip_before) {
        // The run never started: put back what was touched and report.
        log::error!("Run setup failed: {e}");
        restore_safe_tip(&mut rt, safe_tip_before.take());
        drop(rt);
        controller.teardown();
        return Err(e);
    }

    // AssertUnwindSafe is honest here: nothing observes `rt` or `routine`
    // after a panic except the cleanup below, which only restores hardware
    // before re-raising.
    let caught = panic::catch_unwind(AssertUnwindSafe(|| routine.run(&mut rt)));

    if let Err(payload) = &caught {
        let message = panic_message(&**payload);
        log::error!("Routine '{}' panicked: {}", routine.name(), message);
        events.emit(Event::typed(&PanickedEvent {
            routine: routine.name().to_string(),
            message,
        }));
    }

    log::info!("Cleanup starting...");
    match routine.exit_policy() {
        ExitPolicy::Withdraw { retract_steps } => {
            match rt.z() {
                Ok(mut z) => {
                    if let Err(e) = z.withdraw() {
                        log::warn!("Cleanup withdrawal failed: {}", e);
                    }
                }
                Err(e) => log::warn!("Cleanup withdrawal skipped: {}", e),
            }
            if retract_steps > 0 {
                match rt.motor() {
                    Ok(mut motor) => {
                        if let Err(e) = motor.move_3d(0, 0, -(retract_steps as i16)) {
                            log::warn!(
                                "Cleanup retract of {retract_steps} coarse steps failed: {e}"
                            );
                        }
                    }
                    Err(e) => log::warn!("Cleanup retract skipped: {}", e),
                }
            }
        }
        ExitPolicy::LeaveInPlace => log::info!("Leaving the tip in place"),
    }
    if let Some(stream) = rt.controller().stream_snapshot() {
        events.emit(Event::typed(&StreamDumpEvent { stream }));
    }
    restore_safe_tip(&mut rt, safe_tip_before.take());
    drop(rt);
    controller.teardown();
    log::info!("Cleanup complete");

    let caught = caught.map(|r| match r {
        Err(SpmError::ShutdownRequested) => Ok(Outcome::StoppedByUser),
        other => other,
    });
    let (outcome, detail) = match &caught {
        Ok(Ok(outcome)) => outcome.log_name(),
        Ok(Err(e)) => ("error", Some(e.to_string())),
        Err(payload) => ("panicked", Some(panic_message(&**payload))),
    };
    events.emit(Event::run_finished(outcome, detail, started.elapsed()));
    // Hardware is restored; a panic goes back to the caller untouched.
    caught.unwrap_or_else(|payload| panic::resume_unwind(payload))
}

/// Safe-tip as it stood before a run touched it.
#[derive(Debug, Clone, Copy)]
struct SafeTipSnapshot {
    enabled: bool,
    auto_recovery: bool,
    auto_pause_scan: bool,
    threshold_a: f64,
}

/// Apply a [`RunSetup`], every step through the event log.
///
/// The snapshot is written through `before` as soon as it is taken, so a
/// failure on a later step still leaves the caller something to restore.
fn apply_run_setup(
    rt: &mut Rt,
    setup: &RunSetup,
    before: &mut Option<SafeTipSnapshot>,
) -> Result<(), SpmError> {
    if let Some(home) = setup.z_home {
        rt.logged(
            "set_z_home",
            serde_json::json!({ "mode": format!("{:?}", home.mode), "position_m": home.position_m }),
            |c| c.set_z_home(home.mode, home.position_m),
        )?;
        log::info!(
            "Z home: mode={:?}, pos={:.0} nm",
            home.mode,
            home.position_m * 1e9
        );
    }

    if let Some(safe_tip) = setup.safe_tip {
        // Record safe-tip before touching it, so the exit can put it back.
        let (auto_recovery, auto_pause_scan, threshold_a) = rt.controller().safe_tip_status()?;
        let snapshot = SafeTipSnapshot {
            enabled: rt.controller().safe_tip_enabled()?,
            auto_recovery,
            auto_pause_scan,
            threshold_a,
        };
        log::info!(
            "Safe-tip before run: {}, threshold {:.2e} A",
            if snapshot.enabled { "on" } else { "off" },
            snapshot.threshold_a
        );
        *before = Some(snapshot);

        rt.logged(
            "safe_tip_configure",
            serde_json::json!({
                "auto_recovery": false,
                "auto_pause_scan": true,
                "threshold_a": safe_tip.threshold_a,
            }),
            |c| c.safe_tip_configure(false, true, safe_tip.threshold_a),
        )?;
        log::info!("Safe-tip threshold: {:.2e} A", safe_tip.threshold_a);
        if safe_tip.disable {
            rt.logged(
                "safe_tip_set_enabled",
                serde_json::json!({ "enabled": false }),
                |c| c.safe_tip_set_enabled(false),
            )?;
            log::info!("Safe-tip switched off for the run");
        }
    }
    Ok(())
}

/// Put safe-tip back the way [`apply_run_setup`] found it. Best effort:
/// this runs during cleanup, where nothing may short-circuit.
fn restore_safe_tip(rt: &mut Rt, before: Option<SafeTipSnapshot>) {
    let Some(before) = before else {
        return;
    };
    let restored = rt.logged(
        "safe_tip_restore",
        serde_json::json!({
            "enabled": before.enabled,
            "auto_recovery": before.auto_recovery,
            "auto_pause_scan": before.auto_pause_scan,
            "threshold_a": before.threshold_a,
        }),
        |c| {
            c.safe_tip_configure(
                before.auto_recovery,
                before.auto_pause_scan,
                before.threshold_a,
            )?;
            c.safe_tip_set_enabled(before.enabled)
        },
    );
    match restored {
        Ok(()) => log::info!(
            "Safe-tip restored: {}",
            if before.enabled { "on" } else { "off" }
        ),
        Err(e) => log::warn!("Failed to restore safe-tip: {e}"),
    }
}

/// Best-effort rendering of a caught panic payload, which is a `&str` for
/// `panic!("literal")` and a `String` for formatted messages.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
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
            Event::Custom { kind, data, .. } if kind == wanted => Some(data.clone()),
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
        let mut mock = MockController::builder().build();
        let obs = mock.observations();
        let (bus, events) = recording_bus();

        let caught = panic::catch_unwind(AssertUnwindSafe(|| {
            run_routine(&mut mock, &bus, &ShutdownFlag::new(), &mut Panicker)
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
            custom_event(&events, "routine/panicked").expect("the panic must reach the event log");
        assert_eq!(data["routine"], "panicker");
        assert_eq!(data["message"], "routine blew up");
        match events.last() {
            Some(Event::RunFinished {
                outcome, detail, ..
            }) => {
                assert_eq!(outcome, "panicked");
                assert_eq!(detail.as_deref(), Some("routine blew up"));
            }
            other => panic!("the log must close on run_finished, got {other:?}"),
        }
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
        let data = custom_event(&events, "routine/cleanup_failed")
            .expect("a swallowed cleanup failure must still reach the event log");
        assert_eq!(data["body_error"], "body failed");
        assert_eq!(data["cleanup_error"], "cleanup failed");
    }

    struct Idle {
        setup: RunSetup,
        exit: ExitPolicy,
    }

    impl Routine for Idle {
        fn name(&self) -> &str {
            "idle"
        }

        fn run(&mut self, _rt: &mut Rt) -> Result<Outcome, SpmError> {
            Ok(Outcome::Completed)
        }

        fn run_setup(&self) -> RunSetup {
            self.setup
        }

        fn exit_policy(&self) -> ExitPolicy {
            self.exit
        }
    }

    /// The calibrated approach homes the tip to back off from the surface.
    /// That is only a back-off in relative mode; absolute mode drives Z to a
    /// coordinate, surface or not. 0.3 and 0.4 shipped the wrong default,
    /// so the harness sets relative mode for any routine that says nothing.
    #[test]
    fn the_default_setup_sets_a_relative_home_and_leaves_safe_tip_alone() {
        let mut mock = MockController::builder().build();
        let obs = mock.observations();
        let (bus, events) = recording_bus();

        run_routine(
            &mut mock,
            &bus,
            &ShutdownFlag::new(),
            &mut Idle {
                setup: RunSetup::default(),
                exit: ExitPolicy::default(),
            },
        )
        .unwrap();

        let obs = obs.lock();
        let events = events.lock().unwrap();
        let home = started_params(&events, "set_z_home").expect("z home is set by default");
        assert_eq!(home["mode"], "Relative");
        assert_eq!(home["position_m"], 50e-9);
        assert!(!obs.called("safe_tip_configure"));
        assert!(!obs.called("safe_tip_set_enabled"));
        assert_eq!(obs.withdraw_count, 1);
        assert!(obs.motor_displacements.is_empty(), "no retract by default");
        assert!(obs.prepared && obs.torn_down);
    }

    #[test]
    fn run_setup_none_touches_nothing() {
        let mut mock = MockController::builder().build();
        let obs = mock.observations();
        let (bus, _) = recording_bus();

        run_routine(
            &mut mock,
            &bus,
            &ShutdownFlag::new(),
            &mut Idle {
                setup: RunSetup::NONE,
                exit: ExitPolicy::LeaveInPlace,
            },
        )
        .unwrap();

        let obs = obs.lock();
        assert!(!obs.called("set_z_home"));
        assert!(!obs.called("safe_tip_configure"));
        assert_eq!(obs.withdraw_count, 0);
    }

    #[test]
    fn leave_in_place_neither_withdraws_nor_retracts() {
        let mut mock = MockController::builder().build();
        let obs = mock.observations();
        let (bus, _) = recording_bus();

        run_routine(
            &mut mock,
            &bus,
            &ShutdownFlag::new(),
            &mut Idle {
                setup: RunSetup::NONE,
                exit: ExitPolicy::LeaveInPlace,
            },
        )
        .unwrap();

        let obs = obs.lock();
        assert_eq!(obs.withdraw_count, 0);
        assert_eq!(obs.motor_moves, 0);
        assert!(obs.torn_down, "teardown still brackets the run");
    }

    #[test]
    fn safe_tip_is_switched_off_for_the_run_and_put_back_after() {
        let mut mock = MockController::builder().build();
        let obs = mock.observations();
        {
            let mut obs = obs.lock();
            obs.safe_tip_enabled = true;
            obs.safe_tip_config = (true, false, 5e-9);
        }
        let (bus, events) = recording_bus();

        run_routine(
            &mut mock,
            &bus,
            &ShutdownFlag::new(),
            &mut Idle {
                setup: RunSetup {
                    z_home: Some(ZHome {
                        mode: ZHomeMode::Relative,
                        position_m: 50e-9,
                    }),
                    safe_tip: Some(SafeTipSetup {
                        threshold_a: 1e-9,
                        disable: true,
                    }),
                },
                exit: ExitPolicy::Withdraw { retract_steps: 2 },
            },
        )
        .unwrap();

        let obs = obs.lock();
        assert!(obs.safe_tip_enabled, "restored to on");
        assert_eq!(obs.safe_tip_config, (true, false, 5e-9), "restored");
        // Off during the run: the disable comes after the snapshot and
        // before the restore.
        let off = obs.first_index("safe_tip_set_enabled").unwrap();
        let on = obs.last_index("safe_tip_set_enabled").unwrap();
        let withdraw = obs.last_index("withdraw").unwrap();
        assert!(off < withdraw && withdraw < on, "calls: {:?}", obs.calls);
        assert_eq!(obs.motor_displacements.last(), Some(&(0, 0, -2)));

        let events = events.lock().unwrap();
        let home = started_params(&events, "set_z_home").expect("z home is logged");
        assert_eq!(home["mode"], "Relative");
        assert!(started_params(&events, "safe_tip_configure").is_some());
        assert!(started_params(&events, "safe_tip_restore").is_some());
    }

    #[test]
    fn a_failing_setup_restores_safe_tip_and_never_runs_the_routine() {
        let mut mock = MockController::builder()
            .fail_on_call(
                "safe_tip_set_enabled",
                1,
                crate::mock_controller::FaultKind::Io,
            )
            .build();
        let obs = mock.observations();
        obs.lock().safe_tip_enabled = true;
        let (bus, _) = recording_bus();

        struct WithSafeTip;
        impl Routine for WithSafeTip {
            fn name(&self) -> &str {
                "with_safe_tip"
            }
            fn run(&mut self, rt: &mut Rt) -> Result<Outcome, SpmError> {
                rt.bias()?.set(-1.0)?;
                Ok(Outcome::Completed)
            }
            fn run_setup(&self) -> RunSetup {
                RunSetup {
                    z_home: None,
                    safe_tip: Some(SafeTipSetup {
                        threshold_a: 1e-9,
                        disable: true,
                    }),
                }
            }
        }

        let result = run_routine(&mut mock, &bus, &ShutdownFlag::new(), &mut WithSafeTip);

        assert!(result.unwrap_err().is_connection_error());
        let obs = obs.lock();
        assert_eq!(obs.bias, 0.0, "the routine body never ran");
        assert!(obs.torn_down);
        assert!(obs.safe_tip_enabled, "restored after the failed disable");
        assert_eq!(
            obs.count("safe_tip_set_enabled"),
            2,
            "the failed disable, then the restore: {:?}",
            obs.calls
        );
    }
}
