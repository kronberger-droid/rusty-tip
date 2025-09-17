use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::{
    Amplitude, Frequency, MotorAxis, MotorDirection, MotorGroup, MovementMode,
    NanonisValue, Position3D,
};
use std::time::Duration;

impl NanonisClient {
    /// Move the coarse positioning device (motor, piezo actuator)
    pub fn motor_start_move(
        &mut self,
        direction: impl Into<MotorDirection>,
        number_of_steps: impl Into<u16>,
        group: impl Into<MotorGroup>,
        wait_until_finished: bool,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u32 } else { 0u32 };
        self.quick_send(
            "Motor.StartMove",
            vec![
                NanonisValue::U32(direction.into().into()),
                NanonisValue::U16(number_of_steps.into()),
                NanonisValue::U32(group.into().into()),
                NanonisValue::U32(wait_flag),
            ],
            vec!["I", "H", "I", "I"],
            vec![],
        )?;
        Ok(())
    }

    /// Move the coarse positioning device in closed loop
    pub fn motor_start_closed_loop(
        &mut self,
        movement_mode: MovementMode,
        target_position: Position3D,
        wait_until_finished: bool,
        group: MotorGroup,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u32 } else { 0u32 };
        self.quick_send(
            "Motor.StartClosedLoop",
            vec![
                NanonisValue::U32(movement_mode.into()),
                NanonisValue::F64(target_position.x),
                NanonisValue::F64(target_position.y),
                NanonisValue::F64(target_position.z),
                NanonisValue::U32(wait_flag),
                NanonisValue::U32(group.into()),
            ],
            vec!["I", "d", "d", "d", "I", "I"],
            vec![],
        )?;
        Ok(())
    }

    /// Stop the motor motion
    pub fn motor_stop_move(&mut self) -> Result<(), NanonisError> {
        self.quick_send("Motor.StopMove", vec![], vec![], vec![])?;
        Ok(())
    }

    /// Get the positions of the motor control module
    pub fn motor_pos_get(
        &mut self,
        group: MotorGroup,
        timeout: Duration,
    ) -> Result<Position3D, NanonisError> {
        let result = self.quick_send(
            "Motor.PosGet",
            vec![
                NanonisValue::U32(group.into()),
                NanonisValue::U32(timeout.as_millis() as u32),
            ],
            vec!["I", "I"],
            vec!["d", "d", "d"],
        )?;

        if result.len() >= 3 {
            let x = result[0].as_f64()?;
            let y = result[1].as_f64()?;
            let z = result[2].as_f64()?;
            Ok(Position3D::new(x, y, z))
        } else {
            Err(NanonisError::Protocol(
                "Invalid motor position response".to_string(),
            ))
        }
    }

    /// Get step counter values and optionally reset them
    /// Available only on Attocube ANC150 devices
    pub fn motor_step_counter_get(
        &mut self,
        reset_x: bool,
        reset_y: bool,
        reset_z: bool,
    ) -> Result<(i32, i32, i32), NanonisError> {
        let reset_x_flag = if reset_x { 1u32 } else { 0u32 };
        let reset_y_flag = if reset_y { 1u32 } else { 0u32 };
        let reset_z_flag = if reset_z { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "Motor.StepCounterGet",
            vec![
                NanonisValue::U32(reset_x_flag),
                NanonisValue::U32(reset_y_flag),
                NanonisValue::U32(reset_z_flag),
            ],
            vec!["I", "I", "I"],
            vec!["i", "i", "i"],
        )?;

        if result.len() >= 3 {
            let step_x = result[0].as_i32()?;
            let step_y = result[1].as_i32()?;
            let step_z = result[2].as_i32()?;
            Ok((step_x, step_y, step_z))
        } else {
            Err(NanonisError::Protocol(
                "Invalid step counter response".to_string(),
            ))
        }
    }

    /// Get frequency and amplitude of the motor control module
    /// Available only for PD5, PMD4, and Attocube ANC150 devices
    pub fn motor_freq_amp_get(
        &mut self,
        axis: MotorAxis,
    ) -> Result<(Frequency, Amplitude), NanonisError> {
        let result = self.quick_send(
            "Motor.FreqAmpGet",
            vec![NanonisValue::U16(axis.into())],
            vec!["H"],
            vec!["f", "f"],
        )?;

        if result.len() >= 2 {
            let frequency = Frequency::hz(result[0].as_f32()?);
            let amplitude = Amplitude::volts(result[1].as_f32()?);
            Ok((frequency, amplitude))
        } else {
            Err(NanonisError::Protocol(
                "Invalid frequency/amplitude response".to_string(),
            ))
        }
    }

    /// Set frequency and amplitude of the motor control module
    /// Available only for PD5, PMD4, and Attocube ANC150 devices
    pub fn motor_freq_amp_set(
        &mut self,
        frequency: impl Into<Frequency>,
        amplitude: impl Into<Amplitude>,
        axis: impl Into<MotorAxis>,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Motor.FreqAmpSet",
            vec![
                NanonisValue::F32(frequency.into().into()),
                NanonisValue::F32(amplitude.into().into()),
                NanonisValue::U16(axis.into().into()),
            ],
            vec!["f", "f", "H"],
            vec![],
        )?;
        Ok(())
    }
}
