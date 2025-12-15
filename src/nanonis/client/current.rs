use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::NanonisValue;

impl NanonisClient {
    /// Get the current tunneling current value.
    ///
    /// Returns the instantaneous tunneling current measurement from the current amplifier.
    /// This is one of the most fundamental measurements in STM, providing direct information
    /// about the tip-sample conductance.
    ///
    /// # Returns
    /// Current value in Amperes (A).
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
    /// let current = client.current_get()?;
    /// println!("Tunneling current: {:.3e} A", current);
    ///
    /// // Convert to more convenient units
    /// if current.abs() < 1e-9 {
    ///     println!("Current: {:.1} pA", current * 1e12);
    /// } else {
    ///     println!("Current: {:.1} nA", current * 1e9);
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn current_get(&mut self) -> Result<f32, NanonisError> {
        let result = self.quick_send("Current.Get", vec![], vec![], vec!["f"])?;

        match result.first() {
            Some(value) => Ok(value.as_f32()?),
            None => Err(NanonisError::Protocol(
                "No current value returned".to_string(),
            )),
        }
    }

    /// Get the current value from the "Current 100" module.
    ///
    /// Returns the tunneling current from the specialized Current 100 module,
    /// which may have different gain or filtering characteristics than the main
    /// current amplifier.
    ///
    /// # Returns
    /// Current 100 value in Amperes (A).
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
    /// let current_100 = client.current_100_get()?;
    /// println!("Current 100 module: {:.3e} A", current_100);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn current_100_get(&mut self) -> Result<f32, NanonisError> {
        let result = self.quick_send("Current.100Get", vec![], vec![], vec!["f"])?;

        match result.first() {
            Some(value) => Ok(value.as_f32()?),
            None => Err(NanonisError::Protocol(
                "No current 100 value returned".to_string(),
            )),
        }
    }

    /// Get the BEEM current value from the corresponding module.
    ///
    /// Returns the Ballistic Electron Emission Microscopy (BEEM) current value
    /// in systems equipped with BEEM capabilities. BEEM measures hot electrons
    /// transmitted through thin metal films.
    ///
    /// # Returns
    /// BEEM current value in Amperes (A).
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - BEEM module is not available or not configured
    /// - Communication fails or protocol error occurs
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// let beem_current = client.current_beem_get()?;
    /// println!("BEEM current: {:.3e} A", beem_current);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn current_beem_get(&mut self) -> Result<f32, NanonisError> {
        let result = self.quick_send("Current.BEEMGet", vec![], vec![], vec!["f"])?;

        match result.first() {
            Some(value) => Ok(value.as_f32()?),
            None => Err(NanonisError::Protocol(
                "No BEEM current value returned".to_string(),
            )),
        }
    }

    /// Set the gain and filter of the current amplifier.
    ///
    /// Configures the current amplifier's gain and filtering characteristics.
    /// Use `current_gains_get()` to retrieve the available gain and filter options
    /// before setting specific indices.
    ///
    /// # Arguments
    /// * `gain_index` - Index from the list of available gains
    /// * `filter_index` - Index from the list of available filters
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Invalid gain or filter index provided
    /// - Communication fails or protocol error occurs
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Get available options first
    /// let (gains, current_gain_idx, filters, current_filter_idx) = client.current_gains_get()?;
    /// println!("Available gains: {:?}", gains);
    /// println!("Available filters: {:?}", filters);
    ///
    /// // Set to high gain (index 3) with medium filtering (index 1)
    /// client.current_gain_set(3, 1)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn current_gain_set(
        &mut self,
        gain_index: i32,
        filter_index: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Current.GainSet",
            vec![
                NanonisValue::I32(gain_index),
                NanonisValue::I32(filter_index),
            ],
            vec!["i", "i"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the available gains and filters of the current amplifier.
    ///
    /// Returns all selectable gains and filters for the current amplifier, along with
    /// the currently selected indices. This information is needed for `current_gain_set()`.
    ///
    /// # Returns
    /// A tuple containing:
    /// - `Vec<String>` - Array of available gain descriptions
    /// - `u16` - Index of currently selected gain
    /// - `Vec<String>` - Array of available filter descriptions  
    /// - `i32` - Index of currently selected filter
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
    /// let (gains, gain_idx, filters, filter_idx) = client.current_gains_get()?;
    ///
    /// println!("Current gain: {} (index {})", gains[gain_idx as usize], gain_idx);
    /// println!("Current filter: {} (index {})", filters[filter_idx as usize], filter_idx);
    ///
    /// // List all available options
    /// for (i, gain) in gains.iter().enumerate() {
    ///     println!("Gain {}: {}", i, gain);
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn current_gains_get(
        &mut self,
    ) -> Result<(Vec<String>, u16, Vec<String>, i32), NanonisError> {
        let result = self.quick_send(
            "Current.GainsGet",
            vec![],
            vec![],
            vec!["i", "i", "*+c", "i", "i", "i", "*+c", "i"],
        )?;

        if result.len() >= 8 {
            let gains = result[2].as_string_array()?.to_vec();
            let gain_index = result[3].as_u16()?;
            let filters = result[6].as_string_array()?.to_vec();
            let filter_index = result[7].as_i32()?;
            Ok((gains, gain_index, filters, filter_index))
        } else {
            Err(NanonisError::Protocol(
                "Invalid current gains response".to_string(),
            ))
        }
    }

    /// Set the calibration and offset for a specific gain in the Current module.
    ///
    /// Configures the calibration parameters for accurate current measurements.
    /// Each gain setting can have its own calibration and offset values.
    ///
    /// # Arguments
    /// * `gain_index` - Index of the gain to calibrate (-1 for currently selected gain)
    /// * `calibration` - Calibration factor (typically A/V or similar)
    /// * `offset` - Offset value in the same units
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
    /// // Calibrate currently selected gain
    /// client.current_calibr_set(-1, 1.0e-9, 0.0)?;
    ///
    /// // Calibrate specific gain (index 2) with offset correction
    /// client.current_calibr_set(2, 9.87e-10, -1.5e-12)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn current_calibr_set(
        &mut self,
        gain_index: i32,
        calibration: f64,
        offset: f64,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Current.CalibrSet",
            vec![
                NanonisValue::I32(gain_index),
                NanonisValue::F64(calibration),
                NanonisValue::F64(offset),
            ],
            vec!["i", "d", "d"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the calibration and offset for a specific gain in the Current module.
    ///
    /// Returns the calibration parameters for the specified gain setting.
    ///
    /// # Arguments
    /// * `gain_index` - Index of the gain to query (-1 for currently selected gain)
    ///
    /// # Returns
    /// A tuple containing:
    /// - `f64` - Calibration factor
    /// - `f64` - Offset value
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid gain index.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Get calibration for currently selected gain
    /// let (calibration, offset) = client.current_calibr_get(-1)?;
    /// println!("Current calibration: {:.3e}, offset: {:.3e}", calibration, offset);
    ///
    /// // Check calibration for specific gain
    /// let (cal, off) = client.current_calibr_get(2)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn current_calibr_get(&mut self, gain_index: i32) -> Result<(f64, f64), NanonisError> {
        let result = self.quick_send(
            "Current.CalibrGet",
            vec![NanonisValue::I32(gain_index)],
            vec!["i"],
            vec!["d", "d"],
        )?;

        if result.len() >= 2 {
            let calibration = result[0].as_f64()?;
            let offset = result[1].as_f64()?;
            Ok((calibration, offset))
        } else {
            Err(NanonisError::Protocol(
                "Invalid current calibration response".to_string(),
            ))
        }
    }
}
