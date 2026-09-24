//! Z drift as a routine: status, one measurement, a compensation loop, or
//! switching compensation off.
//!
//! The actions in [`crate::action::drift`] do the work; this wraps one of
//! them as a [`Routine`] so the CLI and the workbench run the same code, and
//! so the harness's log, stop handling and cleanup apply. The exit policy is
//! [`ExitPolicy::LeaveInPlace`] and the setup is [`RunSetup::NONE`]: drift is
//! measured *between* passes with the tip engaged and the feedback closed,
//! and nothing here may move the tip or touch the operator's settings.
//!
//! Everything learned goes to the log as typed events (`drift/status`,
//! `drift/measured`, `drift/compensated`, plus the actions' own
//! `drift/burst`), and is kept on the routine as a [`DriftReport`] for a
//! caller that wants to print it.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::action::drift::{CompensateDrift, DriftCompensation, DriftEstimate};
use crate::event::Event;
use crate::experiment_log::{LogEvent, ToolSchema};
use crate::routine::{ExitPolicy, Outcome, Routine, Rt, RunSetup};
use crate::signal_registry::SignalIndex;
use crate::spm_controller::DriftComp;
use crate::spm_error::SpmError;

/// Which drift operation to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum DriftOp {
    /// Report the compensation velocities and which axes have saturated.
    /// Changes nothing.
    #[default]
    Status,
    /// Fit a Z drift rate from one burst. Changes nothing.
    Measure,
    /// Measure in bursts, correcting the Z velocity after each, and leave
    /// compensation switched on.
    Compensate,
    /// Switch compensation off. The velocities are kept, so `Status` still
    /// shows what was last applied.
    Off,
}

impl DriftOp {
    pub fn name(self) -> &'static str {
        match self {
            DriftOp::Status => "status",
            DriftOp::Measure => "measure",
            DriftOp::Compensate => "compensate",
            DriftOp::Off => "off",
        }
    }
}

fn default_window_ms() -> u64 {
    5_000
}
fn default_bursts() -> usize {
    5
}
fn default_trial_vz() -> f64 {
    20e-12
}
fn default_samples() -> usize {
    16
}
fn default_true() -> bool {
    true
}

/// Everything a drift run needs besides the Z signal, in SI units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DriftParams {
    pub op: DriftOp,
    /// Length of one measurement burst, in milliseconds.
    #[serde(default = "default_window_ms")]
    pub window_ms: u64,
    /// Bursts `Compensate` spends, the baseline and the trial included.
    #[serde(default = "default_bursts")]
    pub bursts: usize,
    /// Velocity step `Compensate` uses to learn which way the controller's
    /// `vz` runs, in m/s.
    #[serde(default = "default_trial_vz")]
    pub trial_vz: f64,
    /// The response, once known for this controller: 1 if a positive `vz`
    /// adds to the measured drift, -1 if it subtracts. Skips the trial.
    #[serde(default)]
    pub response: Option<f64>,
    /// Timed reads per window when Z is polled rather than streamed.
    #[serde(default = "default_samples")]
    pub samples: usize,
    /// Fail rather than poll when the data stream does not carry Z. Polled
    /// bursts are far noisier, so this is on unless polling is wanted.
    #[serde(default = "default_true")]
    pub require_stream: bool,
}

impl Default for DriftParams {
    fn default() -> Self {
        Self {
            op: DriftOp::Status,
            window_ms: default_window_ms(),
            bursts: default_bursts(),
            trial_vz: default_trial_vz(),
            response: None,
            samples: default_samples(),
            require_stream: true,
        }
    }
}

/// When a status was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusWhen {
    Before,
    After,
}

/// The compensation as the controller reports it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DriftStatusEvent {
    pub when: StatusWhen,
    pub enabled: bool,
    pub vx_m_s: f64,
    pub vy_m_s: f64,
    pub vz_m_s: f64,
    pub saturation_limit_percent: f64,
    pub x_saturated: bool,
    pub y_saturated: bool,
    pub z_saturated: bool,
}

impl DriftStatusEvent {
    fn new(when: StatusWhen, comp: &DriftComp) -> Self {
        Self {
            when,
            enabled: comp.enabled,
            vx_m_s: comp.vx,
            vy_m_s: comp.vy,
            vz_m_s: comp.vz,
            saturation_limit_percent: comp.saturation_limit_percent,
            x_saturated: comp.x_saturated,
            y_saturated: comp.y_saturated,
            z_saturated: comp.z_saturated,
        }
    }
}

impl LogEvent for DriftStatusEvent {
    const KIND: &'static str = "drift/status";
}

/// One measured drift rate.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DriftMeasuredEvent {
    pub rate_m_s: f64,
    pub std_err_m_s: f64,
    pub samples: usize,
    pub window_s: f64,
    /// Inside two standard errors of zero.
    pub negligible: bool,
}

impl LogEvent for DriftMeasuredEvent {
    const KIND: &'static str = "drift/measured";
}

/// Where a compensation run left things.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DriftCompensatedEvent {
    pub residual_rate_m_s: f64,
    pub residual_std_err_m_s: f64,
    /// Velocity left on the controller.
    pub vz_m_s: f64,
    pub response: Option<f64>,
    pub bursts: usize,
    pub converged: bool,
}

impl LogEvent for DriftCompensatedEvent {
    const KIND: &'static str = "drift/compensated";
}

/// Every kind a drift run can write.
pub fn log_schema() -> ToolSchema {
    ToolSchema::new("drift")
        .with::<DriftStatusEvent>()
        .with::<DriftMeasuredEvent>()
        .with::<DriftCompensatedEvent>()
        .including(crate::action::drift::log_schema())
        .including(crate::routine::log_schema())
}

/// What a run found, for a caller that prints rather than reads the log.
#[derive(Debug, Clone, Default)]
pub struct DriftReport {
    pub before: Option<DriftComp>,
    pub estimate: Option<DriftEstimate>,
    pub compensation: Option<DriftCompensation>,
    pub after: Option<DriftComp>,
}

/// One drift operation as a routine. See the [module docs](self).
pub struct DriftRoutine {
    z: SignalIndex,
    params: DriftParams,
    /// Filled in as the run goes.
    pub report: DriftReport,
}

impl DriftRoutine {
    pub fn new(z: SignalIndex, params: DriftParams) -> Self {
        Self {
            z,
            params,
            report: DriftReport::default(),
        }
    }

    fn status(&mut self, rt: &mut Rt, when: StatusWhen) -> Result<DriftComp, SpmError> {
        let comp = rt.drift()?.get()?;
        rt.emit(Event::typed(&DriftStatusEvent::new(when, &comp)));
        Ok(comp)
    }
}

impl Routine for DriftRoutine {
    fn name(&self) -> &str {
        "drift"
    }

    fn run_setup(&self) -> RunSetup {
        RunSetup::NONE
    }

    fn exit_policy(&self) -> ExitPolicy {
        ExitPolicy::LeaveInPlace
    }

    fn run(&mut self, rt: &mut Rt) -> Result<Outcome, SpmError> {
        let p = self.params.clone();
        let measures = matches!(p.op, DriftOp::Measure | DriftOp::Compensate);
        if measures && p.require_stream && !rt.controller().streams_signal(self.z) {
            return Err(SpmError::Workflow(format!(
                "the data stream does not carry Z (signal {}); add it to the TCP channel \
                 mapping and reconnect, or allow polling",
                self.z
            )));
        }

        let before = self.status(rt, StatusWhen::Before)?;
        self.report.before = Some(before);

        match p.op {
            DriftOp::Status => {}
            DriftOp::Measure => {
                let estimate = rt
                    .drift()?
                    .measure_z(self.z, Duration::from_millis(p.window_ms))?;
                rt.emit(Event::typed(&DriftMeasuredEvent {
                    rate_m_s: estimate.rate_m_s,
                    std_err_m_s: estimate.std_err_m_s,
                    samples: estimate.samples,
                    window_s: estimate.window_s,
                    negligible: estimate.is_negligible(0.0),
                }));
                self.report.estimate = Some(estimate);
            }
            DriftOp::Compensate => {
                let result = rt.drift()?.compensate(&CompensateDrift {
                    window_ms: p.window_ms,
                    samples: p.samples,
                    bursts: p.bursts,
                    trial_vz: p.trial_vz,
                    response: p.response,
                    ..CompensateDrift::new(self.z)
                })?;
                rt.emit(Event::typed(&DriftCompensatedEvent {
                    residual_rate_m_s: result.residual.rate_m_s,
                    residual_std_err_m_s: result.residual.std_err_m_s,
                    vz_m_s: result.vz_m_s,
                    response: result.response,
                    bursts: result.bursts,
                    converged: result.converged,
                }));
                self.report.compensation = Some(result);
                self.report.after = Some(self.status(rt, StatusWhen::After)?);
            }
            DriftOp::Off => {
                rt.drift()?.set(&DriftComp {
                    enabled: false,
                    ..before
                })?;
                self.report.after = Some(self.status(rt, StatusWhen::After)?);
            }
        }
        Ok(Outcome::Completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventBus, Observer};
    use crate::mock_controller::MockController;
    use crate::routine::run_routine;
    use crate::shutdown::ShutdownFlag;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<Event>>>);

    impl Observer for Recorder {
        fn on_event(&self, event: &Event) {
            self.0.lock().unwrap().push(event.clone());
        }
    }

    fn kinds(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Custom { kind, .. } => Some(kind.clone()),
                _ => None,
            })
            .collect()
    }

    const Z: SignalIndex = SignalIndex(30);

    #[test]
    fn status_reads_and_leaves_the_tip_in_place() {
        let mut mock = MockController::builder().build();
        let obs = mock.observations();
        let recorder = Recorder::default();
        let mut bus = EventBus::new();
        bus.add_observer(Box::new(recorder.clone()));

        let mut routine = DriftRoutine::new(Z, DriftParams::default());
        run_routine(&mut mock, &bus, &ShutdownFlag::new(), &mut routine).unwrap();

        let obs = obs.lock();
        assert_eq!(obs.withdraw_count, 0);
        assert!(!obs.called("set_z_home"));
        assert!(!obs.called("safe_tip_configure"));
        assert!(routine.report.before.is_some());
        assert_eq!(kinds(&recorder.0.lock().unwrap()), vec!["drift/status"]);
    }

    #[test]
    fn off_switches_compensation_off_and_keeps_the_velocities() {
        let mut mock = MockController::builder().build();
        let obs = mock.observations();
        obs.lock().drift_comp.enabled = true;
        obs.lock().drift_comp.vz = 3e-12;

        let params = DriftParams {
            op: DriftOp::Off,
            ..Default::default()
        };
        let mut routine = DriftRoutine::new(Z, params);
        run_routine(
            &mut mock,
            &EventBus::new(),
            &ShutdownFlag::new(),
            &mut routine,
        )
        .unwrap();

        let after = routine.report.after.unwrap();
        assert!(!after.enabled);
        assert_eq!(after.vz, 3e-12);
    }

    #[test]
    fn measuring_without_a_stream_fails_clearly_unless_polling_is_allowed() {
        let mut mock = MockController::builder().build();
        let params = DriftParams {
            op: DriftOp::Measure,
            window_ms: 10,
            ..Default::default()
        };
        let err = run_routine(
            &mut mock,
            &EventBus::new(),
            &ShutdownFlag::new(),
            &mut DriftRoutine::new(Z, params.clone()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("does not carry Z"), "{err}");

        let polled = DriftParams {
            require_stream: false,
            samples: 4,
            ..params
        };
        let mut routine = DriftRoutine::new(Z, polled);
        run_routine(
            &mut mock,
            &EventBus::new(),
            &ShutdownFlag::new(),
            &mut routine,
        )
        .unwrap();
        assert!(routine.report.estimate.is_some());
    }

    #[test]
    fn compensate_on_a_drifting_z_reports_the_residual_and_the_status_after() {
        let mut mock = MockController::builder().z_drift(Z, 2e-12, -1.0).build();
        let recorder = Recorder::default();
        let mut bus = EventBus::new();
        bus.add_observer(Box::new(recorder.clone()));

        let params = DriftParams {
            op: DriftOp::Compensate,
            window_ms: 200,
            bursts: 3,
            ..Default::default()
        };
        let mut routine = DriftRoutine::new(Z, params);
        run_routine(&mut mock, &bus, &ShutdownFlag::new(), &mut routine).unwrap();

        let result = routine.report.compensation.as_ref().unwrap();
        assert_eq!(result.bursts, 3);
        assert!(routine.report.after.unwrap().enabled);
        let kinds = kinds(&recorder.0.lock().unwrap());
        assert_eq!(kinds.first().map(String::as_str), Some("drift/status"));
        assert!(kinds.contains(&"drift/burst".to_string()));
        assert!(kinds.contains(&"drift/compensated".to_string()));
        assert_eq!(kinds.last().map(String::as_str), Some("drift/status"));
    }

    #[test]
    fn the_schema_declares_every_kind_it_writes() {
        let declared: Vec<String> = log_schema().kinds.into_iter().map(|k| k.kind).collect();
        for kind in [
            "drift/status",
            "drift/measured",
            "drift/compensated",
            "drift/burst",
            "routine/panicked",
        ] {
            assert!(declared.contains(&kind.to_string()), "{kind} missing");
        }
    }
}
