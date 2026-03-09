use std::time::Duration;

use nanonis_rs::{
    motor::{MotorDirection, MotorDisplacement, MovementMode, Position3D},
    oscilloscope::{OsciData, TriggerConfig},
    scan::{ScanAction, ScanDirection},
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
}

/// What data the oscilloscope should return
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

    // -- TCP Logger --
    fn data_stream_configure(
        &mut self,
        channels: &[i32],
        oversampling: i32,
    ) -> Result<()>;
    fn data_stream_start(&mut self) -> Result<()>;
    fn data_stream_stop(&mut self) -> Result<()>;
    fn data_stream_status(&mut self) -> Result<DataStreamStatus>;
}
