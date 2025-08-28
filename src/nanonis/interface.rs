use std::time::Duration;

use crate::error::NanonisError;
use crate::types::{
    MotorDirection, MotorGroup, MovementMode, Position, Position3D, ScanAction, ScanDirection,
    StepCount,
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
    fn read_signals(&mut self, indices: Vec<i32>, wait: bool) -> Result<Vec<f32>, NanonisError>;

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
    fn set_xy_position(&mut self, position: Position, wait: bool) -> Result<(), NanonisError>;

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
        steps: StepCount,
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

    /// Start automatic approach sequence
    ///
    /// # Arguments
    /// * `wait` - Whether to wait until approach completes
    fn auto_approach(&mut self, wait: bool) -> Result<(), NanonisError>;

    /// Withdraw the tip from the sample
    ///
    /// # Arguments
    /// * `wait` - Whether to wait until withdrawal completes
    /// * `timeout_ms` - Maximum time to wait in milliseconds
    fn z_ctrl_withdraw(&mut self, wait: bool, timeout_ms: i32) -> Result<(), NanonisError>;

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
}
