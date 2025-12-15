use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::NanonisValue;

impl NanonisClient {
    /// Set the bias voltage applied to the scanning probe tip.
    ///
    /// This corresponds to the Nanonis `Bias.Set` command and is fundamental
    /// for tip-sample interaction control.
    ///
    /// # Arguments
    /// * `voltage` - The bias voltage to apply (in volts)
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - The command fails or communication times out
    /// - The voltage is outside the instrument's safe operating range
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::{NanonisClient };
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Set bias to 1.5V
    /// client.set_bias(1.5)?;
    ///
    /// // Set bias to -0.5V   
    /// client.set_bias(-0.5)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_bias(&mut self, voltage: f32) -> Result<(), NanonisError> {
        self.quick_send(
            "Bias.Set",
            vec![NanonisValue::F32(voltage)],
            vec!["f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current bias voltage applied to the scanning probe tip.
    ///
    /// This corresponds to the Nanonis `Bias.Get` command.
    ///
    /// # Returns
    /// The current bias voltage
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - The command fails or communication times out
    /// - The server returns invalid or missing data
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// let current_bias = client.get_bias()?;
    /// println!("Current bias voltage: {:.3}V", current_bias);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn get_bias(&mut self) -> Result<f32, NanonisError> {
        let result = self.quick_send("Bias.Get", vec![], vec![], vec!["f"])?;
        match result.first() {
            Some(value) => Ok(value.as_f32()?),
            None => {
                Err(NanonisError::Protocol("No bias value returned".to_string()))
            }
        }
    }

    /// Set the range of the bias voltage, if different ranges are available.
    ///
    /// Sets the bias voltage range by selecting from available ranges.
    /// Use `bias_range_get()` first to retrieve the list of available ranges.
    ///
    /// # Arguments
    /// * `bias_range_index` - Index from the list of ranges (0-based)
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Invalid range index is provided
    /// - Communication timeout or protocol error
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // First get available ranges
    /// let (ranges, current_index) = client.bias_range_get()?;
    /// println!("Available ranges: {:?}", ranges);
    ///
    /// // Set to range index 1
    /// client.bias_range_set(1)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bias_range_set(
        &mut self,
        bias_range_index: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Bias.RangeSet",
            vec![NanonisValue::U16(bias_range_index)],
            vec!["H"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the selectable ranges of bias voltage and the index of the selected one.
    ///
    /// Returns all available bias voltage ranges and which one is currently selected.
    /// This information is needed for `bias_range_set()` and `bias_calibr_set/get()`.
    ///
    /// # Returns
    /// A tuple containing:
    /// - `Vec<String>` - Array of available bias range descriptions
    /// - `u16` - Index of currently selected range
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
    /// let (ranges, current_index) = client.bias_range_get()?;
    /// println!("Current range: {} (index {})", ranges[current_index as usize], current_index);
    ///
    /// for (i, range) in ranges.iter().enumerate() {
    ///     println!("Range {}: {}", i, range);
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bias_range_get(&mut self) -> Result<(Vec<String>, u16), NanonisError> {
        let result = self.quick_send(
            "Bias.RangeGet",
            vec![],
            vec![],
            vec!["i", "i", "*+c", "H"],
        )?;
        if result.len() >= 4 {
            let ranges = result[2].as_string_array()?.to_vec();
            let current_index = result[3].as_u16()?;
            Ok((ranges, current_index))
        } else {
            Err(NanonisError::Protocol(
                "Invalid bias range response".to_string(),
            ))
        }
    }

    /// Set the calibration and offset of bias voltage.
    ///
    /// Sets the calibration parameters for the currently selected bias range.
    /// If multiple ranges are available, this affects only the selected range.
    ///
    /// # Arguments
    /// * `calibration` - Calibration factor (typically in V/V or similar units)
    /// * `offset` - Offset value in the same units as calibration
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid parameters provided.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Set calibration factor and offset for current range
    /// client.bias_calibr_set(1.0, 0.0)?;
    ///
    /// // Apply a small offset correction
    /// client.bias_calibr_set(0.998, 0.005)?;
    /// Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bias_calibr_set(
        &mut self,
        calibration: f32,
        offset: f32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Bias.CalibrSet",
            vec![NanonisValue::F32(calibration), NanonisValue::F32(offset)],
            vec!["f", "f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the calibration and offset of bias voltage.
    ///
    /// Returns the calibration parameters for the currently selected bias range.
    /// If multiple ranges are available, this returns values for the selected range.
    ///
    /// # Returns
    /// A tuple containing:
    /// - `f32` - Calibration factor
    /// - `f32` - Offset value
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
    /// let (calibration, offset) = client.bias_calibr_get()?;
    /// println!("Bias calibration: {:.6}, offset: {:.6}", calibration, offset);
    /// Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bias_calibr_get(&mut self) -> Result<(f32, f32), NanonisError> {
        let result =
            self.quick_send("Bias.CalibrGet", vec![], vec![], vec!["f", "f"])?;
        if result.len() >= 2 {
            let calibration = result[0].as_f32()?;
            let offset = result[1].as_f32()?;
            Ok((calibration, offset))
        } else {
            Err(NanonisError::Protocol(
                "Invalid bias calibration response".to_string(),
            ))
        }
    }

    /// Generate one bias pulse.
    ///
    /// Applies a bias voltage pulse for a specified duration. This is useful for
    /// tunneling spectroscopy, tip conditioning, or sample manipulation experiments.
    ///
    /// # Arguments
    /// * `wait_until_done` - If true, function waits until pulse completes
    /// * `pulse_width_s` - Pulse duration in seconds
    /// * `bias_value_v` - Bias voltage during pulse (in volts)
    /// * `z_controller_hold` - Z-controller behavior: 0=no change, 1=hold, 2=don't hold
    /// * `pulse_mode` - Pulse mode: 0=no change, 1=relative to current, 2=absolute value
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Invalid pulse parameters (negative duration, etc.)
    /// - Bias voltage exceeds safety limits
    /// - Communication timeout or protocol error
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Apply a 100ms pulse at +2V, holding Z-controller, absolute voltage
    /// client.bias_pulse(true, 0.1, 2.0, 1, 2)?;
    ///
    /// // Quick +0.5V pulse relative to current bias, don't wait
    /// client.bias_pulse(false, 0.01, 0.5, 0, 1)?;
    ///
    /// // Long conditioning pulse at -3V absolute, hold Z-controller
    /// client.bias_pulse(true, 1.0, -3.0, 1, 2)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bias_pulse(
        &mut self,
        wait_until_done: bool,
        pulse_width_s: f32,
        bias_value_v: f32,
        z_controller_hold: u16,
        pulse_mode: u16,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_done { 1u32 } else { 0u32 };

        self.quick_send(
            "Bias.Pulse",
            vec![
                NanonisValue::U32(wait_flag),
                NanonisValue::F32(pulse_width_s),
                NanonisValue::F32(bias_value_v),
                NanonisValue::U16(z_controller_hold),
                NanonisValue::U16(pulse_mode),
            ],
            vec!["I", "f", "f", "H", "H"],
            vec![],
        )?;
        Ok(())
    }
}
