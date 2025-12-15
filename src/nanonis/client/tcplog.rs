use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::{NanonisValue, TCPLogStatus};

impl NanonisClient {
    /// Start the acquisition in the TCP Logger module.
    ///
    /// Before using this function, select the channels to record in the TCP Logger
    /// using `tcplog_chs_set()`.
    ///
    /// # Returns
    /// `Ok(())` if the command succeeds.
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Communication with the server fails
    /// - Protocol error occurs
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Configure channels first (using signal slots, not full indices)
    /// client.tcplog_chs_set(vec![0, 1, 2])?; // First 3 signal slots
    ///
    /// // Start logging
    /// client.tcplog_start()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tcplog_start(&mut self) -> Result<(), NanonisError> {
        self.quick_send("TCPLog.Start", vec![], vec![], vec![])?;
        Ok(())
    }

    /// Stop the acquisition in the TCP Logger module.
    ///
    /// # Returns
    /// `Ok(())` if the command succeeds.
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Communication with the server fails
    /// - Protocol error occurs
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Stop logging
    /// client.tcplog_stop()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tcplog_stop(&mut self) -> Result<(), NanonisError> {
        self.quick_send("TCPLog.Stop", vec![], vec![], vec![])?;
        Ok(())
    }

    /// Set the list of recorded channels in the TCP Logger module.
    ///
    /// The channel indexes are comprised between 0 and 23 for the 24 signals
    /// assigned in the Signals Manager. To get the signal name and its
    /// corresponding index in the list of the 128 available signals in the
    /// Nanonis Controller, use the `signal_names_get()` function.
    ///
    /// # Arguments
    /// * `channel_indexes` - Vector of channel indexes to record (0-23)
    ///
    /// # Returns
    /// `Ok(())` if the command succeeds.
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Invalid channel indexes provided
    /// - Communication with the server fails
    /// - Protocol error occurs
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Record first few signal slots (current, height, etc.)
    /// client.tcplog_chs_set(vec![0, 1, 2])?;
    ///
    /// // Record only the first slot (typically current)
    /// client.tcplog_chs_set(vec![0])?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tcplog_chs_set(&mut self, channel_indexes: Vec<i32>) -> Result<(), NanonisError> {
        for &index in &channel_indexes {
            if !(0..=23).contains(&index) {
                return Err(NanonisError::InvalidCommand(
                    format!("Invalid signal slot index: {}. Must be between 0-23 (signal slots, not full signal indices)", index)
                ));
            }
        }
        let num_channels = channel_indexes.len() as i32;

        self.quick_send(
            "TCPLog.ChsSet",
            vec![
                NanonisValue::I32(num_channels),
                NanonisValue::ArrayI32(channel_indexes),
            ],
            vec!["i", "*i"],
            vec![],
        )?;
        Ok(())
    }

    /// Set the oversampling value in the TCP Logger.
    ///
    /// The oversampling value controls the data acquisition rate.
    ///
    /// # Arguments
    /// * `oversampling_value` - Oversampling index (0-1000)
    ///
    /// # Returns
    /// `Ok(())` if the command succeeds.
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Invalid oversampling value provided (outside 0-1000 range)
    /// - Communication with the server fails
    /// - Protocol error occurs
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::NanonisClient;
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// // Set moderate oversampling
    /// client.tcplog_oversampl_set(100)?;
    ///
    /// // Set maximum oversampling
    /// client.tcplog_oversampl_set(1000)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tcplog_oversampl_set(&mut self, oversampling_value: i32) -> Result<(), NanonisError> {
        if !(0..=1000).contains(&oversampling_value) {
            return Err(NanonisError::InvalidCommand(format!(
                "Invalid oversampling value: {}. Must be between 0-1000",
                oversampling_value
            )));
        }

        self.quick_send(
            "TCPLog.OversamplSet",
            vec![NanonisValue::I32(oversampling_value)],
            vec!["i"],
            vec![],
        )?;
        Ok(())
    }

    /// Return the current status of the TCP Logger.
    ///
    /// # Returns
    /// The current `TCPLogStatus` of the TCP Logger module.
    ///
    /// # Errors
    /// Returns `NanonisError` if:
    /// - Communication with the server fails
    /// - Protocol error occurs
    /// - Invalid status value returned from server
    ///
    /// # Examples
    /// ```no_run
    /// use rusty_tip::{NanonisClient, TCPLogStatus};
    ///
    /// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    ///
    /// let status = client.tcplog_status_get()?;
    /// match status {
    ///     TCPLogStatus::Idle => println!("Logger is idle"),
    ///     TCPLogStatus::Running => println!("Logger is running"),
    ///     TCPLogStatus::BufferOverflow => println!("Warning: Buffer overflow detected!"),
    ///     _ => println!("Logger status: {}", status),
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn tcplog_status_get(&mut self) -> Result<TCPLogStatus, NanonisError> {
        let result = self.quick_send("TCPLog.StatusGet", vec![], vec![], vec!["i"])?;

        println!("{result:?}");

        match result.first() {
            Some(value) => {
                let value = value.as_i32()?;
                match value {
                    0 => Ok(TCPLogStatus::Disconnected),
                    1 => Ok(TCPLogStatus::Idle),
                    2 => Ok(TCPLogStatus::Start),
                    3 => Ok(TCPLogStatus::Stop),
                    4 => Ok(TCPLogStatus::Running),
                    5 => Ok(TCPLogStatus::TCPConnect),
                    6 => Ok(TCPLogStatus::TCPDisconnect),
                    7 => Ok(TCPLogStatus::BufferOverflow),
                    _ => Err(NanonisError::Protocol("Invalid Status value".to_string())),
                }
            }
            None => Err(NanonisError::Protocol(
                "No status value returned".to_string(),
            )),
        }
    }
}
