//! Capability-checked subsystem handles handed out by [`Rt`].
//!
//! Each handle is a thin, short-lived view: its methods delegate to the
//! library's action implementations (so every operation emits
//! started/completed/failed events) or, for pure read/restore plumbing,
//! straight to the controller. Fetch a handle per statement:
//! `rt.bias()?.pulse(4.0, 50)?`.

use std::path::PathBuf;
use std::time::Duration;

use nanonis_rs::scan::{ScanConfig, ScanLineEnd, ScanProps, ScanPropsBuilder};

use crate::multi_pass::MultiPassConfig;
use crate::spm_controller::{DriftComp, ScanBuffer};

use crate::action::ActionOutput;
use crate::action::bias::{BiasPulse, ReadBias, SetBias};
use crate::action::drift::{CompensateDrift, DriftCompensation, DriftEstimate, MeasureZDrift};
use crate::action::motor::{MoveMotor3D, Reposition};
use crate::action::multi_pass::{ActivateMultiPass, ApplyMultiPass, LoadMultiPass, SaveMultiPass};
use crate::action::scan::{ScanActionParam, ScanControl, ScanDirectionParam};
use crate::action::signals::{ReadSignal, ReadStableSignal};
use crate::action::z_controller::{AutoApproach, CalibratedApproach, SetZSetpoint, Withdraw};
use crate::signal_registry::SignalIndex;
use crate::spm_error::SpmError;

use super::Rt;

type Result<T> = std::result::Result<T, SpmError>;

fn expect_value(name: &str, output: ActionOutput) -> Result<f64> {
    match output {
        ActionOutput::Value(v) => Ok(v),
        other => Err(SpmError::Protocol(format!(
            "{name} returned unexpected output: {other:?}"
        ))),
    }
}

// ============================================================================
// Bias
// ============================================================================

/// Bias voltage control, from [`Rt::bias`].
pub struct Bias<'r, 'a> {
    pub(crate) rt: &'r mut Rt<'a>,
}

impl Bias<'_, '_> {
    /// Read the current bias voltage in volts.
    pub fn get(&mut self) -> Result<f64> {
        let output = self.rt.exec(&ReadBias)?;
        expect_value("read_bias", output)
    }

    /// Set the bias voltage in volts.
    pub fn set(&mut self, voltage: f64) -> Result<()> {
        self.rt.exec(&SetBias { voltage })?;
        Ok(())
    }

    /// Fire a bias pulse to an absolute signed voltage for `width_ms`, with
    /// the z-controller held. The bias returns to its previous value after.
    pub fn pulse(&mut self, voltage: f64, width_ms: u64) -> Result<()> {
        self.rt.exec(&BiasPulse {
            voltage,
            duration_ms: width_ms,
            ..Default::default()
        })?;
        Ok(())
    }
}

// ============================================================================
// Z-controller
// ============================================================================

/// Z-controller operations, from [`Rt::z`].
pub struct ZCtrl<'r, 'a> {
    pub(crate) rt: &'r mut Rt<'a>,
}

impl ZCtrl<'_, '_> {
    /// Withdraw the tip from the surface.
    pub fn withdraw(&mut self) -> Result<()> {
        self.rt.exec(&Withdraw::default())?;
        Ok(())
    }

    /// Start the auto-approach and wait for it to finish.
    pub fn auto_approach(&mut self) -> Result<()> {
        self.rt.exec(&AutoApproach::default())?;
        Ok(())
    }

    /// Approach with the calibrated sequence: approach, small withdraw,
    /// center the frequency shift, re-approach. Each approach gets the
    /// action's default budget of five minutes.
    pub fn calibrated_approach(&mut self) -> Result<()> {
        self.rt.exec(&CalibratedApproach::default())?;
        Ok(())
    }

    /// [`calibrated_approach`](Self::calibrated_approach) with an explicit
    /// budget for each of its two approaches. For the approaches that start
    /// from a full withdraw, which can take longer than the default allows.
    pub fn calibrated_approach_within(&mut self, timeout: Duration) -> Result<()> {
        self.rt.exec(&CalibratedApproach {
            wait: true,
            timeout_ms: timeout.as_millis() as u64,
        })?;
        Ok(())
    }

    /// Set the z-controller setpoint (e.g. current in amperes).
    pub fn set_setpoint(&mut self, setpoint: f64) -> Result<()> {
        self.rt.exec(&SetZSetpoint { setpoint })?;
        Ok(())
    }
}

// ============================================================================
// Signals
// ============================================================================

/// Gates and sizing for a stable signal read, used by [`Signals::read_stable`].
///
/// A stable read collects `num_samples` stream samples and accepts the batch
/// only if it passes the noise gate (`max_std_dev`) and the drift gate
/// (`max_slope`, in units/s judged at `sample_rate_hz`); failing batches are
/// retried up to `max_retries` times with exponential backoff.
#[derive(Debug, Clone)]
pub struct StableReadSpec {
    pub num_samples: usize,
    pub max_std_dev: f64,
    pub max_slope: f64,
    pub max_retries: usize,
    pub sample_rate_hz: f64,
}

/// Signal reading, from [`Rt::signals`].
pub struct Signals<'r, 'a> {
    pub(crate) rt: &'r mut Rt<'a>,
}

impl Signals<'_, '_> {
    /// Read a single signal value.
    pub fn read(&mut self, index: SignalIndex) -> Result<f64> {
        let output = self.rt.exec(&ReadSignal {
            index,
            wait_for_newest: true,
        })?;
        expect_value("read_signal", output)
    }

    /// Read a noise- and drift-gated signal value (see [`StableReadSpec`]).
    /// Emits one `stable_read` measurement event per accepted batch.
    pub fn read_stable(&mut self, index: SignalIndex, spec: &StableReadSpec) -> Result<f64> {
        let output = self.rt.exec(&ReadStableSignal {
            index,
            num_samples: spec.num_samples,
            max_std_dev: spec.max_std_dev,
            max_slope: spec.max_slope,
            max_retries: spec.max_retries,
            sample_rate_hz: spec.sample_rate_hz,
        })?;
        expect_value("read_stable_signal", output)
    }

    /// Discard buffered stream samples so the next read sees only fresh data.
    pub fn clear_buffer(&mut self) {
        self.rt.controller().clear_data_buffer();
    }
}

// ============================================================================
// Motor
// ============================================================================

/// Parameters for [`Motor::reposition`]: withdraw, step the coarse motors,
/// settle, re-approach (calibrated), settle again.
#[derive(Debug, Clone)]
pub struct RepositionSpec {
    /// Coarse motor steps in x.
    pub x_steps: i16,
    /// Coarse motor steps in y.
    pub y_steps: i16,
    /// Z retraction steps before the lateral move.
    pub z_retract: i16,
    /// Settle between the motor move and the re-approach (ms).
    pub post_move_settle_ms: u64,
    /// Settle after the re-approach (ms).
    pub post_approach_settle_ms: u64,
    /// Budget for each of the two approaches in the re-approach (ms).
    pub approach_timeout_ms: u64,
}

impl Default for RepositionSpec {
    fn default() -> Self {
        Self {
            x_steps: 0,
            y_steps: 0,
            z_retract: -3,
            post_move_settle_ms: 500,
            post_approach_settle_ms: 500,
            approach_timeout_ms: crate::action::z_controller::DEFAULT_APPROACH_TIMEOUT_MS,
        }
    }
}

/// Coarse motor positioning, from [`Rt::motor`].
pub struct Motor<'r, 'a> {
    pub(crate) rt: &'r mut Rt<'a>,
}

impl Motor<'_, '_> {
    /// Move to a fresh surface spot: withdraw, step the motors, re-approach.
    /// Also needs the `ZController` and `Pll` capabilities.
    pub fn reposition(&mut self, spec: &RepositionSpec) -> Result<()> {
        self.rt.exec(&Reposition {
            x_steps: spec.x_steps,
            y_steps: spec.y_steps,
            z_retract: spec.z_retract,
            post_move_settle_ms: spec.post_move_settle_ms,
            post_approach_settle_ms: spec.post_approach_settle_ms,
            approach_timeout_ms: spec.approach_timeout_ms,
        })?;
        Ok(())
    }

    /// Step the coarse motors by (x, y, z) and wait for the move to finish.
    pub fn move_3d(&mut self, x: i16, y: i16, z: i16) -> Result<()> {
        self.rt.exec(&MoveMotor3D {
            x,
            y,
            z,
            wait: true,
        })?;
        Ok(())
    }
}

// ============================================================================
// Scan
// ============================================================================

/// Scan control, from [`Rt::scan`].
///
/// Everything that changes the scanner's state emits events: `start`/`stop`
/// through the action layer, `props_set`/`speed_set` directly. The reads
/// (`status`, `props_get`, `speed_get`) stay silent — `status` in particular
/// is polled in a loop, and logging a control-flow poll buries the run.
pub struct Scan<'r, 'a> {
    pub(crate) rt: &'r mut Rt<'a>,
}

impl Scan<'_, '_> {
    /// Start scanning in the given direction.
    pub fn start(&mut self, direction: ScanDirectionParam) -> Result<()> {
        self.rt.exec(&ScanControl {
            action: ScanActionParam::Start,
            direction,
        })?;
        Ok(())
    }

    /// Stop scanning.
    pub fn stop(&mut self) -> Result<()> {
        self.rt.exec(&ScanControl {
            action: ScanActionParam::Stop,
            direction: ScanDirectionParam::Up,
        })?;
        Ok(())
    }

    /// Whether a scan is currently running.
    pub fn status(&mut self) -> Result<bool> {
        self.rt.controller().scan_status()
    }

    /// Current scan properties (for save/restore around a sweep).
    pub fn props_get(&mut self) -> Result<ScanProps> {
        self.rt.controller().scan_props_get()
    }

    /// Apply scan properties.
    pub fn props_set(&mut self, props: ScanPropsBuilder) -> Result<()> {
        self.rt.logged(
            "scan_props_set",
            serde_json::json!({ "props": format!("{props:?}") }),
            |c| c.scan_props_set(props),
        )
    }

    /// Which signals the scan records, and the frame resolution.
    pub fn buffer(&mut self) -> Result<ScanBuffer> {
        self.rt.controller().scan_buffer_get()
    }

    /// Add `channels` to the scan buffer, keeping whatever is already there.
    ///
    /// Returns the buffer as it now stands. A signal that is not in the buffer
    /// is not acquired, however it is configured elsewhere.
    pub fn ensure_channels(&mut self, channels: &[SignalIndex]) -> Result<ScanBuffer> {
        let channels = channels.to_vec();
        self.rt.logged(
            "scan_buffer_ensure",
            serde_json::json!({ "channels": channels.iter().map(|c| c.0).collect::<Vec<_>>() }),
            |c| c.scan_buffer_ensure(&channels),
        )
    }

    /// Block until the scan finishes a line, or until `timeout` elapses.
    ///
    /// Check `timed_out` on the result before trusting the rest: a timeout
    /// returns normally, carrying stale line and pass numbers. Silent, since
    /// this is polled in a loop.
    pub fn wait_end_of_line(&mut self, timeout: Duration) -> Result<ScanLineEnd> {
        self.rt.controller().scan_wait_end_of_line(timeout)
    }

    /// Current scan speed configuration (for save/restore).
    pub fn speed_get(&mut self) -> Result<ScanConfig> {
        self.rt.controller().scan_speed_get()
    }

    /// Apply a scan speed configuration.
    pub fn speed_set(&mut self, config: ScanConfig) -> Result<()> {
        self.rt.logged(
            "scan_speed_set",
            serde_json::json!({
                "forward_linear_speed_m_s": config.forward_linear_speed_m_s,
                "backward_linear_speed_m_s": config.backward_linear_speed_m_s,
                "keep_parameter_constant": config.keep_parameter_constant,
            }),
            |c| c.scan_speed_set(config),
        )
    }
}

/// Piezo drift compensation, from [`Rt::drift`].
///
/// The real-time system applies a constant velocity per axis for as long as
/// compensation is on. This handle is about working out what that velocity
/// should be, and noticing when it has gone stale.
pub struct Drift<'r, 'a> {
    pub(crate) rt: &'r mut Rt<'a>,
}

impl Drift<'_, '_> {
    /// Current velocities and saturation state.
    pub fn get(&mut self) -> Result<DriftComp> {
        self.rt.controller().drift_comp_get()
    }

    /// Set the compensation velocities directly.
    ///
    /// Prefer [`compensate`](Self::compensate) unless the velocity is already
    /// known: the sign convention is undocumented, and applying it backwards
    /// doubles the drift rather than cancelling it.
    pub fn set(&mut self, comp: &DriftComp) -> Result<()> {
        let comp = *comp;
        self.rt.logged(
            "drift_comp_set",
            serde_json::json!({
                "enabled": comp.enabled,
                "vx_m_s": comp.vx,
                "vy_m_s": comp.vy,
                "vz_m_s": comp.vz,
            }),
            |c| c.drift_comp_set(&comp),
        )
    }

    /// Measure the Z drift rate with one burst of `window`, without changing
    /// anything. Needs the Z controller on, and a flat, quiet spot. The
    /// estimate carries its own standard error; a rate inside it is a
    /// finding, not a failure.
    pub fn measure_z(&mut self, z: SignalIndex, window: Duration) -> Result<DriftEstimate> {
        let output = self.rt.exec(&MeasureZDrift {
            z,
            window_ms: window.as_millis() as u64,
            samples: 16,
        })?;
        output.into_data("measure_z_drift")
    }

    /// Measure the Z drift in bursts, correcting after each, and leave the
    /// controller compensating for it. Start from [`CompensateDrift::new`]
    /// and override what differs. The result says what is left, under which
    /// velocity, and whether the loop converged or ran out of bursts.
    pub fn compensate(&mut self, action: &CompensateDrift) -> Result<DriftCompensation> {
        let output = self.rt.exec(action)?;
        output.into_data("compensate_drift")
    }
}

/// Multi-pass scanning, from [`Rt::multi_pass`].
///
/// Multi-pass scans each line several times with per-pass overrides, and can
/// record a signal in one pass and play it back in a later one. The controller
/// will not accept a configuration over TCP, only a path to a file it can
/// reach, which is why [`apply`](Self::apply) takes two paths.
pub struct MultiPass<'r, 'a> {
    pub(crate) rt: &'r mut Rt<'a>,
}

impl MultiPass<'_, '_> {
    /// Write `config`, load it, and switch multi-pass on.
    ///
    /// `local_path` is where we write the file; `host_path` is where the
    /// controller looks for it. They differ whenever the controller is not on
    /// this machine.
    pub fn apply(
        &mut self,
        config: &MultiPassConfig,
        local_path: impl Into<PathBuf>,
        host_path: impl Into<String>,
    ) -> Result<()> {
        self.rt.exec(&ApplyMultiPass {
            config: config.clone(),
            local_path: local_path.into(),
            host_path: host_path.into(),
        })?;
        Ok(())
    }

    /// Load a configuration the controller can already reach. An empty path
    /// loads the one held in the session settings file.
    pub fn load(&mut self, host_path: impl Into<String>) -> Result<()> {
        self.rt.exec(&LoadMultiPass {
            host_path: host_path.into(),
        })?;
        Ok(())
    }

    /// Save the active configuration to a path the controller can reach.
    pub fn save(&mut self, host_path: impl Into<String>) -> Result<()> {
        self.rt.exec(&SaveMultiPass {
            host_path: host_path.into(),
        })?;
        Ok(())
    }

    /// Switch multi-pass on or off. Switching it on stops a running scan.
    pub fn activate(&mut self, on: bool) -> Result<()> {
        self.rt.exec(&ActivateMultiPass { on })?;
        Ok(())
    }
}
