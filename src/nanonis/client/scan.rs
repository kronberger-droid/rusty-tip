use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::{NanonisValue, Position, ScanAction, ScanConfig, ScanDirection, ScanFrame};
use std::time::Duration;

impl NanonisClient {
    /// Start, stop, pause or resume a scan
    pub fn scan_action(
        &mut self,
        scan_action: ScanAction,
        scan_direction: ScanDirection,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Scan.Action",
            vec![
                NanonisValue::U16(scan_action.into()),
                NanonisValue::U32(scan_direction.into()),
            ],
            vec!["H", "I"],
            vec![],
        )?;
        Ok(())
    }

    /// Configure the scan frame parameters
    pub fn scan_frame_set(&mut self, frame: ScanFrame) -> Result<(), NanonisError> {
        self.quick_send(
            "Scan.FrameSet",
            vec![
                NanonisValue::F32(frame.center.x as f32),
                NanonisValue::F32(frame.center.y as f32),
                NanonisValue::F32(frame.width_m),
                NanonisValue::F32(frame.height_m),
                NanonisValue::F32(frame.angle_deg),
            ],
            vec!["f", "f", "f", "f", "f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the scan frame parameters
    pub fn scan_frame_get(&mut self) -> Result<ScanFrame, NanonisError> {
        let result = self.quick_send(
            "Scan.FrameGet",
            vec![],
            vec![],
            vec!["f", "f", "f", "f", "f"],
        )?;
        if result.len() >= 5 {
            let center_x = result[0].as_f64()?;
            let center_y = result[1].as_f64()?;
            let width = result[2].as_f32()?;
            let height = result[3].as_f32()?;
            let angle = result[4].as_f32()?;

            Ok(ScanFrame::new(
                Position::new(center_x, center_y),
                width,
                height,
                angle,
            ))
        } else {
            Err(NanonisError::Protocol(
                "Invalid scan frame response".to_string(),
            ))
        }
    }

    /// Get the scan buffer parameters
    /// Returns: (channel_indexes, pixels, lines)
    pub fn scan_buffer_get(&mut self) -> Result<(Vec<i32>, i32, i32), NanonisError> {
        let result =
            self.quick_send("Scan.BufferGet", vec![], vec![], vec!["i", "*i", "i", "i"])?;
        if result.len() >= 4 {
            let channel_indexes = result[1].as_i32_array()?.to_vec();
            let pixels = result[2].as_i32()?;
            let lines = result[3].as_i32()?;
            Ok((channel_indexes, pixels, lines))
        } else {
            Err(NanonisError::Protocol(
                "Invalid scan buffer response".to_string(),
            ))
        }
    }

    /// Get the current scan status.
    ///
    /// Returns whether a scan is currently running or not.
    ///
    /// # Returns
    /// `true` if scan is running, `false` if scan is not running.
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
    /// if client.scan_status_get()? {
    ///     println!("Scan is currently running");
    /// } else {
    ///     println!("Scan is stopped");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn scan_status_get(&mut self) -> Result<bool, NanonisError> {
        let result = self.quick_send("Scan.StatusGet", vec![], vec![], vec!["I"])?;

        match result.first() {
            Some(value) => Ok(value.as_u32()? == 1),
            None => Err(NanonisError::Protocol(
                "No scan status returned".to_string(),
            )),
        }
    }

    /// Configure the scan buffer parameters.
    ///
    /// Sets which channels to record during scanning and the scan resolution.
    /// The channel indexes refer to the 24 signals assigned in the Signals Manager (0-23).
    ///
    /// **Important**: The number of pixels is coerced to the closest multiple of 16
    /// because scan data is sent in packages of 16 pixels.
    ///
    /// # Arguments
    /// * `channel_indexes` - Indexes of channels to record (0-23 for signals in Signals Manager)
    /// * `pixels` - Number of pixels per line (coerced to multiple of 16)
    /// * `lines` - Number of scan lines
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
    /// // Record channels 0, 1, and 2 with 512x512 resolution
    /// client.scan_buffer_set(vec![0, 1, 2], 512, 512)?;
    ///
    /// // High resolution scan with multiple channels
    /// client.scan_buffer_set(vec![0, 1, 2, 3, 4], 1024, 1024)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn scan_buffer_set(
        &mut self,
        channel_indexes: Vec<i32>,
        pixels: i32,
        lines: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Scan.BufferSet",
            vec![
                NanonisValue::ArrayI32(channel_indexes),
                NanonisValue::I32(pixels),
                NanonisValue::I32(lines),
            ],
            vec!["+*i", "i", "i"],
            vec![],
        )?;
        Ok(())
    }

    /// Configure scan speed parameters.
    ///
    /// Sets the tip scanning speeds for both forward and backward scan directions.
    /// You can specify either linear speed or time per line, and set speed ratios
    /// between forward and backward scanning.
    ///
    /// # Arguments
    /// * `forward_linear_speed_m_s` - Forward linear speed in m/s
    /// * `backward_linear_speed_m_s` - Backward linear speed in m/s
    /// * `forward_time_per_line_s` - Forward time per line in seconds
    /// * `backward_time_per_line_s` - Backward time per line in seconds
    /// * `keep_parameter_constant` - Which parameter to keep constant: 0=no change, 1=linear speed, 2=time per line
    /// * `speed_ratio` - Backward tip speed relative to forward speed
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
    /// // Set 1 μm/s forward, 2 μm/s backward, keep linear speed constant
    /// client.scan_speed_set(1e-6, 2e-6, 0.1, 0.05, 1, 2.0)?;
    ///
    /// // Set based on time per line, equal forward/backward speed
    /// client.scan_speed_set(1e-6, 1e-6, 0.1, 0.1, 2, 1.0)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn scan_config_set(&mut self, config: ScanConfig) -> Result<(), NanonisError> {
        self.quick_send(
            "Scan.SpeedSet",
            vec![
                NanonisValue::F32(config.forward_linear_speed_m_s),
                NanonisValue::F32(config.backward_linear_speed_m_s),
                NanonisValue::F32(config.forward_time_per_line_s),
                NanonisValue::F32(config.backward_time_per_line_s),
                NanonisValue::U16(config.keep_parameter_constant),
                NanonisValue::F32(config.speed_ratio),
            ],
            vec!["f", "f", "f", "f", "H", "f"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the current scan speed parameters.
    ///
    /// Returns all scan speed configuration values including linear speeds,
    /// time per line, and speed ratio settings.
    ///
    /// # Returns
    /// A tuple containing:
    /// - `f32` - Forward linear speed (m/s)
    /// - `f32` - Backward linear speed (m/s)
    /// - `f32` - Forward time per line (s)
    /// - `f32` - Backward time per line (s)
    /// - `u16` - Keep parameter constant (0=linear speed, 1=time per line)
    /// - `f32` - Speed ratio (backward relative to forward)
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
    /// let (fwd_speed, bwd_speed, fwd_time, bwd_time, keep_param, speed_ratio) =
    ///     client.scan_speed_get()?;
    ///
    /// println!("Forward speed: {:.2e} m/s", fwd_speed);
    /// println!("Backward speed: {:.2e} m/s", bwd_speed);
    /// println!("Speed ratio: {:.1}", speed_ratio);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn scan_speed_get(&mut self) -> Result<ScanConfig, NanonisError> {
        let result = self.quick_send(
            "Scan.SpeedGet",
            vec![],
            vec![],
            vec!["f", "f", "f", "f", "H", "f"],
        )?;

        if result.len() >= 6 {
            Ok(ScanConfig {
                forward_linear_speed_m_s: result[0].as_f32()?,
                backward_linear_speed_m_s: result[1].as_f32()?,
                forward_time_per_line_s: result[2].as_f32()?,
                backward_time_per_line_s: result[3].as_f32()?,
                keep_parameter_constant: result[4].as_u16()?,
                speed_ratio: result[5].as_f32()?,
            })
        } else {
            Err(NanonisError::Protocol(
                "Invalid scan speed response".to_string(),
            ))
        }
    }

    /// Get the current XY position during scanning.
    ///
    /// Returns the current values of the X and Y signals, useful for monitoring
    /// tip position during scanning operations.
    ///
    /// # Arguments
    /// * `wait_newest_data` - If `true`, discards first value and waits for fresh data
    ///
    /// # Returns
    /// A tuple containing (X position in m, Y position in m)
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
    /// // Get current position immediately
    /// let (x, y) = client.scan_xy_pos_get(false)?;
    /// println!("Current position: ({:.6}, {:.6}) m", x, y);
    ///
    /// // Wait for fresh position data
    /// let (x, y) = client.scan_xy_pos_get(true)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn scan_xy_pos_get(&mut self, wait_newest_data: bool) -> Result<(f32, f32), NanonisError> {
        let wait_flag = if wait_newest_data { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "Scan.XYPosGet",
            vec![NanonisValue::U32(wait_flag)],
            vec!["I"],
            vec!["f", "f"],
        )?;

        if result.len() >= 2 {
            Ok((result[0].as_f32()?, result[1].as_f32()?))
        } else {
            Err(NanonisError::Protocol(
                "Invalid XY position response".to_string(),
            ))
        }
    }

    /// Save the current scan data buffer to file.
    ///
    /// Saves the current scan data into a file. If `wait_until_saved` is true,
    /// the function waits for the save operation to complete before returning.
    ///
    /// # Arguments
    /// * `wait_until_saved` - If `true`, waits for save completion before returning
    /// * `timeout_ms` - Timeout in milliseconds (-1 for indefinite wait)
    ///
    /// # Returns
    /// `true` if timeout occurred while waiting for save completion, `false` otherwise
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
    /// // Save immediately without waiting
    /// let timed_out = client.scan_save(false, 5000)?;
    ///
    /// // Save and wait up to 30 seconds for completion
    /// let timed_out = client.scan_save(true, 30000)?;
    /// if timed_out {
    ///     println!("Save operation timed out");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn scan_save(
        &mut self,
        wait_until_saved: bool,
        timeout_ms: i32,
    ) -> Result<bool, NanonisError> {
        let wait_flag = if wait_until_saved { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "Scan.Save",
            vec![NanonisValue::U32(wait_flag), NanonisValue::I32(timeout_ms)],
            vec!["I", "i"],
            vec!["I"],
        )?;

        match result.first() {
            Some(value) => Ok(value.as_u32()? == 1),
            None => Err(NanonisError::Protocol(
                "No save status returned".to_string(),
            )),
        }
    }

    /// Get scan frame data for a specific channel and direction.
    ///
    /// Returns the complete 2D scan data array for the selected channel.
    /// The channel must be one of the channels configured in the scan buffer.
    ///
    /// # Arguments
    /// * `channel_index` - Index of channel to retrieve data from (must be in acquired channels)
    /// * `data_direction` - Data direction: `true` for forward, `false` for backward
    ///
    /// # Returns
    /// A tuple containing:
    /// - `String` - Channel name
    /// - `Vec<Vec<f32>>` - 2D scan data array \[rows\]\[columns\]
    /// - `bool` - Scan direction: `true` for up, `false` for down
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Invalid channel index (not in acquired channels)
    /// - Communication fails or protocol error occurs
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Get forward scan data for channel 0
    /// let (channel_name, data, scan_up) = client.scan_frame_data_grab(0, true)?;
    /// println!("Channel: {}, Direction: {}", channel_name, if scan_up { "up" } else { "down" });
    /// println!("Data size: {}x{}", data.len(), data[0].len());
    ///
    /// // Get backward scan data
    /// let (_, back_data, _) = client.scan_frame_data_grab(0, false)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn scan_frame_data_grab(
        &mut self,
        channel_index: u32,
        data_direction: bool,
    ) -> Result<(String, Vec<Vec<f32>>, bool), NanonisError> {
        let direction_flag = if data_direction { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "Scan.FrameDataGrab",
            vec![
                NanonisValue::U32(channel_index),
                NanonisValue::U32(direction_flag),
            ],
            vec!["I", "I"],
            vec!["i", "*-c", "i", "i", "2f", "I"],
        )?;

        if result.len() >= 6 {
            let channel_name = result[1].as_string()?.to_string();
            let rows = result[2].as_i32()? as usize;
            let cols = result[3].as_i32()? as usize;

            // Parse 2D array from flat f32 array
            let flat_data = result[4].as_f32_array()?;
            let mut data_2d = Vec::with_capacity(rows);

            for row in 0..rows {
                let start_idx = row * cols;
                let end_idx = start_idx + cols;
                data_2d.push(flat_data[start_idx..end_idx].to_vec());
            }

            let scan_direction = result[5].as_u32()? == 1;
            Ok((channel_name, data_2d, scan_direction))
        } else {
            Err(NanonisError::Protocol(
                "Invalid frame data response".to_string(),
            ))
        }
    }

    /// Wait for the End-of-Scan.
    ///
    /// Waits for the current scan to complete or timeout to occur, whichever comes first.
    /// This is useful for synchronizing operations with scan completion.
    ///
    /// # Arguments
    /// * `timeout` - Timeout duration (-1 for indefinite wait)
    ///
    /// # Returns
    /// A tuple containing:
    /// - `bool` - `true` if timeout occurred, `false` if scan completed normally
    /// - `String` - File path where data was auto-saved (empty if no auto-save)
    ///
    /// # Errors
    /// Returns `NanonisError` if communication fails or protocol error occurs.
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::{NanonisClient, ScanAction, ScanDirection};
    /// use std::time::Duration;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Start a scan
    /// client.scan_action(ScanAction::Start, ScanDirection::Up)?;
    ///
    /// // Wait for scan to complete (up to 5 minutes)
    /// let (timed_out, file_path) = client.scan_wait_end_of_scan(Duration::from_secs(300))?;
    ///
    /// if timed_out {
    ///     println!("Scan timed out after 5 minutes");
    /// } else {
    ///     println!("Scan completed");
    ///     if !file_path.is_empty() {
    ///         println!("Data saved to: {}", file_path);
    ///     }
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn scan_wait_end_of_scan(
        &mut self,
        timeout: Duration,
    ) -> Result<(bool, String), NanonisError> {
        let result = self.quick_send(
            "Scan.WaitEndOfScan",
            vec![NanonisValue::I32(timeout.as_millis() as i32)],
            vec!["i"],
            vec!["I", "I", "*-c"],
        )?;

        if result.len() >= 3 {
            let timeout_occurred = result[0].as_u32()? == 1;
            let file_path = result[2].as_string()?.to_string();
            Ok((timeout_occurred, file_path))
        } else {
            Err(NanonisError::Protocol(
                "Invalid scan wait response".to_string(),
            ))
        }
    }
}
