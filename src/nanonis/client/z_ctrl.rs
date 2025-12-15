use std::time::Duration;

use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::NanonisValue;

impl NanonisClient {
    /// Switch the Z-Controller on or off.
    ///
    /// Controls the Z-Controller state. This is fundamental for enabling/disabling
    /// tip-sample distance regulation during scanning and positioning operations.
    ///
    /// # Arguments
    /// * `controller_on` - `true` to turn controller on, `false` to turn off
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or protocol error occurs.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Turn Z-controller on for feedback control
    /// client.z_ctrl_on_off_set(true)?;
    ///
    /// // Turn Z-controller off for manual positioning
    /// client.z_ctrl_on_off_set(false)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_on_off_set(
        &mut self,
        controller_on: bool,
    ) -> Result<(), NanonisError> {
        let status_flag = if controller_on { 1u32 } else { 0u32 };

        self.quick_send(
            "ZCtrl.OnOffSet",
            vec![NanonisValue::U32(status_flag)],
            vec!["I"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current status of the Z-Controller.
    ///
    /// Returns the real-time status from the controller (not from the Z-Controller module).
    /// This is useful to ensure the controller is truly off before starting experiments,
    /// as there can be communication delays and switch-off delays.
    ///
    /// # Returns
    /// `true` if controller is on, `false` if controller is off.
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or protocol error occurs.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Check controller status before experiment
    /// if client.z_ctrl_on_off_get()? {
    ///     println!("Z-controller is active");
    /// } else {
    ///     println!("Z-controller is off - safe to move tip manually");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_on_off_get(&mut self) -> Result<bool, NanonisError> {
        let result = self.quick_send("ZCtrl.OnOffGet", vec![], vec![], vec!["I"])?;

        match result.first() {
            Some(value) => Ok(value.as_u32()? == 1),
            None => Err(NanonisError::Protocol(
                "No Z-controller status returned".to_string(),
            )),
        }
    }

    /// Set the Z position of the tip.
    ///
    /// **Important**: The Z-controller must be switched OFF to change the tip position.
    /// This function directly sets the tip's Z coordinate for manual positioning.
    ///
    /// # Arguments
    /// * `z_position_m` - Z position in meters
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Z-controller is still active (must be turned off first)
    /// - Position is outside safe limits
    /// - Communication fails or protocol error occurs
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Ensure Z-controller is off
    /// client.z_ctrl_on_off_set(false)?;
    ///
    /// // Move tip to specific Z position (10 nm above surface)
    /// client.z_ctrl_z_pos_set(10e-9)?;
    ///
    /// // Move tip closer to surface (2 nm)
    /// client.z_ctrl_z_pos_set(2e-9)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_z_pos_set(
        &mut self,
        z_position_m: f32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "ZCtrl.ZPosSet",
            vec![NanonisValue::F32(z_position_m)],
            vec!["f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current Z position of the tip.
    ///
    /// Returns the current tip Z coordinate in meters. This works whether
    /// the Z-controller is on or off.
    ///
    /// # Returns
    /// Current Z position in meters.
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or protocol error occurs.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// let z_pos = client.z_ctrl_z_pos_get()?;
    /// println!("Current tip height: {:.2} nm", z_pos * 1e9);
    ///
    /// // Check if tip is at safe distance
    /// if z_pos > 5e-9 {
    ///     println!("Tip is safely withdrawn");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_z_pos_get(&mut self) -> Result<f32, NanonisError> {
        let result = self.quick_send("ZCtrl.ZPosGet", vec![], vec![], vec!["f"])?;

        match result.first() {
            Some(value) => Ok(value.as_f32()?),
            None => {
                Err(NanonisError::Protocol("No Z position returned".to_string()))
            }
        }
    }

    /// Set the setpoint of the Z-Controller.
    ///
    /// The setpoint is the target value for the feedback signal that the Z-controller
    /// tries to maintain by adjusting the tip-sample distance.
    ///
    /// # Arguments
    /// * `setpoint` - Z-controller setpoint value (units depend on feedback signal)
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or protocol error occurs.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Set tunneling current setpoint to 100 pA
    /// client.z_ctrl_setpoint_set(100e-12)?;
    ///
    /// // Set force setpoint for AFM mode
    /// client.z_ctrl_setpoint_set(1e-9)?;  // 1 nN
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_setpoint_set(
        &mut self,
        setpoint: f32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "ZCtrl.SetpntSet",
            vec![NanonisValue::F32(setpoint)],
            vec!["f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current setpoint of the Z-Controller.
    ///
    /// Returns the target value that the Z-controller is trying to maintain.
    ///
    /// # Returns
    /// Current Z-controller setpoint value.
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or protocol error occurs.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// let setpoint = client.z_ctrl_setpoint_get()?;
    /// println!("Current setpoint: {:.3e}", setpoint);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_setpoint_get(&mut self) -> Result<f32, NanonisError> {
        let result =
            self.quick_send("ZCtrl.SetpntGet", vec![], vec![], vec!["f"])?;

        match result.first() {
            Some(value) => Ok(value.as_f32()?),
            None => Err(NanonisError::Protocol("No setpoint returned".to_string())),
        }
    }

    /// Set the Z-Controller gains and time settings.
    ///
    /// Configures the PID controller parameters for Z-axis feedback control.
    /// The integral gain is calculated as I = P/T where P is proportional gain
    /// and T is the time constant.
    ///
    /// # Arguments
    /// * `p_gain` - Proportional gain of the regulation loop
    /// * `time_constant_s` - Time constant T in seconds
    /// * `i_gain` - Integral gain of the regulation loop (calculated as P/T)
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid gains provided.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Set moderate feedback gains for stable operation
    /// client.z_ctrl_gain_set(1.0, 0.1, 10.0)?;
    ///
    /// // Set aggressive gains for fast response
    /// client.z_ctrl_gain_set(5.0, 0.05, 100.0)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_gain_set(
        &mut self,
        p_gain: f32,
        time_constant_s: f32,
        i_gain: f32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "ZCtrl.GainSet",
            vec![
                NanonisValue::F32(p_gain),
                NanonisValue::F32(time_constant_s),
                NanonisValue::F32(i_gain),
            ],
            vec!["f", "f", "f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current Z-Controller gains and time settings.
    ///
    /// Returns the PID controller parameters currently in use.
    ///
    /// # Returns
    /// A tuple containing:
    /// - `f32` - Proportional gain
    /// - `f32` - Time constant in seconds
    /// - `f32` - Integral gain (P/T)
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or protocol error occurs.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// let (p_gain, time_const, i_gain) = client.z_ctrl_gain_get()?;
    /// println!("P: {:.3}, T: {:.3}s, I: {:.3}", p_gain, time_const, i_gain);
    ///
    /// // Check if gains are in reasonable range
    /// if p_gain > 10.0 {
    ///     println!("Warning: High proportional gain may cause instability");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_gain_get(&mut self) -> Result<(f32, f32, f32), NanonisError> {
        let result =
            self.quick_send("ZCtrl.GainGet", vec![], vec![], vec!["f", "f", "f"])?;

        if result.len() >= 3 {
            Ok((
                result[0].as_f32()?,
                result[1].as_f32()?,
                result[2].as_f32()?,
            ))
        } else {
            Err(NanonisError::Protocol("Invalid gain response".to_string()))
        }
    }

    /// Move the tip to its home position.
    ///
    /// Moves the tip to the predefined home position, which can be either absolute
    /// (fixed position) or relative to the current position, depending on the
    /// controller configuration.
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or protocol error occurs.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Move tip to home position after experiment
    /// client.z_ctrl_home()?;
    ///
    /// // Wait a moment for positioning to complete
    /// std::thread::sleep(std::time::Duration::from_secs(1));
    ///
    /// // Check final position
    /// let final_pos = client.z_ctrl_z_pos_get()?;
    /// println!("Tip homed to: {:.2} nm", final_pos * 1e9);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_home(&mut self) -> Result<(), NanonisError> {
        self.quick_send("ZCtrl.Home", vec![], vec![], vec![])?;
        Ok(())
    }

    /// Withdraw the tip.
    ///
    /// Switches off the Z-Controller and fully withdraws the tip to the upper limit.
    /// This is a safety function to prevent tip crashes during approach or when
    /// moving to new scan areas.
    ///
    /// # Arguments
    /// * `wait_until_finished` - If `true`, waits for withdrawal to complete
    /// * `timeout_ms` - Timeout in milliseconds for the withdrawal operation
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or withdrawal times out.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    /// use std::time::Duration;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Emergency withdrawal - don't wait
    /// client.z_ctrl_withdraw(false, Duration::from_secs(5))?;
    ///
    /// // Controlled withdrawal with waiting
    /// client.z_ctrl_withdraw(true, Duration::from_secs(10))?;
    /// println!("Tip safely withdrawn");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_withdraw(
        &mut self,
        wait_until_finished: bool,
        timeout_ms: Duration,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u32 } else { 0u32 };
        self.quick_send(
            "ZCtrl.Withdraw",
            vec![
                NanonisValue::U32(wait_flag),
                NanonisValue::I32(timeout_ms.as_millis() as i32),
            ],
            vec!["I", "i"],
            vec![],
        )?;
        Ok(())
    }
}
