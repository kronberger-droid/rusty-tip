use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::NanonisValue;
use std::time::Duration;

/// Configuration parameters for tip shaper
#[derive(Debug, Clone)]
pub struct TipShaperConfig {
    pub switch_off_delay: Duration,
    pub change_bias: bool,
    pub bias_v: f32,
    pub tip_lift_m: f32,
    pub lift_time_1: Duration,
    pub bias_lift_v: f32,
    pub bias_settling_time: Duration,
    pub lift_height_m: f32,
    pub lift_time_2: Duration,
    pub end_wait_time: Duration,
    pub restore_feedback: bool,
}

/// Return type for tip shaper properties
pub type TipShaperProps = (f32, u32, f32, f32, f32, f32, f32, f32, f32, f32, u32);

impl NanonisClient {
    /// Set the buffer size of the Tip Move Recorder.
    ///
    /// Sets the number of data elements that can be stored in the Tip Move Recorder
    /// buffer. This recorder tracks signal values while the tip is moving in Follow Me mode.
    /// **Note**: This function clears the existing graph data.
    ///
    /// # Arguments
    /// * `buffer_size` - Number of data elements to store in the recorder buffer
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid buffer size.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Set buffer for 10,000 data points
    /// client.tip_rec_buffer_size_set(10000)?;
    ///
    /// // Set smaller buffer for quick tests
    /// client.tip_rec_buffer_size_set(1000)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tip_rec_buffer_size_set(&mut self, buffer_size: i32) -> Result<(), NanonisError> {
        self.quick_send(
            "TipRec.BufferSizeSet",
            vec![NanonisValue::I32(buffer_size)],
            vec!["i"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current buffer size of the Tip Move Recorder.
    ///
    /// Returns the number of data elements that can be stored in the recorder buffer.
    ///
    /// # Returns
    /// Current buffer size (number of data elements).
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// let buffer_size = client.tip_rec_buffer_size_get()?;
    /// println!("Tip recorder buffer size: {} points", buffer_size);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tip_rec_buffer_size_get(&mut self) -> Result<i32, NanonisError> {
        let result = self.quick_send("TipRec.BufferSizeGet", vec![], vec![], vec!["i"])?;

        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No buffer size returned".to_string(),
            )),
        }
    }

    /// Clear the buffer of the Tip Move Recorder.
    ///
    /// Removes all recorded data from the Tip Move Recorder buffer, resetting
    /// it to an empty state. This is useful before starting a new recording session.
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Clear buffer before starting new measurement
    /// client.tip_rec_buffer_clear()?;
    /// println!("Tip recorder buffer cleared");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tip_rec_buffer_clear(&mut self) -> Result<(), NanonisError> {
        self.quick_send("TipRec.BufferClear", vec![], vec![], vec![])?;
        Ok(())
    }

    /// Get the recorded data from the Tip Move Recorder.
    ///
    /// Returns all data recorded while the tip was moving in Follow Me mode.
    /// This includes channel indexes, names, and the complete 2D data array
    /// with measurements taken during tip movement.
    ///
    /// # Returns
    /// A tuple containing:
    /// - `Vec<i32>` - Channel indexes (0-23 for Signals Manager slots)
    /// - `Vec<Vec<f32>>` - 2D data array \[rows\]\[columns\] with recorded measurements
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or no data available.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Get recorded tip movement data
    /// let (channel_indexes, data) = client.tip_rec_data_get()?;
    ///
    /// println!("Recorded {} channels with {} data points",
    ///          channel_indexes.len(), data.len());
    ///
    /// // Analyze data for each channel
    /// for (i, &channel_idx) in channel_indexes.iter().enumerate() {
    ///     if i < data[0].len() {
    ///         println!("Channel {}: {} values", channel_idx, data.len());
    ///     }
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tip_rec_data_get(&mut self) -> Result<(Vec<i32>, Vec<Vec<f32>>), NanonisError> {
        let result = self.quick_send(
            "TipRec.DataGet",
            vec![],
            vec![],
            vec!["i", "*i", "i", "i", "2f"],
        )?;

        if result.len() >= 5 {
            let channel_indexes = result[1].as_i32_array()?.to_vec();
            let rows = result[2].as_i32()? as usize;
            let cols = result[3].as_i32()? as usize;

            // Parse 2D data array
            let flat_data = result[4].as_f32_array()?;
            let mut data_2d = Vec::with_capacity(rows);
            for row in 0..rows {
                let start_idx = row * cols;
                let end_idx = start_idx + cols;
                data_2d.push(flat_data[start_idx..end_idx].to_vec());
            }

            Ok((channel_indexes, data_2d))
        } else {
            Err(NanonisError::Protocol(
                "Invalid tip recorder data response".to_string(),
            ))
        }
    }

    /// Save the tip movement data to a file.
    ///
    /// Saves all data recorded in Follow Me mode to a file with the specified basename.
    /// Optionally clears the buffer after saving to prepare for new recordings.
    ///
    /// # Arguments
    /// * `clear_buffer` - If `true`, clears buffer after saving
    /// * `basename` - Base filename for saved data (empty to use last basename)
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or file save error occurs.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Save data and clear buffer for next measurement
    /// client.tip_rec_data_save(true, "tip_approach_001")?;
    ///
    /// // Save without clearing buffer (keep data for analysis)
    /// client.tip_rec_data_save(false, "tip_movement_log")?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tip_rec_data_save(
        &mut self,
        clear_buffer: bool,
        basename: &str,
    ) -> Result<(), NanonisError> {
        let clear_flag = if clear_buffer { 1u32 } else { 0u32 };

        self.quick_send(
            "TipRec.DataSave",
            vec![
                NanonisValue::U32(clear_flag),
                NanonisValue::String(basename.to_string()),
            ],
            vec!["I", "+*c"],
            vec![],
        )?;
        Ok(())
    }

    /// Start the tip shaper procedure for tip conditioning.
    ///
    /// Initiates the tip shaper procedure which performs controlled tip conditioning
    /// by applying specific voltage sequences and mechanical movements. This is used
    /// to improve tip sharpness and stability after crashes or contamination.
    ///
    /// # Arguments
    /// * `wait_until_finished` - If `true`, waits for procedure completion
    /// * `timeout_ms` - Timeout in milliseconds (-1 for infinite wait)
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or procedure cannot start.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Start tip shaping and wait for completion (30 second timeout)
    /// client.tip_shaper_start(true, 30000)?;
    /// println!("Tip shaping completed");
    ///
    /// // Start tip shaping without waiting
    /// client.tip_shaper_start(false, 0)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tip_shaper_start(
        &mut self,
        wait_until_finished: bool,
        timeout: Duration,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u32 } else { 0u32 };
        let timeout = timeout.as_millis().min(u32::MAX as u128) as i32;

        self.quick_send(
            "TipShaper.Start",
            vec![NanonisValue::U32(wait_flag), NanonisValue::I32(timeout)],
            vec!["I", "i"],
            vec![],
        )?;
        Ok(())
    }

    /// Set the tip shaper procedure configuration.
    ///
    /// Configures all parameters for the tip conditioning procedure including
    /// timing, voltages, and mechanical movements. This is a complex procedure
    /// with multiple stages of tip treatment.
    ///
    /// # Arguments
    /// * `config` - Tip shaper configuration parameters
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid parameters.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::{NanonisClient, TipShaperConfig};
    /// use std::time::Duration;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Conservative tip conditioning parameters
    /// let config = TipShaperConfig {
    ///     switch_off_delay: Duration::from_millis(100),
    ///     change_bias: 1,      // true
    ///     bias_v: -2.0,
    ///     tip_lift_m: 50e-9,   // 50 nm
    ///     lift_time_1: Duration::from_secs(1),
    ///     bias_lift_v: 5.0,
    ///     bias_settling_time: Duration::from_millis(500),
    ///     lift_height_m: 100e-9, // 100 nm
    ///     lift_time_2: Duration::from_millis(500),
    ///     end_wait_time: Duration::from_millis(200),
    ///     restore_feedback: 1, // true
    /// };
    /// client.tip_shaper_props_set(config)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tip_shaper_props_set(&mut self, config: TipShaperConfig) -> Result<(), NanonisError> {
        self.quick_send(
            "TipShaper.PropsSet",
            vec![
                NanonisValue::F32(config.switch_off_delay.as_secs_f32()),
                NanonisValue::U32(config.change_bias.into()),
                NanonisValue::F32(config.bias_v),
                NanonisValue::F32(config.tip_lift_m),
                NanonisValue::F32(config.lift_time_1.as_secs_f32()),
                NanonisValue::F32(config.bias_lift_v),
                NanonisValue::F32(config.bias_settling_time.as_secs_f32()),
                NanonisValue::F32(config.lift_height_m),
                NanonisValue::F32(config.lift_time_2.as_secs_f32()),
                NanonisValue::F32(config.end_wait_time.as_secs_f32()),
                NanonisValue::U32(config.restore_feedback.into()),
            ],
            vec!["f", "I", "f", "f", "f", "f", "f", "f", "f", "f", "I"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current tip shaper procedure configuration.
    ///
    /// Returns all parameters currently configured for the tip conditioning procedure.
    /// Use this to verify settings before starting the procedure.
    ///
    /// # Returns
    /// A tuple containing all tip shaper parameters:
    /// - `f32` - Switch off delay (s)
    /// - `u32` - Change bias flag (0=no change, 1=true, 2=false)
    /// - `f32` - Bias voltage (V)
    /// - `f32` - Tip lift distance (m)
    /// - `f32` - First lift time (s)
    /// - `f32` - Bias lift voltage (V)
    /// - `f32` - Bias settling time (s)
    /// - `f32` - Second lift height (m)
    /// - `f32` - Second lift time (s)
    /// - `f32` - End wait time (s)
    /// - `u32` - Restore feedback flag (0=no change, 1=true, 2=false)
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// let (switch_delay, change_bias, bias_v, tip_lift, lift_time1,
    ///      bias_lift, settling, lift_height, lift_time2, end_wait, restore) =
    ///      client.tip_shaper_props_get()?;
    ///
    /// println!("Tip lift: {:.1} nm, Bias: {:.1} V", tip_lift * 1e9, bias_v);
    /// println!("Total procedure time: ~{:.1} s",
    ///          switch_delay + lift_time1 + settling + lift_time2 + end_wait);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tip_shaper_props_get(&mut self) -> Result<TipShaperProps, NanonisError> {
        let result = self.quick_send(
            "TipShaper.PropsGet",
            vec![],
            vec![],
            vec!["f", "I", "f", "f", "f", "f", "f", "f", "f", "f", "I"],
        )?;

        if result.len() >= 11 {
            Ok((
                result[0].as_f32()?,  // switch_off_delay
                result[1].as_u32()?,  // change_bias
                result[2].as_f32()?,  // bias_v
                result[3].as_f32()?,  // tip_lift_m
                result[4].as_f32()?,  // lift_time_1_s
                result[5].as_f32()?,  // bias_lift_v
                result[6].as_f32()?,  // bias_settling_time_s
                result[7].as_f32()?,  // lift_height_m
                result[8].as_f32()?,  // lift_time_2_s
                result[9].as_f32()?,  // end_wait_time_s
                result[10].as_u32()?, // restore_feedback
            ))
        } else {
            Err(NanonisError::Protocol(
                "Invalid tip shaper properties response".to_string(),
            ))
        }
    }

    /// Get the Tip Shaper properties as a type-safe TipShaperConfig struct
    ///
    /// This method returns the same information as `tip_shaper_props_get()` but
    /// with Duration types for time fields instead of raw f32 seconds.
    ///
    /// # Returns
    /// Returns a `TipShaperConfig` struct with type-safe Duration fields for all time parameters.
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or if the response format is invalid.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    /// use std::time::Duration;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    /// let config = client.tip_shaper_config_get()?;
    ///
    /// println!("Tip lift: {:.1} nm, Bias: {:.1} V",
    ///          config.tip_lift_m * 1e9, config.bias_v);
    /// println!("Total procedure time: {:.1} s",
    ///          (config.switch_off_delay + config.lift_time_1 +
    ///           config.bias_settling_time + config.lift_time_2 +
    ///           config.end_wait_time).as_secs_f32());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tip_shaper_config_get(&mut self) -> Result<TipShaperConfig, NanonisError> {
        let result = self.quick_send(
            "TipShaper.PropsGet",
            vec![],
            vec![],
            vec!["f", "I", "f", "f", "f", "f", "f", "f", "f", "f", "I"],
        )?;

        if result.len() >= 11 {
            let restore_feedback = match result[10].as_u32()? {
                0 => true,
                1 => false,
                _ => panic!("Wrong return value for restore_feedback"),
            };

            let change_bias = match result[1].as_u32()? {
                0 => true,
                1 => false,
                _ => panic!("Wrong return value for change_bias"),
            };

            Ok(TipShaperConfig {
                switch_off_delay: Duration::from_secs_f32(result[0].as_f32()?),
                change_bias,
                bias_v: result[2].as_f32()?,
                tip_lift_m: result[3].as_f32()?,
                lift_time_1: Duration::from_secs_f32(result[4].as_f32()?),
                bias_lift_v: result[5].as_f32()?,
                bias_settling_time: Duration::from_secs_f32(result[6].as_f32()?),
                lift_height_m: result[7].as_f32()?,
                lift_time_2: Duration::from_secs_f32(result[8].as_f32()?),
                end_wait_time: Duration::from_secs_f32(result[9].as_f32()?),
                restore_feedback,
            })
        } else {
            Err(NanonisError::Protocol(
                "Invalid tip shaper properties response".to_string(),
            ))
        }
    }
}
