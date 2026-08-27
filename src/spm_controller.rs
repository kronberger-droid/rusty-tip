use std::time::Duration;

pub use nanonis_rs::z_ctrl::{ZControllerStatus, ZHomeMode};

use nanonis_rs::{
    Position,
    motor::{MotorDirection, MotorDisplacement, MovementMode, Position3D},
    oscilloscope::{OsciData, TriggerConfig},
    scan::{ScanAction, ScanConfig, ScanDirection, ScanProps, ScanPropsBuilder},
    tcplog::TCPLogStatus,
    tip_recovery::TipShaperConfig,
};

use std::collections::HashSet;

use crate::signal_registry::SignalIndex;
use crate::spm_error::SpmError;

pub type DataStreamStatus = TCPLogStatus;
pub type Result<T> = std::result::Result<T, SpmError>;

/// Oscilloscope trigger configuration (level, slope, hysteresis)
pub type TriggerSetup = TriggerConfig;

/// Solve for the compensation velocity that cancels the drift.
///
/// Given the drift `s0` measured at compensation velocity `v0`, and `s1`
/// measured at `v1`, the drift responds to the velocity as
/// `s(v) = s0 + response * (v - v0)`, and this returns the `v` where that
/// reaches zero. Deliberately not assuming what `response` is: its sign is the
/// undocumented convention we are trying to avoid guessing at, and its
/// magnitude covers a channel that is scaled rather than one-to-one.
///
/// `None` when the drift did not respond to the change, which means the
/// compensation is off, saturated, or otherwise not connected to anything.
fn solve_compensation(v0: f64, s0: f64, v1: f64, s1: f64) -> Option<f64> {
    let response = (s1 - s0) / (v1 - v0);
    // A response far below 1 cannot be a working compensation channel: the
    // velocity is in the same units as the drift, so any sane channel is close
    // to plus or minus one.
    if !response.is_finite() || response.abs() < 0.1 {
        return None;
    }
    Some(v0 - s0 / response)
}

/// Piezo drift compensation: constant velocities the real-time system adds to
/// each axis, plus whether any axis has run out of range.
///
/// The velocities are metres per second. `saturation_limit_percent` is a
/// **percentage of full piezo range**, not a fraction, so 10.0 means 10%.
///
/// Saturation latches. When an axis reaches the limit the controller stops
/// compensating that axis and leaves it stopped; nothing recovers it except
/// switching compensation off and on again. So a `*_saturated` flag does not
/// mean "about to stop", it means "already stopped, for an unknown length of
/// time".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriftComp {
    pub enabled: bool,
    /// Compensation velocity along X, in m/s.
    pub vx: f64,
    /// Compensation velocity along Y, in m/s.
    pub vy: f64,
    /// Compensation velocity along Z, in m/s.
    pub vz: f64,
    /// Saturation limit, as a percentage of full piezo range.
    pub saturation_limit_percent: f64,
    pub x_saturated: bool,
    pub y_saturated: bool,
    pub z_saturated: bool,
}

/// Which signals a scan records, and at what resolution.
///
/// The channels are RT signal slots, the same 0..=127 numbering
/// [`SignalIndex`] carries everywhere else. `pixels` is coerced by the
/// controller to the nearest multiple of 16, since scan data reaches the host
/// in packets of 16.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanBuffer {
    pub channels: Vec<SignalIndex>,
    pub pixels: i32,
    pub lines: i32,
}

/// Hardware capability that a controller may or may not support.
///
/// Actions declare which capabilities they require via `Action::requires()`.
/// The execution layer can check `SpmController::capabilities()` before
/// running an action to give a clear error instead of a runtime failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Signal reading (read_signal, read_signals, signal_names)
    Signals,
    /// Bias voltage control (get_bias, set_bias, bias_pulse)
    Bias,
    /// Z-controller operations (withdraw, auto_approach, set_z_setpoint)
    ZController,
    /// Piezo fine positioning (get_position, set_position)
    PiezoPosition,
    /// Coarse motor positioning (move_motor, move_motor_3d, etc.)
    Motor,
    /// Scan control (scan_action, scan_status)
    Scanning,
    /// Oscilloscope data acquisition (osci_read)
    Oscilloscope,
    /// Tip shaper / tip conditioning (tip_shaper)
    TipShaper,
    /// Phase-locked loop (pll_center_freq_shift)
    Pll,
    /// High-throughput data streaming (data_stream_*)
    DataStream,
    /// Tip-crash protection (safe_tip_configure, safe_tip_status)
    SafeTip,
    /// Multi-pass scanning (multi_pass_load, multi_pass_activate)
    MultiPass,
    /// Piezo drift compensation (drift_comp_get, drift_comp_set)
    DriftCompensation,
}

/// What data the oscilloscope should return
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionMode {
    /// Return current buffer contents immediately
    Current,
    /// Wait for the next trigger event, then return
    NextTrigger,
    /// Wait for two trigger events, then return
    WaitTwoTriggers,
}

pub trait SpmController: Send {
    /// Report which capabilities this controller supports.
    fn capabilities(&self) -> HashSet<Capability>;

    // -- Lifecycle --

    /// One-time hardware setup: load configuration, set safe operating
    /// defaults, apply vendor-specific workarounds.  Called once before
    /// the main experiment loop.  Default is a no-op.
    fn prepare(&mut self) -> Result<()> {
        Ok(())
    }

    /// Best-effort resource cleanup: stop data streams, disable safety
    /// overrides, release hardware locks.  Implementations should log
    /// errors internally rather than propagating, since teardown must
    /// not short-circuit.  Default is a no-op.
    fn teardown(&mut self) {}

    /// Returns `true` if the connection is healthy and commands can be sent.
    ///
    /// A return value of `false` indicates the connection was poisoned by a
    /// previous I/O error and [`reconnect()`](Self::reconnect) must be called
    /// before further use.
    fn is_connected(&self) -> bool {
        true
    }

    /// Re-establish the connection to the hardware controller.
    ///
    /// Call this after an I/O error has poisoned the connection. The default
    /// implementation is a no-op (always succeeds), suitable for mock
    /// controllers in tests.
    fn reconnect(&mut self) -> Result<()> {
        Ok(())
    }

    // -- Signals --
    fn read_signal(&mut self, index: SignalIndex, wait_for_newest: bool) -> Result<f64>;
    fn read_signals(&mut self, indices: &[SignalIndex], wait_for_newest: bool) -> Result<Vec<f64>>;
    fn signal_names(&mut self) -> Result<Vec<String>>;

    // -- Bias --
    fn get_bias(&mut self) -> Result<f64>;
    fn set_bias(&mut self, voltage: f64) -> Result<()>;
    fn bias_pulse(
        &mut self,
        voltage: f64,
        width: Duration,
        z_hold: bool,
        absolute: bool,
    ) -> Result<()>;

    // -- Z-Controller --
    fn withdraw(&mut self, wait: bool, timeout: Duration) -> Result<()>;
    fn auto_approach(&mut self, wait: bool, timeout: Duration) -> Result<()>;
    fn set_z_setpoint(&mut self, setpoint: f64) -> Result<()>;
    fn set_z_home(&mut self, mode: ZHomeMode, position: f64) -> Result<()>;
    /// Move the tip to the configured z-home position.
    fn go_z_home(&mut self) -> Result<()>;
    /// Query the current z-controller status.
    fn z_controller_status(&mut self) -> Result<ZControllerStatus>;

    // -- Piezo Positioning (FolMe) --
    fn get_position(&mut self, wait_for_newest: bool) -> Result<Position>;
    fn set_position(&mut self, pos: Position, wait: bool) -> Result<()>;

    // -- Motor (Coarse Positioning) --
    fn move_motor(&mut self, direction: MotorDirection, steps: u16, wait: bool) -> Result<()>;
    fn move_motor_3d(&mut self, displacement: MotorDisplacement, wait: bool) -> Result<()>;
    fn move_motor_closed_loop(&mut self, target: Position3D, mode: MovementMode) -> Result<()>;
    fn stop_motor(&mut self) -> Result<()>;

    // -- Scanning --
    fn scan_action(&mut self, action: ScanAction, direction: ScanDirection) -> Result<()>;
    fn scan_status(&mut self) -> Result<bool>;
    fn scan_props_get(&mut self) -> Result<ScanProps>;
    fn scan_props_set(&mut self, props: ScanPropsBuilder) -> Result<()>;
    fn scan_speed_get(&mut self) -> Result<ScanConfig>;
    fn scan_speed_set(&mut self, config: ScanConfig) -> Result<()>;

    /// Which signals the scan records, and the frame resolution.
    fn scan_buffer_get(&mut self) -> Result<ScanBuffer>;

    /// Set the recorded signals and the frame resolution.
    fn scan_buffer_set(&mut self, buffer: &ScanBuffer) -> Result<()>;

    /// Add `channels` to the scan buffer, keeping whatever is already there.
    ///
    /// A signal that is not in the buffer is not acquired, however it is
    /// configured elsewhere. Multi-pass in particular records and plays a
    /// signal through its own buffers, which says nothing about whether the
    /// resulting `[P1]`/`[P2]` frames are saved, so anything worth looking at
    /// afterwards has to be in here too.
    fn scan_buffer_ensure(&mut self, channels: &[SignalIndex]) -> Result<()> {
        let mut buffer = self.scan_buffer_get()?;
        let missing: Vec<SignalIndex> = channels
            .iter()
            .filter(|c| !buffer.channels.contains(c))
            .copied()
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        buffer.channels.extend(missing);
        self.scan_buffer_set(&buffer)
    }

    /// Grab pixel data from a completed (or in-progress) scan frame.
    ///
    /// Returns `(channel_name, data_2d, scan_direction_up)` where `data_2d`
    /// is a row-major `Vec<Vec<f32>>` (rows x cols).
    ///
    /// - `channel_index`: which scan buffer channel to read (0-based)
    /// - `forward`: `true` for the forward scan direction, `false` for backward
    fn scan_frame_data_grab(
        &mut self,
        channel_index: u32,
        forward: bool,
    ) -> Result<(String, Vec<Vec<f32>>, bool)>;

    // -- Drift compensation --

    /// Current drift compensation velocities and saturation state.
    fn drift_comp_get(&mut self) -> Result<DriftComp>;

    /// Set the drift compensation velocities. The `*_saturated` fields are
    /// read-only status and are ignored here.
    fn drift_comp_set(&mut self, comp: &DriftComp) -> Result<()>;

    /// Measure how fast Z is drifting, in metres per second.
    ///
    /// Takes `samples` readings of `z` spread over `window`, and fits a line.
    /// The Z controller has to be **on**: with the feedback open, Z is whatever
    /// it was parked at and the drift being measured is invisible. This is also
    /// why drift can only be measured between passes, never during one.
    ///
    /// Nothing here decides where the tip is. A drift estimate only means
    /// something on flat, quiet ground, so position the tip first; measured
    /// over a step edge or a molecule this returns a confident, wrong number.
    fn measure_z_drift(&mut self, z: SignalIndex, window: Duration, samples: usize) -> Result<f64> {
        if samples < 3 {
            return Err(SpmError::Workflow(
                "measure_z_drift: need at least 3 samples to fit a line".into(),
            ));
        }
        if self.z_controller_status()? != ZControllerStatus::On {
            return Err(SpmError::Workflow(
                "measure_z_drift: the Z controller is off, so Z cannot follow the drift".into(),
            ));
        }

        let interval = window / (samples as u32 - 1);
        let start = std::time::Instant::now();
        let mut values = Vec::with_capacity(samples);
        for i in 0..samples {
            values.push(self.read_signal(z, true)?);
            if i + 1 < samples {
                std::thread::sleep(interval);
            }
        }
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed <= 0.0 {
            return Err(SpmError::Workflow(
                "measure_z_drift: no time passed between the first and last sample".into(),
            ));
        }

        // `compute_stability_metrics` fits per sample, and the samples are
        // evenly spaced by construction, so the measured elapsed time converts
        // it to per second. Using the real elapsed time rather than a nominal
        // rate keeps TCP jitter out of the answer.
        let (_, _, slope_per_sample) = crate::action::signals::compute_stability_metrics(&values);
        Ok(slope_per_sample * (samples as f64 - 1.0) / elapsed)
    }

    /// Measure the Z drift and leave the controller compensating for it.
    ///
    /// Returns the residual drift, i.e. what is left after compensation, so a
    /// caller can tell a good correction from a useless one without knowing
    /// how any of this works. Costs three measurement windows.
    ///
    /// Three things this handles that a bare [`Self::drift_comp_set`] does not:
    ///
    /// - **A latched axis is re-armed first.** Compensation stops for good at
    ///   the saturation limit, so measuring against a stopped axis would fit a
    ///   drift nothing is correcting and then write into a channel that
    ///   ignores it.
    /// - **The sign is measured, not assumed.** Which way a positive `vz`
    ///   moves the piezo is not documented anywhere we can check, and guessing
    ///   wrong does not leave the drift uncorrected, it doubles it. So this
    ///   applies a trial velocity, watches how the drift responds, and solves
    ///   for the velocity that cancels it. A compensation channel that does
    ///   not respond at all is an error rather than a coin flip.
    /// - **Positioning is the caller's problem**, as with [`Self::measure_z_drift`].
    fn compensate_drift(
        &mut self,
        z: SignalIndex,
        window: Duration,
        samples: usize,
    ) -> Result<f64> {
        let mut comp = self.drift_comp_get()?;
        if comp.z_saturated {
            self.drift_comp_set(&DriftComp {
                enabled: false,
                ..comp
            })?;
            self.drift_comp_set(&DriftComp {
                enabled: true,
                ..comp
            })?;
            comp = self.drift_comp_get()?;
        }

        let v0 = comp.vz;
        let s0 = self.measure_z_drift(z, window, samples)?;

        // The trial velocity doubles as the first guess: if the sign convention
        // happens to run the way we would guess, this already cancels the
        // drift and the solve below simply confirms it.
        let v1 = v0 - s0;
        self.drift_comp_set(&DriftComp {
            enabled: true,
            vz: v1,
            ..comp
        })?;
        let s1 = self.measure_z_drift(z, window, samples)?;

        let vz = solve_compensation(v0, s0, v1, s1).ok_or_else(|| {
            SpmError::Workflow(format!(
                "compensate_drift: changing vz from {v0} to {v1} m/s did not change the \
                 measured drift ({s0} then {s1} m/s), so the Z compensation is not \
                 responding. Check that it is enabled and not saturated."
            ))
        })?;

        self.drift_comp_set(&DriftComp {
            enabled: true,
            vz,
            ..comp
        })?;
        self.measure_z_drift(z, window, samples)
    }

    // -- Multi-pass --

    /// Load a `.mpas` multi-pass configuration on the controller.
    ///
    /// `host_path` is resolved by the *controller*, not by us: on a real
    /// instrument the Nanonis software runs on its own PC, and the file has to
    /// exist on that machine's filesystem or on a share it can reach. An empty
    /// path loads the configuration held in the session settings file, if there
    /// is one.
    fn multi_pass_load(&mut self, host_path: &str) -> Result<()>;

    /// Save the controller's active multi-pass configuration to `host_path`,
    /// again resolved on the controller's side. An empty path saves into the
    /// session settings file rather than a `.mpas`.
    fn multi_pass_save(&mut self, host_path: &str) -> Result<()>;

    /// Switch multi-pass scanning on or off.
    ///
    /// Activating stops a running scan, so call this before starting one. That
    /// is the Multi-Pass module manual's behaviour, not something the TCP
    /// protocol reference mentions; it has not been checked on hardware.
    /// Note that the scan mode (Normal vs Linefeed) is *not* part of what the
    /// configuration carries and cannot be set over TCP at all; Linefeed, the
    /// mode that keeps every pass on the same line, has to be ticked by hand
    /// in the Scan Control module.
    fn multi_pass_activate(&mut self, on: bool) -> Result<()>;

    // -- Oscilloscope --
    // Combines channel set + trigger config + run + data get
    fn osci_read(
        &mut self,
        channel: i32,
        trigger: Option<&TriggerSetup>,
        mode: AcquisitionMode,
    ) -> Result<OsciData>;

    // -- Tip Shaper --
    // Combines props_set + start
    fn tip_shaper(&mut self, config: &TipShaperConfig, wait: bool, timeout: Duration)
    -> Result<()>;

    // -- PLL --
    fn pll_center_freq_shift(&mut self) -> Result<()>;

    // -- Safe Tip --
    fn safe_tip_configure(
        &mut self,
        auto_recovery: bool,
        auto_pause_scan: bool,
        threshold: f64,
    ) -> Result<()>;
    fn safe_tip_status(&mut self) -> Result<(bool, bool, f64)>;
    /// Enable or disable the safe-tip crash protection.
    fn safe_tip_set_enabled(&mut self, enabled: bool) -> Result<()>;
    /// Check whether safe-tip crash protection is currently enabled.
    fn safe_tip_enabled(&mut self) -> Result<bool>;

    // -- TCP Logger --
    fn data_stream_configure(&mut self, channels: &[i32], oversampling: i32) -> Result<()>;
    fn data_stream_start(&mut self) -> Result<()>;
    fn data_stream_stop(&mut self) -> Result<()>;
    fn data_stream_status(&mut self) -> Result<DataStreamStatus>;

    /// Discard any buffered data samples so the next read_stable_signal
    /// returns only fresh post-operation data.  Default is a no-op for
    /// controllers without internal buffering.
    fn clear_data_buffer(&mut self) {}

    // -- Signal Reading --

    /// Collect raw signal samples for analysis or averaging.
    ///
    /// Returns up to `num_samples` data points for the given signal.
    /// The default implementation polls `read_signal` in a loop, which works
    /// but is slow and subject to aliasing.  Implementations with access to a
    /// high-throughput data stream (e.g. Nanonis TCP logger) should override
    /// this to collect frames from the stream instead.
    ///
    /// `index` is the same signal index used by `read_signal`.
    fn read_signal_samples(&mut self, index: SignalIndex, num_samples: usize) -> Result<Vec<f64>> {
        if num_samples == 0 {
            return Err(SpmError::Protocol(
                "read_signal_samples: num_samples must be > 0".into(),
            ));
        }
        let mut samples = Vec::with_capacity(num_samples);
        for _ in 0..num_samples {
            samples.push(self.read_signal(index, true)?);
        }
        Ok(samples)
    }

    /// Read a noise-reduced signal value by averaging multiple samples.
    ///
    /// Convenience wrapper around `read_signal_samples` that returns the mean.
    fn read_stable_signal(&mut self, index: SignalIndex, num_samples: usize) -> Result<f64> {
        let samples = self.read_signal_samples(index, num_samples)?;
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        Ok(mean)
    }
}

#[cfg(test)]
mod tests {
    use super::solve_compensation;

    /// 5 pm/s of drift, measured with no compensation applied yet.
    const DRIFT: f64 = 5e-12;

    /// The measured drift after applying `v`, for a channel where a positive
    /// velocity adds to the drift (`sign` = 1) or subtracts from it (-1).
    fn measured(v: f64, sign: f64) -> f64 {
        DRIFT + sign * v
    }

    #[test]
    fn solves_for_either_sign_convention() {
        // The whole point: neither branch knows which convention it is in, and
        // both land on the velocity that zeroes the drift.
        for sign in [1.0, -1.0] {
            let (v0, s0) = (0.0, measured(0.0, sign));
            let v1 = v0 - s0;
            let vz = solve_compensation(v0, s0, v1, measured(v1, sign)).unwrap();
            assert!(
                measured(vz, sign).abs() < 1e-24,
                "sign {sign}: vz {vz} leaves {} m/s",
                measured(vz, sign)
            );
        }
    }

    #[test]
    fn solves_when_compensation_is_already_partly_applied() {
        // Re-running against a machine that is already compensating must not
        // throw away the velocity that is already in effect.
        let sign = -1.0;
        let v0 = 3e-12;
        let s0 = measured(v0, sign);
        let v1 = v0 - s0;
        let vz = solve_compensation(v0, s0, v1, measured(v1, sign)).unwrap();
        assert!((vz - DRIFT).abs() < 1e-24, "expected {DRIFT}, got {vz}");
    }

    #[test]
    fn solves_a_channel_that_is_scaled_rather_than_one_to_one() {
        let scaled = |v: f64| DRIFT - 0.5 * v;
        let (v0, s0) = (0.0, scaled(0.0));
        let v1 = v0 - s0;
        let vz = solve_compensation(v0, s0, v1, scaled(v1)).unwrap();
        assert!(scaled(vz).abs() < 1e-24);
    }

    #[test]
    fn refuses_a_channel_that_does_not_respond() {
        // Saturated, disabled, or otherwise not wired to the piezo: the drift
        // is unchanged by the trial velocity. Dividing by that response would
        // produce a huge confident number.
        assert_eq!(solve_compensation(0.0, DRIFT, -DRIFT, DRIFT), None);
        // And the degenerate case where no trial was actually applied.
        assert_eq!(solve_compensation(0.0, DRIFT, 0.0, DRIFT), None);
    }
}
