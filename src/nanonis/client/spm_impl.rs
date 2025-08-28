use std::time::Duration;

use super::NanonisClient;
use crate::error::NanonisError;
use crate::nanonis::interface::{PulseMode, SPMInterface, ZControllerHold};
use crate::types::{
    MotorDirection, MotorGroup, MovementMode, Position, Position3D, ScanAction, ScanDirection,
    StepCount,
};

/// Implementation of SPMInterface for NanonisClient
/// 
/// This implementation maps universal SPM concepts to Nanonis-specific protocol commands.
/// The mapping handles type conversions and maintains the semantic meaning of operations
/// while adapting to Nanonis protocol requirements.
impl SPMInterface for NanonisClient {
    // === Signal Operations ===

    fn read_signals(&mut self, indices: Vec<i32>, wait: bool) -> Result<Vec<f32>, NanonisError> {
        self.signals_vals_get(indices, wait)
    }

    fn get_signal_names(&mut self) -> Result<Vec<String>, NanonisError> {
        self.signal_names_get(false)
    }

    // === Bias Operations ===

    fn get_bias(&mut self) -> Result<f32, NanonisError> {
        // Forward to the actual NanonisClient method (same name, so need explicit call)
        NanonisClient::get_bias(self)
    }

    fn set_bias(&mut self, voltage: f32) -> Result<(), NanonisError> {
        // Forward to the actual NanonisClient method (same name, so need explicit call)
        NanonisClient::set_bias(self, voltage)
    }

    fn bias_pulse(
        &mut self,
        wait: bool,
        width: Duration,
        voltage: f32,
        hold: ZControllerHold,
        mode: PulseMode,
    ) -> Result<(), NanonisError> {
        // Convert universal types to Nanonis-specific values
        let width_seconds = width.as_secs_f32();
        let nanonis_hold: u16 = hold.into();
        let nanonis_mode: u16 = mode.into();
        
        self.bias_pulse(wait, width_seconds, voltage, nanonis_hold, nanonis_mode)
    }

    // === XY Positioning ===

    fn get_xy_position(&mut self, wait: bool) -> Result<Position, NanonisError> {
        self.folme_xy_pos_get(wait)
    }

    fn set_xy_position(&mut self, position: Position, wait: bool) -> Result<(), NanonisError> {
        self.folme_xy_pos_set(position, wait)
    }

    // === Motor Operations (Coarse Positioning) ===

    fn motor_start_move(
        &mut self,
        direction: MotorDirection,
        steps: StepCount,
        group: MotorGroup,
        wait: bool,
    ) -> Result<(), NanonisError> {
        NanonisClient::motor_start_move(self, direction, steps, group, wait)
    }

    fn motor_start_closed_loop(
        &mut self,
        mode: MovementMode,
        target: Position3D,
        wait: bool,
        group: MotorGroup,
    ) -> Result<(), NanonisError> {
        NanonisClient::motor_start_closed_loop(self, mode, target, wait, group)
    }

    fn motor_stop_move(&mut self) -> Result<(), NanonisError> {
        NanonisClient::motor_stop_move(self)
    }

    // === Control Operations ===

    fn auto_approach(&mut self, wait: bool) -> Result<(), NanonisError> {
        if wait {
            self.auto_approach_and_wait()
        } else {
            // For non-waiting approach, we need to implement the basic approach
            // This would typically be auto_approach_open + auto_approach_on_off_set
            // For now, fallback to waiting approach
            self.auto_approach_and_wait()
        }
    }

    fn z_ctrl_withdraw(&mut self, wait: bool, timeout_ms: i32) -> Result<(), NanonisError> {
        NanonisClient::z_ctrl_withdraw(self, wait, timeout_ms)
    }

    // === Scan Operations ===

    fn scan_action(
        &mut self,
        action: ScanAction,
        direction: ScanDirection,
    ) -> Result<(), NanonisError> {
        NanonisClient::scan_action(self, action, direction)
    }

    fn scan_status_get(&mut self) -> Result<bool, NanonisError> {
        NanonisClient::scan_status_get(self)
    }
}