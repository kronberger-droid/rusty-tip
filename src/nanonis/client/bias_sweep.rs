use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::NanonisValue;

impl NanonisClient {
    /// Open the Bias Sweep module.
    ///
    /// Opens and initializes the Bias Sweep module for bias voltage sweep measurements.
    /// This must be called before performing bias sweep operations.
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or module cannot be opened.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Open bias sweep module
    /// client.bias_sweep_open()?;
    /// println!("Bias Sweep module opened");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bias_sweep_open(&mut self) -> Result<(), NanonisError> {
        self.quick_send("BiasSwp.Open", vec![], vec![], vec![])?;
        Ok(())
    }

    /// Start a bias sweep measurement.
    ///
    /// Starts a bias voltage sweep with the configured parameters. The sweep will
    /// step through bias voltages between the set limits while recording selected channels.
    ///
    /// # Arguments
    /// * `get_data` - If `true`, returns measurement data; if `false`, only starts measurement
    /// * `sweep_direction` - Sweep direction: `true` starts from lower limit, `false` from upper
    /// * `z_controller_status` - Z-controller behavior: 0=no change, 1=turn off, 2=don't turn off
    /// * `save_base_name` - Base filename for saving data (empty for no change)
    /// * `reset_bias` - Whether to reset bias after sweep: `true` for on, `false` for off
    ///
    /// # Returns
    /// If `get_data` is true, returns a tuple containing:
    /// - `Vec<String>` - Channel names
    /// - `Vec<Vec<f32>>` - 2D measurement data [rows][columns]
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or sweep cannot start.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Start sweep and get data, from lower to upper limit, turn off Z-controller
    /// let (channels, data) = client.bias_sweep_start(
    ///     true,           // get_data
    ///     true,           // sweep from lower limit
    ///     1,              // turn off Z-controller
    ///     "bias_sweep_001", // save basename
    ///     true            // reset bias after sweep
    /// )?;
    /// println!("Recorded {} channels with {} points", channels.len(), data.len());
    ///
    /// // Just start sweep without getting data
    /// let (_, _) = client.bias_sweep_start(false, true, 0, "", false)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bias_sweep_start(
        &mut self,
        get_data: bool,
        sweep_direction: bool,
        z_controller_status: u32,
        save_base_name: &str,
        reset_bias: bool,
    ) -> Result<(Vec<String>, Vec<Vec<f32>>), NanonisError> {
        let get_data_flag = if get_data { 1u32 } else { 0u32 };
        let direction_flag = if sweep_direction { 1u32 } else { 0u32 };
        let reset_flag = if reset_bias { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "BiasSwp.Start",
            vec![
                NanonisValue::U32(get_data_flag),
                NanonisValue::U32(direction_flag),
                NanonisValue::U32(z_controller_status),
                NanonisValue::String(save_base_name.to_string()),
                NanonisValue::U32(reset_flag),
            ],
            vec!["I", "I", "I", "+*c", "I"],
            vec!["i", "i", "*+c", "i", "i", "2f"],
        )?;

        if result.len() >= 6 {
            let channel_names = result[2].as_string_array()?.to_vec();
            let rows = result[3].as_i32()? as usize;
            let cols = result[4].as_i32()? as usize;

            // Parse 2D data array
            let flat_data = result[5].as_f32_array()?;
            let mut data_2d = Vec::with_capacity(rows);
            for row in 0..rows {
                let start_idx = row * cols;
                let end_idx = start_idx + cols;
                data_2d.push(flat_data[start_idx..end_idx].to_vec());
            }

            Ok((channel_names, data_2d))
        } else {
            Err(NanonisError::Protocol(
                "Invalid bias sweep start response".to_string(),
            ))
        }
    }

    /// Set the bias sweep configuration parameters.
    ///
    /// Configures the bias sweep measurement parameters including number of steps,
    /// timing, and save behavior.
    ///
    /// # Arguments
    /// * `number_of_steps` - Number of bias steps in the sweep (0 = no change)
    /// * `period_ms` - Period between steps in milliseconds (0 = no change)
    /// * `autosave` - Auto-save behavior: 0=no change, 1=on, 2=off
    /// * `save_dialog_box` - Show save dialog: 0=no change, 1=on, 2=off
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
    /// // Configure 100 steps, 50ms per step, auto-save on, no dialog
    /// client.bias_sweep_props_set(100, 50, 1, 2)?;
    ///
    /// // High resolution sweep with slower timing
    /// client.bias_sweep_props_set(500, 100, 1, 2)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bias_sweep_props_set(
        &mut self,
        number_of_steps: u16,
        period_ms: u16,
        autosave: u16,
        save_dialog_box: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "BiasSwp.PropsSet",
            vec![
                NanonisValue::U16(number_of_steps),
                NanonisValue::U16(period_ms),
                NanonisValue::U16(autosave),
                NanonisValue::U16(save_dialog_box),
            ],
            vec!["H", "H", "H", "H"],
            vec![],
        )?;
        Ok(())
    }

    /// Set the bias sweep voltage limits.
    ///
    /// Configures the voltage range for the bias sweep. The sweep will step
    /// between these limits according to the configured number of steps.
    ///
    /// # Arguments
    /// * `lower_limit` - Lower voltage limit in volts
    /// * `upper_limit` - Upper voltage limit in volts
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid limits provided.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Set sweep range from -2V to +2V
    /// client.bias_sweep_limits_set(-2.0, 2.0)?;
    ///
    /// // Positive voltage sweep only
    /// client.bias_sweep_limits_set(0.0, 5.0)?;
    ///
    /// // Negative voltage sweep
    /// client.bias_sweep_limits_set(-3.0, 0.0)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bias_sweep_limits_set(
        &mut self,
        lower_limit: f32,
        upper_limit: f32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "BiasSwp.LimitsSet",
            vec![
                NanonisValue::F32(lower_limit),
                NanonisValue::F32(upper_limit),
            ],
            vec!["f", "f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current bias sweep voltage limits.
    ///
    /// Returns the voltage range configuration for bias sweep measurements.
    ///
    /// # Returns
    /// A tuple containing:
    /// - `f32` - Lower voltage limit in volts
    /// - `f32` - Upper voltage limit in volts
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
    /// let (lower, upper) = client.bias_sweep_limits_get()?;
    /// println!("Bias sweep range: {:.2}V to {:.2}V", lower, upper);
    /// println!("Sweep span: {:.2}V", upper - lower);
    ///
    /// // Check if limits are reasonable
    /// if (upper - lower).abs() < 0.1 {
    ///     println!("Warning: Very narrow sweep range");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn bias_sweep_limits_get(&mut self) -> Result<(f32, f32), NanonisError> {
        let result =
            self.quick_send("BiasSwp.LimitsGet", vec![], vec![], vec!["f", "f"])?;

        if result.len() >= 2 {
            Ok((result[0].as_f32()?, result[1].as_f32()?))
        } else {
            Err(NanonisError::Protocol(
                "Invalid bias sweep limits response".to_string(),
            ))
        }
    }
}
