use std::time::Duration;

pub use nanonis_rs::z_ctrl::{ZControllerStatus, ZHomeMode};

use nanonis_rs::{
    Position,
    motor::{MotorDirection, MotorDisplacement, MovementMode, Position3D},
    oscilloscope::{OsciData, TriggerConfig},
    scan::{ScanAction, ScanConfig, ScanDirection, ScanLineEnd, ScanProps, ScanPropsBuilder},
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
    fn scan_buffer_ensure(&mut self, channels: &[SignalIndex]) -> Result<ScanBuffer> {
        let mut buffer = self.scan_buffer_get()?;
        let missing: Vec<SignalIndex> = channels
            .iter()
            .filter(|c| !buffer.channels.contains(c))
            .copied()
            .collect();
        if missing.is_empty() {
            return Ok(buffer);
        }
        buffer.channels.extend(missing);
        self.scan_buffer_set(&buffer)?;
        Ok(buffer)
    }

    /// Block until the scan finishes a line, or until `timeout` elapses.
    ///
    /// The returned [`ScanLineEnd`] reports the line number, what the head was
    /// doing, and the multi-pass pass number. Direction and pass are separate
    /// fields, which is worth noting: a `[PassN]` section in a `.mpas` file is
    /// one *direction* of a pass, so the two numbering schemes here are not
    /// the same thing.
    ///
    /// Check `timed_out` before trusting the rest. A timeout returns normally
    /// with stale line and pass numbers rather than an error.
    fn scan_wait_end_of_line(&mut self, timeout: Duration) -> Result<ScanLineEnd>;

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
