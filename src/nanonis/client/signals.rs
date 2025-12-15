use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::{NanonisValue, SignalIndex};

impl NanonisClient {
    /// Get available signal names
    pub fn signal_names_get(&mut self) -> Result<Vec<String>, NanonisError> {
        let result = self.quick_send("Signals.NamesGet", vec![], vec![], vec!["+*c"])?;
        match result.first() {
            Some(value) => {
                let signal_names = value.as_string_array()?.to_vec();

                Ok(signal_names)
            }
            None => Err(NanonisError::Protocol(
                "No signal names returned".to_string(),
            )),
        }
    }

    /// Get calibration and offset of a signal by index
    pub fn signals_calibr_get(
        &mut self,
        signal_index: SignalIndex,
    ) -> Result<(f32, f32), NanonisError> {
        let result = self.quick_send(
            "Signals.CalibrGet",
            vec![NanonisValue::I32(signal_index.into())],
            vec!["i"],
            vec!["f", "f"],
        )?;
        if result.len() >= 2 {
            Ok((result[0].as_f32()?, result[1].as_f32()?))
        } else {
            Err(NanonisError::Protocol(
                "Invalid calibration response".to_string(),
            ))
        }
    }

    /// Get range limits of a signal by index
    pub fn signals_range_get(
        &mut self,
        signal_index: SignalIndex,
    ) -> Result<(f32, f32), NanonisError> {
        let result = self.quick_send(
            "Signals.RangeGet",
            vec![NanonisValue::I32(signal_index.into())],
            vec!["i"],
            vec!["f", "f"],
        )?;
        if result.len() >= 2 {
            Ok((result[0].as_f32()?, result[1].as_f32()?)) // (max, min)
        } else {
            Err(NanonisError::Protocol("Invalid range response".to_string()))
        }
    }

    /// Get current values of signals by index(es)
    pub fn signals_vals_get(
        &mut self,
        signal_indexes: Vec<i32>,
        wait_for_newest_data: bool,
    ) -> Result<Vec<f32>, NanonisError> {
        let indexes = signal_indexes;
        let wait_flag = if wait_for_newest_data { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "Signals.ValsGet",
            vec![
                NanonisValue::ArrayI32(indexes),
                NanonisValue::U32(wait_flag),
            ],
            vec!["+*i", "I"],
            vec!["i", "*f"],
        )?;

        if result.len() >= 2 {
            match &result[1] {
                NanonisValue::ArrayF32(values) => Ok(values.clone()),
                _ => Err(NanonisError::Protocol(
                    "Invalid signal values response".to_string(),
                )),
            }
        } else {
            Err(NanonisError::Protocol(
                "Incomplete signal values response".to_string(),
            ))
        }
    }

    /// Get the current value of a single selected signal.
    ///
    /// Returns the current value of the selected signal, oversampled during the
    /// Acquisition Period time (Tap). The signal is continuously oversampled and published
    /// every Tap seconds.
    ///
    /// # Signal Measurement Principle
    /// This function waits for the next oversampled data to be published and returns its value.
    /// It does not trigger a measurement but waits for data to be published. The function
    /// returns a value 0 to Tap seconds after being called.
    ///
    /// **Important**: If you change a signal and immediately call this function, you might
    /// get "old" data measured before the signal change. Set `wait_for_newest_data` to `true`
    /// to ensure you get only fresh data.
    ///
    /// # Arguments
    /// * `signal_index` - Signal index (0-127)
    /// * `wait_for_newest_data` - If `true`, discards first value and waits for fresh data.
    ///   Takes Tap to 2*Tap seconds. If `false`, returns next available value (0 to Tap seconds).
    ///
    /// # Returns
    /// The signal value in physical units.
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Invalid signal index provided
    /// - Communication timeout or protocol error
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::{NanonisClient, SignalIndex};
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Read bias signal immediately
    /// let bias_value = client.signal_val_get(SignalIndex(24), false)?;
    ///
    /// // Wait for fresh data after signal change
    /// let fresh_value = client.signal_val_get(SignalIndex(24), true)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn signal_val_get(
        &mut self,
        signal_index: impl Into<SignalIndex>,
        wait_for_newest_data: bool,
    ) -> Result<f32, NanonisError> {
        let wait_flag = if wait_for_newest_data { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "Signals.ValGet",
            vec![
                NanonisValue::I32(signal_index.into().into()),
                NanonisValue::U32(wait_flag),
            ],
            vec!["i", "I"],
            vec!["f"],
        )?;

        match result.first() {
            Some(value) => Ok(value.as_f32()?),
            None => Err(NanonisError::Protocol(
                "No signal value returned".to_string(),
            )),
        }
    }

    /// Get the list of measurement channels names available in the software.
    ///
    /// Returns the names of measurement channels used in sweepers and other measurement modules.
    ///
    /// **Important Note**: Measurement channels are different from Signals. Measurement channels
    /// are used in sweepers, while Signals are used by graphs and other modules. The indexes
    /// returned here are used for sweeper channel configuration (e.g., `GenSwp.ChannelsGet/Set`).
    ///
    /// # Returns
    /// A vector of measurement channel names where each name corresponds to an index
    /// that can be used in sweeper functions.
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
    /// let meas_channels = client.signals_meas_names_get()?;
    /// println!("Available measurement channels: {}", meas_channels.len());
    ///
    /// for (index, name) in meas_channels.iter().enumerate() {
    ///     println!("  {}: {}", index, name);
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn signals_meas_names_get(&mut self) -> Result<Vec<String>, NanonisError> {
        let result = self.quick_send(
            "Signals.MeasNamesGet",
            vec![],
            vec![],
            vec!["i", "i", "*+c"],
        )?;

        if result.len() >= 3 {
            let meas_names = result[2].as_string_array()?.to_vec();
            Ok(meas_names)
        } else {
            Err(NanonisError::Protocol(
                "Invalid measurement names response".to_string(),
            ))
        }
    }

    /// Get the list of additional Real-Time (RT) signals and current assignments.
    ///
    /// Returns the list of additional RT signals available for assignment to Internal 23 and 24,
    /// plus the names of signals currently assigned to these internal channels.
    ///
    /// **Note**: This assignment in the Signals Manager doesn't automatically make them available
    /// in graphs and modules. Internal 23 and 24 must be assigned to one of the 24 display slots
    /// using functions like `Signals.InSlotSet` to be visible in the software.
    ///
    /// # Returns
    /// A tuple containing:
    /// - `Vec<String>` - List of additional RT signals that can be assigned to Internal 23/24
    /// - `String` - Name of RT signal currently assigned to Internal 23
    /// - `String` - Name of RT signal currently assigned to Internal 24
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
    /// let (available_signals, internal_23, internal_24) = client.signals_add_rt_get()?;
    ///
    /// println!("Available additional RT signals: {}", available_signals.len());
    /// for (i, signal) in available_signals.iter().enumerate() {
    ///     println!("  {}: {}", i, signal);
    /// }
    ///
    /// println!("Internal 23 assigned to: {}", internal_23);
    /// println!("Internal 24 assigned to: {}", internal_24);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn signals_add_rt_get(&mut self) -> Result<(Vec<String>, String, String), NanonisError> {
        let result = self.quick_send(
            "Signals.AddRTGet",
            vec![],
            vec![],
            vec!["i", "i", "*+c", "i", "*-c", "i", "*-c"],
        )?;

        if result.len() >= 7 {
            let available_signals = result[2].as_string_array()?.to_vec();
            let internal_23 = result[4].as_string()?.to_string();
            let internal_24 = result[6].as_string()?.to_string();
            Ok((available_signals, internal_23, internal_24))
        } else {
            Err(NanonisError::Protocol(
                "Invalid additional RT signals response".to_string(),
            ))
        }
    }

    /// Assign additional Real-Time (RT) signals to Internal 23 and 24 signals.
    ///
    /// Links advanced RT signals to Internal 23 and Internal 24 in the Signals Manager.
    /// This enables routing of specialized real-time signals through the internal channel system.
    ///
    /// **Important Note**: This assignment only links the RT signals to Internal 23/24.
    /// To make them visible in graphs and available for acquisition in modules, Internal 23 and 24
    /// must be assigned to one of the 24 display slots using functions like `Signals.InSlotSet`.
    ///
    /// # Arguments
    /// * `additional_rt_signal_1` - Index of the RT signal to assign to Internal 23 (from `signals_add_rt_get()`)
    /// * `additional_rt_signal_2` - Index of the RT signal to assign to Internal 24 (from `signals_add_rt_get()`)
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Invalid RT signal indices provided
    /// - RT signals are not available or accessible
    /// - Communication fails or protocol error occurs
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // First, get the available RT signals
    /// let (available_signals, current_23, current_24) = client.signals_add_rt_get()?;
    ///
    /// println!("Available RT signals:");
    /// for (i, signal) in available_signals.iter().enumerate() {
    ///     println!("  {}: {}", i, signal);
    /// }
    ///
    /// // Assign RT signal index 0 to Internal 23 and index 1 to Internal 24
    /// client.signals_add_rt_set(0, 1)?;
    ///
    /// // Verify the assignment
    /// let (_, new_23, new_24) = client.signals_add_rt_get()?;
    /// println!("Internal 23 now assigned to: {}", new_23);
    /// println!("Internal 24 now assigned to: {}", new_24);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn signals_add_rt_set(
        &mut self,
        additional_rt_signal_1: i32,
        additional_rt_signal_2: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Signals.AddRTSet",
            vec![
                NanonisValue::I32(additional_rt_signal_1),
                NanonisValue::I32(additional_rt_signal_2),
            ],
            vec!["i", "i"],
            vec![],
        )?;
        Ok(())
    }
}
