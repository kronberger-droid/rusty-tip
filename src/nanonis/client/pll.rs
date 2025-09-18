use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::NanonisValue;

impl NanonisClient {
    /// Set the amplitude controller setpoint for a PLL modulator.
    ///
    /// Sets the amplitude controller setpoint value for the specified PLL modulator.
    /// This controls the target oscillation amplitude for the phase-locked loop.
    ///
    /// # Arguments
    /// * `modulator_index` - PLL modulator index (starts from 1)
    /// * `setpoint_m` - Amplitude setpoint in meters
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid modulator index.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Set amplitude setpoint for first PLL to 1 nanometer
    /// client.pll_amp_ctrl_setpnt_set(1, 1e-9)?;
    ///
    /// // Set amplitude setpoint for second PLL to 500 picometers
    /// client.pll_amp_ctrl_setpnt_set(2, 500e-12)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn pll_amp_ctrl_setpnt_set(
        &mut self,
        modulator_index: i32,
        setpoint_m: f32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "PLL.AmpCtrlSetpntSet",
            vec![
                NanonisValue::I32(modulator_index),
                NanonisValue::F32(setpoint_m),
            ],
            vec!["i", "f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the amplitude controller setpoint for a PLL modulator.
    ///
    /// Returns the current amplitude controller setpoint value for the specified
    /// PLL modulator.
    ///
    /// # Arguments
    /// * `modulator_index` - PLL modulator index (starts from 1)
    ///
    /// # Returns
    /// * `f32` - Current amplitude setpoint in meters
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid modulator index.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Get current amplitude setpoint for first PLL
    /// let setpoint = client.pll_amp_ctrl_setpnt_get(1)?;
    /// println!("Current amplitude setpoint: {:.2e} m", setpoint);
    ///
    /// // Check if setpoint is within expected range
    /// if setpoint > 1e-9 {
    ///     println!("Amplitude setpoint is greater than 1 nm");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn pll_amp_ctrl_setpnt_get(&mut self, modulator_index: i32) -> Result<f32, NanonisError> {
        let response = self.quick_send(
            "PLL.AmpCtrlSetpntGet",
            vec![NanonisValue::I32(modulator_index)],
            vec!["i"],
            vec!["f"],
        )?;

        match response.first() {
            Some(NanonisValue::F32(setpoint)) => Ok(*setpoint),
            _ => Err(NanonisError::SerializationError(
                "Expected f32 amplitude setpoint".to_string(),
            )),
        }
    }

    /// Set the amplitude controller on/off status for a PLL modulator.
    ///
    /// Switches the amplitude controller for the specified PLL modulator on or off.
    /// When enabled, the amplitude controller actively maintains the oscillation
    /// amplitude at the setpoint value.
    ///
    /// # Arguments
    /// * `modulator_index` - PLL modulator index (starts from 1)
    /// * `status` - true to turn on, false to turn off
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid modulator index.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Turn on amplitude controller for first PLL
    /// client.pll_amp_ctrl_on_off_set(1, true)?;
    ///
    /// // Turn off amplitude controller for second PLL
    /// client.pll_amp_ctrl_on_off_set(2, false)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn pll_amp_ctrl_on_off_set(
        &mut self,
        modulator_index: i32,
        status: bool,
    ) -> Result<(), NanonisError> {
        let status_u32 = if status { 1u32 } else { 0u32 };

        self.quick_send(
            "PLL.AmpCtrlOnOffSet",
            vec![
                NanonisValue::I32(modulator_index),
                NanonisValue::U32(status_u32),
            ],
            vec!["i", "I"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the amplitude controller on/off status for a PLL modulator.
    ///
    /// Returns the current on/off status of the amplitude controller for the
    /// specified PLL modulator.
    ///
    /// # Arguments
    /// * `modulator_index` - PLL modulator index (starts from 1)
    ///
    /// # Returns
    /// * `bool` - true if controller is on, false if off
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid modulator index.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Check amplitude controller status for first PLL
    /// let is_on = client.pll_amp_ctrl_on_off_get(1)?;
    /// if is_on {
    ///     println!("Amplitude controller is active");
    /// } else {
    ///     println!("Amplitude controller is inactive");
    /// }
    ///
    /// // Enable controller if it's off
    /// if !is_on {
    ///     client.pll_amp_ctrl_on_off_set(1, true)?;
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn pll_amp_ctrl_on_off_get(&mut self, modulator_index: i32) -> Result<bool, NanonisError> {
        let response = self.quick_send(
            "PLL.AmpCtrlOnOffGet",
            vec![NanonisValue::I32(modulator_index)],
            vec!["i"],
            vec!["I"],
        )?;

        match response.first() {
            Some(NanonisValue::U32(status)) => Ok(*status != 0),
            _ => Err(NanonisError::InvalidResponse(
                "Expected u32 amplitude controller status".to_string(),
            )),
        }
    }

    /// Set the amplitude controller gain parameters for a PLL modulator.
    ///
    /// Sets the proportional gain and time constant for the amplitude controller
    /// of the specified PLL modulator. These parameters control the response
    /// characteristics of the amplitude control loop.
    ///
    /// # Arguments
    /// * `modulator_index` - PLL modulator index (starts from 1)
    /// * `p_gain_v_div_m` - Proportional gain in V/m
    /// * `time_constant_s` - Time constant in seconds
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid modulator index.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Set moderate gain and fast response for first PLL
    /// client.pll_amp_ctrl_gain_set(1, 1e6, 0.01)?;
    ///
    /// // Set higher gain and slower response for second PLL
    /// client.pll_amp_ctrl_gain_set(2, 5e6, 0.1)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn pll_amp_ctrl_gain_set(
        &mut self,
        modulator_index: i32,
        p_gain_v_div_m: f32,
        time_constant_s: f32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "PLL.AmpCtrlGainSet",
            vec![
                NanonisValue::I32(modulator_index),
                NanonisValue::F32(p_gain_v_div_m),
                NanonisValue::F32(time_constant_s),
            ],
            vec!["i", "f", "f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the amplitude controller gain parameters for a PLL modulator.
    ///
    /// Returns the current proportional gain and time constant settings for the
    /// amplitude controller of the specified PLL modulator.
    ///
    /// # Arguments
    /// * `modulator_index` - PLL modulator index (starts from 1)
    ///
    /// # Returns
    /// * `(f32, f32)` - Tuple of (proportional gain in V/m, time constant in seconds)
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid modulator index.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Get current gain parameters for first PLL
    /// let (p_gain, time_const) = client.pll_amp_ctrl_gain_get(1)?;
    /// println!("P gain: {:.2e} V/m, Time constant: {:.3} s", p_gain, time_const);
    ///
    /// // Check if parameters are within acceptable range
    /// if p_gain < 1e5 {
    ///     println!("Warning: Low proportional gain");
    /// }
    /// if time_const > 1.0 {
    ///     println!("Warning: Slow time constant");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn pll_amp_ctrl_gain_get(
        &mut self,
        modulator_index: i32,
    ) -> Result<(f32, f32), NanonisError> {
        let response = self.quick_send(
            "PLL.AmpCtrlGainGet",
            vec![NanonisValue::I32(modulator_index)],
            vec!["i"],
            vec!["f", "f"],
        )?;

        match (response.first(), response.get(1)) {
            (Some(NanonisValue::F32(p_gain)), Some(NanonisValue::F32(time_const))) => {
                Ok((*p_gain, *time_const))
            }
            _ => Err(NanonisError::InvalidResponse(
                "Expected f32 gain parameters (p_gain, time_constant)".to_string(),
            )),
        }
    }
}
