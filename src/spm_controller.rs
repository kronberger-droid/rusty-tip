use std::time::Duration;

pub use nanonis_rs::z_ctrl::ZHomeMode;

use nanonis_rs::{
    motor::{MotorDirection, MotorDisplacement, MovementMode, Position3D},
    oscilloscope::{OsciData, TriggerConfig},
    scan::{ScanAction, ScanConfig, ScanDirection, ScanProps, ScanPropsBuilder},
    tcplog::TCPLogStatus,
    tip_recovery::TipShaperConfig,
    Position,
};

use std::collections::HashSet;

use crate::spm_error::SpmError;

pub type DataStreamStatus = TCPLogStatus;
pub type Result<T> = std::result::Result<T, SpmError>;

/// Oscilloscope trigger configuration (level, slope, hysteresis)
pub type TriggerSetup = TriggerConfig;

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
    fn prepare(&mut self) -> Result<()> { Ok(()) }

    /// Best-effort resource cleanup: stop data streams, disable safety
    /// overrides, release hardware locks.  Implementations should log
    /// errors internally rather than propagating, since teardown must
    /// not short-circuit.  Default is a no-op.
    fn teardown(&mut self) {}

    // -- Signals --
    fn read_signal(&mut self, index: u32, wait_for_newest: bool)
        -> Result<f64>;
    fn read_signals(
        &mut self,
        indices: &[u32],
        wait_for_newest: bool,
    ) -> Result<Vec<f64>>;
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

    // -- Piezo Positioning (FolMe) --
    fn get_position(&mut self, wait_for_newest: bool) -> Result<Position>;
    fn set_position(&mut self, pos: Position, wait: bool) -> Result<()>;

    // -- Motor (Coarse Positioning) --
    fn move_motor(
        &mut self,
        direction: MotorDirection,
        steps: u16,
        wait: bool,
    ) -> Result<()>;
    fn move_motor_3d(
        &mut self,
        displacement: MotorDisplacement,
        wait: bool,
    ) -> Result<()>;
    fn move_motor_closed_loop(
        &mut self,
        target: Position3D,
        mode: MovementMode,
    ) -> Result<()>;
    fn stop_motor(&mut self) -> Result<()>;

    // -- Scanning --
    fn scan_action(
        &mut self,
        action: ScanAction,
        direction: ScanDirection,
    ) -> Result<()>;
    fn scan_status(&mut self) -> Result<bool>;
    fn scan_props_get(&mut self) -> Result<ScanProps>;
    fn scan_props_set(&mut self, props: ScanPropsBuilder) -> Result<()>;
    fn scan_speed_get(&mut self) -> Result<ScanConfig>;
    fn scan_speed_set(&mut self, config: ScanConfig) -> Result<()>;

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
    fn tip_shaper(
        &mut self,
        config: &TipShaperConfig,
        wait: bool,
        timeout: Duration,
    ) -> Result<()>;

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
    fn data_stream_configure(
        &mut self,
        channels: &[i32],
        oversampling: i32,
    ) -> Result<()>;
    fn data_stream_start(&mut self) -> Result<()>;
    fn data_stream_stop(&mut self) -> Result<()>;
    fn data_stream_status(&mut self) -> Result<DataStreamStatus>;

    /// Discard any buffered data samples so the next read_stable_signal
    /// returns only fresh post-operation data.  Default is a no-op for
    /// controllers without internal buffering.
    fn clear_data_buffer(&mut self) {}

    // -- Stable Signal Reading --

    /// Read a noise-reduced signal value by averaging multiple samples.
    ///
    /// The default implementation polls `read_signal` in a loop, which works
    /// but is slow and subject to aliasing.  Implementations with access to a
    /// high-throughput data stream (e.g. Nanonis TCP logger) should override
    /// this to collect frames from the stream instead.
    ///
    /// `index` is the same signal index used by `read_signal`.
    fn read_stable_signal(
        &mut self,
        index: u32,
        num_samples: usize,
    ) -> Result<f64> {
        if num_samples == 0 {
            return Err(crate::spm_error::SpmError::Protocol(
                "read_stable_signal: num_samples must be > 0".into(),
            ));
        }
        let mut sum = 0.0;
        for _ in 0..num_samples {
            sum += self.read_signal(index, true)?;
        }
        Ok(sum / num_samples as f64)
    }
}
