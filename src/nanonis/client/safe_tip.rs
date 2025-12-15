use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::NanonisValue;

impl NanonisClient {
    /// Switch the Safe Tip feature on or off.
    ///
    /// The Safe Tip feature provides automatic tip protection by monitoring specific signals
    /// and triggering safety actions when dangerous conditions are detected. This prevents
    /// tip crashes and damage during scanning and approach operations.
    ///
    /// # Arguments
    /// * `safe_tip_on` - `true` to enable Safe Tip, `false` to disable
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
    /// // Enable Safe Tip protection
    /// client.safe_tip_on_off_set(true)?;
    /// println!("Safe Tip protection enabled");
    ///
    /// // Disable for manual operations
    /// client.safe_tip_on_off_set(false)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn safe_tip_on_off_set(
        &mut self,
        safe_tip_on: bool,
    ) -> Result<(), NanonisError> {
        let status = if safe_tip_on { 1u16 } else { 2u16 };

        self.quick_send(
            "SafeTip.OnOffSet",
            vec![NanonisValue::U16(status)],
            vec!["H"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current on-off status of the Safe Tip feature.
    ///
    /// Returns whether the Safe Tip protection is currently active. This is essential
    /// for verifying tip safety status before starting potentially dangerous operations.
    ///
    /// # Returns
    /// `true` if Safe Tip is enabled, `false` if disabled.
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
    /// if client.safe_tip_on_off_get()? {
    ///     println!("Safe Tip protection is active");
    /// } else {
    ///     println!("Warning: Safe Tip protection is disabled");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn safe_tip_on_off_get(&mut self) -> Result<bool, NanonisError> {
        let result =
            self.quick_send("SafeTip.OnOffGet", vec![], vec![], vec!["H"])?;

        match result.first() {
            Some(value) => Ok(value.as_u16()? == 1),
            None => Err(NanonisError::Protocol(
                "No Safe Tip status returned".to_string(),
            )),
        }
    }

    /// Get the current Safe Tip signal value.
    ///
    /// Returns the current value of the signal being monitored by the Safe Tip system.
    /// This allows real-time monitoring of the safety-critical parameter.
    ///
    /// # Returns
    /// Current signal value being monitored by Safe Tip.
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
    /// let signal_value = client.safe_tip_signal_get()?;
    /// println!("Safe Tip signal: {:.3e}", signal_value);
    ///
    /// // Check if approaching threshold
    /// let (_, _, threshold) = client.safe_tip_props_get()?;
    /// if signal_value.abs() > threshold * 0.8 {
    ///     println!("Warning: Approaching Safe Tip threshold!");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn safe_tip_signal_get(&mut self) -> Result<f32, NanonisError> {
        let result =
            self.quick_send("SafeTip.SignalGet", vec![], vec![], vec!["f"])?;

        match result.first() {
            Some(value) => Ok(value.as_f32()?),
            None => Err(NanonisError::Protocol(
                "No Safe Tip signal value returned".to_string(),
            )),
        }
    }

    /// Set the Safe Tip configuration parameters.
    ///
    /// Configures the behavior of the Safe Tip protection system including automatic
    /// recovery and scan pause features. These settings determine how the system
    /// responds to safety threshold violations.
    ///
    /// # Arguments
    /// * `auto_recovery` - Enable automatic Z-controller recovery after Safe Tip event
    /// * `auto_pause_scan` - Enable automatic scan pause/hold on Safe Tip events
    /// * `threshold` - Signal threshold value that triggers Safe Tip protection
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
    /// // Configure Safe Tip with automatic recovery and scan pause
    /// client.safe_tip_props_set(true, true, 1e-9)?;  // 1 nA threshold
    ///
    /// // Conservative settings for delicate samples
    /// client.safe_tip_props_set(true, true, 500e-12)?;  // 500 pA threshold
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn safe_tip_props_set(
        &mut self,
        auto_recovery: bool,
        auto_pause_scan: bool,
        threshold: f32,
    ) -> Result<(), NanonisError> {
        let recovery_flag = if auto_recovery { 1u16 } else { 0u16 };
        let pause_flag = if auto_pause_scan { 1u16 } else { 0u16 };

        self.quick_send(
            "SafeTip.PropsSet",
            vec![
                NanonisValue::U16(recovery_flag),
                NanonisValue::U16(pause_flag),
                NanonisValue::F32(threshold),
            ],
            vec!["H", "H", "f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current Safe Tip configuration.
    ///
    /// Returns all Safe Tip protection parameters including automatic recovery settings,
    /// scan pause behavior, and the safety threshold value.
    ///
    /// # Returns
    /// A tuple containing:
    /// - `bool` - Auto recovery enabled/disabled
    /// - `bool` - Auto pause scan enabled/disabled  
    /// - `f32` - Threshold value for triggering Safe Tip protection
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
    /// let (auto_recovery, auto_pause, threshold) = client.safe_tip_props_get()?;
    ///
    /// println!("Safe Tip Configuration:");
    /// println!("  Auto recovery: {}", if auto_recovery { "On" } else { "Off" });
    /// println!("  Auto pause scan: {}", if auto_pause { "On" } else { "Off" });
    /// println!("  Threshold: {:.3e}", threshold);
    ///
    /// // Convert threshold to more readable units
    /// if threshold < 1e-9 {
    ///     println!("  Threshold: {:.1} pA", threshold * 1e12);
    /// } else {
    ///     println!("  Threshold: {:.1} nA", threshold * 1e9);
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn safe_tip_props_get(&mut self) -> Result<(bool, bool, f32), NanonisError> {
        let result = self.quick_send(
            "SafeTip.PropsGet",
            vec![],
            vec![],
            vec!["H", "H", "f"],
        )?;

        if result.len() >= 3 {
            let auto_recovery = result[0].as_u16()? == 1;
            let auto_pause_scan = result[1].as_u16()? == 1;
            let threshold = result[2].as_f32()?;
            Ok((auto_recovery, auto_pause_scan, threshold))
        } else {
            Err(NanonisError::Protocol(
                "Invalid Safe Tip properties response".to_string(),
            ))
        }
    }

    /// Set the tip lift distance for safety operations.
    ///
    /// Sets the distance the tip is lifted when safety procedures are triggered.
    /// This is part of the Z-controller tip safety system and works in conjunction
    /// with Safe Tip to provide comprehensive tip protection.
    ///
    /// **Note**: This function is part of the Z-controller system but included here
    /// for safety-related operations.
    ///
    /// # Arguments
    /// * `tip_lift_m` - Tip lift distance in meters (positive = away from surface)
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or invalid lift distance.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Set conservative lift distance for safety
    /// client.z_ctrl_tip_lift_set(100e-9)?;  // 100 nm lift
    ///
    /// // Set larger lift for problematic areas
    /// client.z_ctrl_tip_lift_set(500e-9)?;  // 500 nm lift
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_tip_lift_set(
        &mut self,
        tip_lift_m: f32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "ZCtrl.TipLiftSet",
            vec![NanonisValue::F32(tip_lift_m)],
            vec!["f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current tip lift distance setting.
    ///
    /// Returns the distance the tip will be lifted during safety operations.
    /// This helps verify that adequate safety margins are configured.
    ///
    /// # Returns
    /// Current tip lift distance in meters.
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
    /// let tip_lift = client.z_ctrl_tip_lift_get()?;
    /// println!("Tip lift distance: {:.1} nm", tip_lift * 1e9);
    ///
    /// // Check if lift distance is adequate
    /// if tip_lift < 50e-9 {
    ///     println!("Warning: Tip lift may be too small for safe operation");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn z_ctrl_tip_lift_get(&mut self) -> Result<f32, NanonisError> {
        let result =
            self.quick_send("ZCtrl.TipLiftGet", vec![], vec![], vec!["f"])?;

        match result.first() {
            Some(value) => Ok(value.as_f32()?),
            None => Err(NanonisError::Protocol(
                "No tip lift distance returned".to_string(),
            )),
        }
    }

    /// Perform a comprehensive safety check of all tip protection systems.
    ///
    /// This convenience method checks the status of all major tip safety systems
    /// and returns a summary. Use this before starting critical operations to ensure
    /// all safety measures are properly configured.
    ///
    /// # Returns
    /// A tuple containing safety status information:
    /// - `bool` - Safe Tip enabled
    /// - `f32` - Current Safe Tip signal value
    /// - `f32` - Safe Tip threshold
    /// - `f32` - Tip lift distance (m)
    /// - `bool` - Z-controller status
    ///
    /// # Errors
    /// Returns `NanonisError` if any safety system check fails.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// let (safe_tip_on, signal_val, threshold, tip_lift, z_ctrl_on) =
    ///     client.safety_status_comprehensive()?;
    ///
    /// println!("=== Tip Safety Status ===");
    /// println!("Safe Tip: {}", if safe_tip_on { "ENABLED" } else { "DISABLED" });
    /// println!("Signal: {:.2e} / Threshold: {:.2e}", signal_val, threshold);
    /// println!("Tip Lift: {:.1} nm", tip_lift * 1e9);
    /// println!("Z-Controller: {}", if z_ctrl_on { "ON" } else { "OFF" });
    ///
    /// if !safe_tip_on {
    ///     println!("WARNING: Safe Tip protection is disabled!");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn safety_status_comprehensive(
        &mut self,
    ) -> Result<(bool, f32, f32, f32, bool), NanonisError> {
        let safe_tip_on = self.safe_tip_on_off_get()?;
        let signal_value = self.safe_tip_signal_get()?;
        let (_, _, threshold) = self.safe_tip_props_get()?;
        let tip_lift = self.z_ctrl_tip_lift_get()?;
        let z_ctrl_on = self.z_ctrl_on_off_get()?;

        Ok((safe_tip_on, signal_value, threshold, tip_lift, z_ctrl_on))
    }
}
