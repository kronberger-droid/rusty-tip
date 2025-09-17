use std::time::Duration;

use crate::error::NanonisError;
use crate::types::{
    AutoApproachResult, DataToGet, MotorDirection, MotorGroup, MovementMode, OsciTriggerMode,
    OversamplingIndex, Position, Position3D, ScanAction, ScanDirection,
    TimebaseIndex, TriggerSlope,
};

/// Universal SPM pulse modes - concepts that apply to any SPM system
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PulseMode {
    /// Keep current bias voltage unchanged
    Keep = 0,
    /// Add voltage to current bias (relative)
    Relative = 1,
    /// Set bias to absolute voltage value
    Absolute = 2,
}

impl From<PulseMode> for u16 {
    fn from(mode: PulseMode) -> Self {
        mode as u16
    }
}

/// Universal Z-controller hold states for SPM operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZControllerHold {
    /// Don't modify Z-controller state
    NoChange = 0,
    /// Hold Z-controller during operation
    Hold = 1,
    /// Release/disable Z-controller during operation
    Release = 2,
}

impl From<ZControllerHold> for u16 {
    fn from(hold: ZControllerHold) -> Self {
        hold as u16
    }
}

/// Universal SPM interface defining operations common to all SPM systems
///
/// This trait abstracts core SPM functionality using universal domain concepts
/// rather than system-specific implementation details. Each SPM system
/// (Nanonis, Createc, RHK, etc.) can implement this trait by mapping
/// universal concepts to their specific protocols.
///
/// # Design Philosophy
/// - Use universal SPM concepts (not implementation-specific details)
/// - Self-documenting through type safety
/// - Suitable for any SPM control system
/// - Enable testing through mock implementations
pub trait SPMInterface: Send + Sync {
    // === Signal Operations ===

    /// Read multiple signal values by their indices
    ///
    /// # Arguments
    /// * `indices` - Signal indices to read
    /// * `wait` - Whether to wait for newest data or return immediately
    ///
    /// # Returns
    /// Vector of signal values in the same order as requested indices
    fn read_signals(
        &mut self,
        indices: Vec<i32>,
        wait: bool,
    ) -> Result<Vec<f32>, NanonisError>;

    /// Get available signal names from the SPM system
    ///
    /// # Returns
    /// Vector of available signal names (may be system-dependent)
    fn get_signal_names(&mut self) -> Result<Vec<String>, NanonisError>;

    // === Bias Operations ===

    /// Get the current bias voltage applied to the tip
    ///
    /// # Returns
    /// Current bias voltage in volts
    fn get_bias(&mut self) -> Result<f32, NanonisError>;

    /// Set the bias voltage applied to the tip
    ///
    /// # Arguments
    /// * `voltage` - Bias voltage in volts
    fn set_bias(&mut self, voltage: f32) -> Result<(), NanonisError>;

    /// Apply a bias voltage pulse
    ///
    /// # Arguments
    /// * `wait` - Whether to wait until pulse completes
    /// * `width` - Pulse duration
    /// * `voltage` - Bias voltage during pulse (interpretation depends on mode)
    /// * `hold` - Z-controller behavior during pulse
    /// * `mode` - How to interpret the voltage parameter
    fn bias_pulse(
        &mut self,
        wait: bool,
        width: Duration,
        voltage: f32,
        hold: ZControllerHold,
        mode: PulseMode,
    ) -> Result<(), NanonisError>;

    // === XY Positioning ===

    /// Get current XY position of the tip
    ///
    /// # Arguments
    /// * `wait` - Whether to wait for newest position data
    ///
    /// # Returns
    /// Current XY position in meters
    fn get_xy_position(&mut self, wait: bool) -> Result<Position, NanonisError>;

    /// Set XY position of the tip
    ///
    /// # Arguments
    /// * `position` - Target XY position in meters
    /// * `wait` - Whether to wait until movement completes
    fn set_xy_position(
        &mut self,
        position: Position,
        wait: bool,
    ) -> Result<(), NanonisError>;

    // === Motor Operations (Coarse Positioning) ===

    /// Start motor movement in a specified direction
    ///
    /// # Arguments
    /// * `direction` - Direction to move
    /// * `steps` - Number of steps to take
    /// * `group` - Motor group to move
    /// * `wait` - Whether to wait until movement completes
    fn motor_start_move(
        &mut self,
        direction: MotorDirection,
        steps: u16,
        group: MotorGroup,
        wait: bool,
    ) -> Result<(), NanonisError>;

    /// Start closed-loop motor movement to target position
    ///
    /// # Arguments
    /// * `mode` - Movement mode (relative or absolute)
    /// * `target` - Target 3D position
    /// * `wait` - Whether to wait until movement completes
    /// * `group` - Motor group to move
    fn motor_start_closed_loop(
        &mut self,
        mode: MovementMode,
        target: Position3D,
        wait: bool,
        group: MotorGroup,
    ) -> Result<(), NanonisError>;

    /// Stop any ongoing motor movement
    fn motor_stop_move(&mut self) -> Result<(), NanonisError>;

    // === Control Operations ===

    /// Perform auto-approach operation with timeout and proper error handling
    ///
    /// # Arguments
    /// * `wait` - Whether to wait until approach completes
    /// * `timeout` - Maximum time to wait for completion
    fn auto_approach_with_timeout(&mut self, wait: bool, timeout: Duration) -> Result<AutoApproachResult, NanonisError>;

    /// Start automatic approach sequence (legacy compatibility)
    ///
    /// # Arguments
    /// * `wait` - Whether to wait until approach completes
    fn auto_approach(&mut self, wait: bool) -> Result<(), NanonisError> {
        let result = self.auto_approach_with_timeout(wait, Duration::from_secs(300))?;
        match result {
            AutoApproachResult::Success => Ok(()),
            _ => Err(NanonisError::InvalidCommand(format!(
                "Auto-approach failed: {}",
                result.error_message().unwrap_or("Unknown error")
            ))),
        }
    }

    /// Withdraw the tip from the sample
    ///
    /// # Arguments
    /// * `wait` - Whether to wait until withdrawal completes
    /// * `timeout_ms` - Maximum time to wait in milliseconds
    fn z_ctrl_withdraw(
        &mut self,
        wait: bool,
        timeout_ms: Duration,
    ) -> Result<(), NanonisError>;

    // === Scan Operations ===

    /// Execute a scan control action
    ///
    /// # Arguments
    /// * `action` - Scan action to perform (start, stop, pause, etc.)
    /// * `direction` - Scan direction
    fn scan_action(
        &mut self,
        action: ScanAction,
        direction: ScanDirection,
    ) -> Result<(), NanonisError>;

    /// Get current scanning status
    ///
    /// # Returns
    /// True if currently scanning, false otherwise
    fn scan_status_get(&mut self) -> Result<bool, NanonisError>;

    // === Oscilloscope 1-Channel Operations ===

    /// Set the channel for Oscilloscope 1-Channel
    ///
    /// # Arguments
    /// * `channel_index` - Signal index (0-23)
    fn osci1t_ch_set(&mut self, channel_index: i32) -> Result<(), NanonisError>;

    /// Get the channel for Oscilloscope 1-Channel
    ///
    /// # Returns
    /// Channel index (0-23)
    fn osci1t_ch_get(&mut self) -> Result<i32, NanonisError>;

    /// Set the timebase for Oscilloscope 1-Channel
    ///
    /// # Arguments
    /// * `timebase_index` - Timebase index from available timebases
    fn osci1t_timebase_set(
        &mut self,
        timebase_index: TimebaseIndex,
    ) -> Result<(), NanonisError>;

    /// Get the timebase for Oscilloscope 1-Channel
    ///
    /// # Returns
    /// Tuple of (timebase_index, available_timebases)
    fn osci1t_timebase_get(
        &mut self,
    ) -> Result<(TimebaseIndex, Vec<f32>), NanonisError>;

    /// Set trigger configuration for Oscilloscope 1-Channel
    ///
    /// # Arguments
    /// * `trigger_mode` - Trigger mode (Immediate, Level, Auto)
    /// * `trigger_slope` - Trigger slope (Falling, Rising)
    /// * `trigger_level` - Trigger level value
    /// * `trigger_hysteresis` - Trigger hysteresis value
    fn osci1t_trig_set(
        &mut self,
        trigger_mode: OsciTriggerMode,
        trigger_slope: TriggerSlope,
        trigger_level: f64,
        trigger_hysteresis: f64,
    ) -> Result<(), NanonisError>;

    /// Get trigger configuration for Oscilloscope 1-Channel
    ///
    /// # Returns
    /// Tuple of (trigger_mode, trigger_slope, trigger_level, trigger_hysteresis)
    fn osci1t_trig_get(
        &mut self,
    ) -> Result<(OsciTriggerMode, TriggerSlope, f64, f64), NanonisError>;

    /// Start the Oscilloscope 1-Channel
    fn osci1t_run(&mut self) -> Result<(), NanonisError>;

    /// Get data from Oscilloscope 1-Channel
    ///
    /// # Arguments
    /// * `data_to_get` - Data acquisition mode (Current, NextTrigger, Wait2Triggers)
    ///
    /// # Returns
    /// Tuple of (t0, dt, size, data_values)
    fn osci1t_data_get(
        &mut self,
        data_to_get: DataToGet,
    ) -> Result<(f64, f64, i32, Vec<f64>), NanonisError>;

    // === Oscilloscope 2-Channels Operations ===

    /// Set channels for Oscilloscope 2-Channels
    ///
    /// # Arguments
    /// * `channel_a_index` - Channel A signal index (0-23)
    /// * `channel_b_index` - Channel B signal index (0-23)
    fn osci2t_ch_set(
        &mut self,
        channel_a_index: i32,
        channel_b_index: i32,
    ) -> Result<(), NanonisError>;

    /// Get channels for Oscilloscope 2-Channels
    ///
    /// # Returns
    /// Tuple of (channel_a_index, channel_b_index)
    fn osci2t_ch_get(&mut self) -> Result<(i32, i32), NanonisError>;

    /// Set timebase for Oscilloscope 2-Channels
    ///
    /// # Arguments
    /// * `timebase_index` - Timebase index from available timebases
    fn osci2t_timebase_set(
        &mut self,
        timebase_index: TimebaseIndex,
    ) -> Result<(), NanonisError>;

    /// Get timebase for Oscilloscope 2-Channels
    ///
    /// # Returns
    /// Tuple of (timebase_index, available_timebases)
    fn osci2t_timebase_get(
        &mut self,
    ) -> Result<(TimebaseIndex, Vec<f32>), NanonisError>;

    /// Set oversampling for Oscilloscope 2-Channels
    ///
    /// # Arguments
    /// * `oversampling_index` - Oversampling configuration
    fn osci2t_oversampl_set(
        &mut self,
        oversampling_index: OversamplingIndex,
    ) -> Result<(), NanonisError>;

    /// Get oversampling for Oscilloscope 2-Channels
    ///
    /// # Returns
    /// Current oversampling configuration
    fn osci2t_oversampl_get(&mut self) -> Result<OversamplingIndex, NanonisError>;

    /// Set trigger configuration for Oscilloscope 2-Channels
    ///
    /// # Arguments
    /// * `trigger_mode` - Trigger mode (Immediate, Level, Auto)
    /// * `trig_channel` - Trigger channel
    /// * `trigger_slope` - Trigger slope (Falling, Rising)
    /// * `trigger_level` - Trigger level value
    /// * `trigger_hysteresis` - Trigger hysteresis value
    /// * `trig_position` - Trigger position
    fn osci2t_trig_set(
        &mut self,
        trigger_mode: OsciTriggerMode,
        trig_channel: u16,
        trigger_slope: TriggerSlope,
        trigger_level: f64,
        trigger_hysteresis: f64,
        trig_position: f64,
    ) -> Result<(), NanonisError>;

    /// Get trigger configuration for Oscilloscope 2-Channels
    ///
    /// # Returns
    /// Tuple of (trigger_mode, trig_channel, trigger_slope, trigger_level, trigger_hysteresis, trig_position)
    fn osci2t_trig_get(
        &mut self,
    ) -> Result<(OsciTriggerMode, u16, TriggerSlope, f64, f64, f64), NanonisError>;

    /// Start the Oscilloscope 2-Channels
    fn osci2t_run(&mut self) -> Result<(), NanonisError>;

    /// Get data from Oscilloscope 2-Channels
    ///
    /// # Arguments
    /// * `data_to_get` - Data acquisition mode (Current, NextTrigger, Wait2Triggers)
    ///
    /// # Returns
    /// Tuple of (t0, dt, channel_a_data, channel_b_data)
    fn osci2t_data_get(
        &mut self,
        data_to_get: DataToGet,
    ) -> Result<(f64, f64, Vec<f64>, Vec<f64>), NanonisError>;
}
