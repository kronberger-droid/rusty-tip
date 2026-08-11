//! Capability-checked subsystem handles handed out by [`Rt`].
//!
//! Each handle is a thin, short-lived view: its methods delegate to the
//! library's action implementations (so every operation emits
//! started/completed/failed events) or, for pure read/restore plumbing,
//! straight to the controller. Fetch a handle per statement:
//! `rt.bias()?.pulse(4.0, 50)?`.

use nanonis_rs::scan::{ScanConfig, ScanProps, ScanPropsBuilder};

use crate::action::bias::{BiasPulse, ReadBias, SetBias};
use crate::action::motor::{MoveMotor3D, Reposition};
use crate::action::scan::{ScanActionParam, ScanControl, ScanDirectionParam};
use crate::action::signals::{ReadSignal, ReadStableSignal};
use crate::action::z_controller::{AutoApproach, CalibratedApproach, SetZSetpoint, Withdraw};
use crate::signal_registry::SignalIndex;
use crate::spm_error::SpmError;

use super::Rt;

type Result<T> = std::result::Result<T, SpmError>;

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
        self.rt.exec(&ReadBias)
    }

    /// Set the bias voltage in volts.
    pub fn set(&mut self, voltage: f64) -> Result<()> {
        self.rt.exec(&SetBias { voltage })?;
        Ok(())
    }

    /// Fire a bias pulse (signed voltage, width in ms) with the z-controller
    /// held, relative to the current bias.
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
    /// center the frequency shift, re-approach.
    pub fn calibrated_approach(&mut self) -> Result<()> {
        let check_safe_tip = self.rt.safe_tip_guard();
        self.rt.exec(&CalibratedApproach {
            check_safe_tip,
            ..Default::default()
        })?;
        Ok(())
    }

    /// The same sequence with an explicit timeout, for the first approach of
    /// a run: it starts from an unknown coarse position, so it can take far
    /// longer than one that begins from a known height.
    pub fn calibrated_approach_within(&mut self, timeout_ms: u64) -> Result<()> {
        let check_safe_tip = self.rt.safe_tip_guard();
        self.rt.exec(&CalibratedApproach {
            timeout_ms,
            check_safe_tip,
            ..Default::default()
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
        self.rt.exec(&ReadSignal {
            index,
            wait_for_newest: true,
        })
    }

    /// Read a noise- and drift-gated signal value (see [`StableReadSpec`]).
    /// Emits one `stable_read` measurement event per accepted batch.
    pub fn read_stable(&mut self, index: SignalIndex, spec: &StableReadSpec) -> Result<f64> {
        self.rt.exec(&ReadStableSignal {
            index,
            num_samples: spec.num_samples,
            max_std_dev: spec.max_std_dev,
            max_slope: spec.max_slope,
            max_retries: spec.max_retries,
            sample_rate_hz: spec.sample_rate_hz,
        })
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
}

impl Default for RepositionSpec {
    fn default() -> Self {
        Self {
            x_steps: 0,
            y_steps: 0,
            z_retract: -3,
            post_move_settle_ms: 500,
            post_approach_settle_ms: 500,
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
        let check_safe_tip = self.rt.safe_tip_guard();
        self.rt.exec(&Reposition {
            check_safe_tip,
            x_steps: spec.x_steps,
            y_steps: spec.y_steps,
            z_retract: spec.z_retract,
            post_move_settle_ms: spec.post_move_settle_ms,
            post_approach_settle_ms: spec.post_approach_settle_ms,
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
