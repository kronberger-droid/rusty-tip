use crate::protocol::{Protocol, HEADER_SIZE, MAX_RETRY_COUNT};
use crate::types::{BiasVoltage, ConnectionConfig, NanonisError, NanonisValue, Position};
use log::{debug, trace, warn};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

pub struct NanonisClient {
    stream: TcpStream,
    debug: bool,
    config: ConnectionConfig,
}

impl NanonisClient {
    /// Create a new client with default configuration
    pub fn new(addr: &str) -> Result<Self, NanonisError> {
        Self::with_config(addr, ConnectionConfig::default())
    }

    /// Create a new client with custom configuration
    pub fn with_config(addr: &str, config: ConnectionConfig) -> Result<Self, NanonisError> {
        let socket_addr: SocketAddr = addr
            .parse()
            .map_err(|_| NanonisError::InvalidAddress(addr.to_string()))?;

        debug!("Connecting to Nanonis at {addr}");

        let stream =
            TcpStream::connect_timeout(&socket_addr, config.connect_timeout).map_err(|e| {
                warn!("Failed to connect to {addr}: {e}");
                if e.kind() == std::io::ErrorKind::TimedOut {
                    NanonisError::Timeout
                } else {
                    NanonisError::Io(e)
                }
            })?;

        // Set socket timeouts
        stream.set_read_timeout(Some(config.read_timeout))?;
        stream.set_write_timeout(Some(config.write_timeout))?;

        debug!("Successfully connected to Nanonis");

        Ok(Self {
            stream,
            debug: false,
            config,
        })
    }

    /// Enable or disable debug logging
    pub fn set_debug(&mut self, debug: bool) {
        self.debug = debug;
    }

    /// Get current connection configuration
    pub fn config(&self) -> &ConnectionConfig {
        &self.config
    }

    /// Send a command and receive response
    pub fn quick_send(
        &mut self,
        command: &str,
        body: &[NanonisValue],
        body_types: &[&str],
        response_types: &[&str],
    ) -> Result<Vec<NanonisValue>, NanonisError> {
        if body.len() != body_types.len() {
            return Err(NanonisError::InvalidCommand(format!(
                "Body length ({}) doesn't match body types length ({})",
                body.len(),
                body_types.len()
            )));
        }

        let response = self.send(command, body, body_types)?;
        if !response.is_empty() {
            let response_data = Protocol::parse_response(&response, response_types)?;
            if self.debug {
                debug!("Response: {response_data:?}");
            }
            Ok(response_data)
        } else {
            debug!("No data returned for command: {command}");
            Ok(vec![])
        }
    }

    fn send(
        &mut self,
        command: &str,
        body: &[NanonisValue],
        body_types: &[&str],
    ) -> Result<Vec<u8>, NanonisError> {
        let mut body_part = Vec::new();

        // Serialize body
        for (i, value) in body.iter().enumerate() {
            let body_type = body_types[i];
            Protocol::serialize_value(value, body_type, &mut body_part)?;
        }

        let body_size = body_part.len() as u32;

        // Create message header
        let mut message = Protocol::create_command_header(command, body_size);

        // Append body
        message.extend_from_slice(&body_part);

        if self.debug {
            trace!("Send message: {message:?}");
        }

        // Send message
        self.stream.write_all(&message)?;
        self.stream.flush()?;

        trace!("Message sent, waiting for response");

        // Read response header (40 bytes)
        let mut header = [0u8; HEADER_SIZE];
        self.stream.read_exact(&mut header)?;

        // Validate header and get body size
        let response_body_size = Protocol::validate_response_header(&header, command)? as usize;

        // Read response body with retry logic
        let mut response_body = vec![0u8; response_body_size];
        let mut bytes_read = 0;
        let mut counter = 0;

        while bytes_read < response_body_size && counter < MAX_RETRY_COUNT {
            match self.stream.read(&mut response_body[bytes_read..]) {
                Ok(n) if n > 0 => bytes_read += n,
                Ok(_) => break, // EOF
                Err(e) => return Err(NanonisError::Io(e)),
            }
            counter += 1;
        }

        if bytes_read < response_body_size {
            warn!("Incomplete response: got {bytes_read} bytes, expected {response_body_size}");
        }

        response_body.truncate(bytes_read);

        if self.debug {
            debug!("Body size: {response_body_size}, received: {bytes_read}");
            trace!("Header: {header:?}");
            trace!("Body: {response_body:?}");
        }

        Ok(response_body)
    }

    // ==================== Type-safe method implementations ====================

    /// Set bias voltage with type safety
    pub fn set_bias(&mut self, voltage: BiasVoltage) -> Result<(), NanonisError> {
        self.quick_send("Bias.Set", &[NanonisValue::F32(voltage.0)], &["f"], &[])?;
        Ok(())
    }

    /// Get bias voltage with type safety
    pub fn get_bias(&mut self) -> Result<BiasVoltage, NanonisError> {
        let result = self.quick_send("Bias.Get", &[], &[], &["f"])?;
        match result.first() {
            Some(value) => Ok(BiasVoltage(value.as_f32()?)),
            None => Err(NanonisError::Protocol("No bias value returned".to_string())),
        }
    }

    /// Get available signal names
    pub fn signal_names_get(&mut self, print: bool) -> Result<Vec<String>, NanonisError> {
        let result = self.quick_send("Signals.NamesGet", &[], &[], &["+*c"])?;
        match result.first() {
            Some(value) => {
                let signal_names = value.as_string_array()?.to_vec();

                if print {
                    Self::print_signal_names(&signal_names);
                }

                Ok(signal_names)
            }
            None => Err(NanonisError::Protocol(
                "No signal names returned".to_string(),
            )),
        }
    }

    /// Helper function for printing signal names
    fn print_signal_names(names: &[String]) {
        println!("Available signal names ({} total):", names.len());
        for (index, name) in names.iter().enumerate() {
            println!("  {index}: {name}");
        }
    }

    /// Get calibration and offset of a signal by index
    pub fn signals_calibr_get(&mut self, signal_index: i32) -> Result<(f32, f32), NanonisError> {
        let result = self.quick_send(
            "Signals.CalibrGet",
            &[NanonisValue::I32(signal_index)],
            &["i"],
            &["f", "f"],
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
    pub fn signals_range_get(&mut self, signal_index: i32) -> Result<(f32, f32), NanonisError> {
        let result = self.quick_send(
            "Signals.RangeGet",
            &[NanonisValue::I32(signal_index)],
            &["i"],
            &["f", "f"],
        )?;
        if result.len() >= 2 {
            Ok((result[0].as_f32()?, result[1].as_f32()?)) // (max, min)
        } else {
            Err(NanonisError::Protocol("Invalid range response".to_string()))
        }
    }

    /// Get current values of signals by index(es)
    pub fn signals_val_get(
        &mut self,
        signal_indexes: Vec<i32>,
        wait_for_newest_data: bool,
    ) -> Result<Vec<f32>, NanonisError> {
        let indexes = signal_indexes;
        let wait_flag = if wait_for_newest_data { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "Signals.ValsGet",
            &[
                NanonisValue::ArrayI32(indexes),
                NanonisValue::U32(wait_flag),
            ],
            &["+*i", "I"],
            &["i", "*f"],
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

    /// Find signal index by name (case-insensitive)
    pub fn find_signal_index(&mut self, signal_name: &str) -> Result<Option<usize>, NanonisError> {
        let signals = self.signal_names_get(false)?;
        let signal_name_lower = signal_name.to_lowercase();

        for (index, name) in signals.iter().enumerate() {
            if name.to_lowercase().contains(&signal_name_lower) {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    /// Read a signal by name (finds index automatically)
    pub fn read_signal_by_name(
        &mut self,
        signal_name: &str,
        wait_for_newest: bool,
    ) -> Result<f32, NanonisError> {
        match self.find_signal_index(signal_name)? {
            Some(index) => {
                let values = self.signals_val_get(vec![index as i32], wait_for_newest)?;
                values
                    .first()
                    .copied()
                    .ok_or_else(|| NanonisError::Protocol("No signal value returned".to_string()))
            }
            None => Err(NanonisError::InvalidCommand(format!(
                "Signal '{signal_name}' not found"
            ))),
        }
    }

    // ==================== AutoApproach Functions ====================

    /// Open the Auto-Approach module
    pub fn auto_approach_open(&mut self) -> Result<(), NanonisError> {
        self.quick_send("AutoApproach.Open", &[], &[], &[])?;
        Ok(())
    }

    /// Start or stop the Z auto-approach procedure
    pub fn auto_approach_on_off_set(&mut self, on_off: bool) -> Result<(), NanonisError> {
        let value = if on_off { 1u16 } else { 0u16 };
        self.quick_send(
            "AutoApproach.OnOffSet",
            &[NanonisValue::U16(value)],
            &["H"],
            &[],
        )?;
        Ok(())
    }

    /// Get the on-off status of the Z auto-approach procedure
    pub fn auto_approach_on_off_get(&mut self) -> Result<bool, NanonisError> {
        let result = self.quick_send("AutoApproach.OnOffGet", &[], &[], &["H"])?;
        match result.first() {
            Some(value) => {
                let status = value.as_u16()?;
                Ok(status == 1)
            }
            None => Err(NanonisError::Protocol(
                "No auto-approach status returned".to_string(),
            )),
        }
    }

    /// Auto-approach and wait until completion (convenience function)
    pub fn auto_approach_and_wait(&mut self) -> Result<(), NanonisError> {
        println!("Starting auto-approach...");

        // Open auto-approach module
        self.auto_approach_open()?;

        // Wait a bit for module to initialize
        std::thread::sleep(std::time::Duration::from_millis(1000));

        // Start auto-approach
        self.auto_approach_on_off_set(true)?;

        println!("Waiting for auto-approach to complete...");

        // Wait until auto-approach completes
        loop {
            let is_running = self.auto_approach_on_off_get()?;
            if !is_running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        println!("Auto-approach finished");
        Ok(())
    }

    // ==================== Z-Controller Functions ====================

    /// Withdraw the tip
    /// Switches off the Z-Controller and fully withdraws the tip to upper limit
    pub fn z_ctrl_withdraw(
        &mut self,
        wait_until_finished: bool,
        timeout_ms: i32,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u32 } else { 0u32 };
        self.quick_send(
            "ZCtrl.Withdraw",
            &[NanonisValue::U32(wait_flag), NanonisValue::I32(timeout_ms)],
            &["I", "i"],
            &[],
        )?;
        Ok(())
    }

    // ==================== FolMe Functions ====================

    /// Get current X,Y tip coordinates
    /// Returns the X,Y tip coordinates (oversampled during the Acquisition Period time)
    pub fn folme_xy_pos_get(
        &mut self,
        wait_for_newest_data: bool,
    ) -> Result<Position, NanonisError> {
        let wait_flag = if wait_for_newest_data { 1u32 } else { 0u32 };
        let result = self.quick_send(
            "FolMe.XYPosGet",
            &[NanonisValue::U32(wait_flag)],
            &["I"],
            &["d", "d"],
        )?;

        if result.len() >= 2 {
            let x = result[0].as_f64()?;
            let y = result[1].as_f64()?;
            Ok(Position { x, y })
        } else {
            Err(NanonisError::Protocol(
                "Invalid XY position response".to_string(),
            ))
        }
    }

    /// Set XY position with type safety
    pub fn folme_xy_pos_set(
        &mut self,
        position: Position,
        wait_end: bool,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "FolMe.XYPosSet",
            &[
                NanonisValue::F64(position.x),
                NanonisValue::F64(position.y),
                NanonisValue::U32(if wait_end { 1 } else { 0 }),
            ],
            &["d", "d", "I"],
            &[],
        )?;
        Ok(())
    }

    // ==================== Motor Functions ====================

    /// Move the coarse positioning device (motor, piezo actuator)
    /// Direction: 0=X+, 1=X-, 2=Y+, 3=Y-, 4=Z+, 5=Z-
    /// Group: 0=Group1, 1=Group2, 2=Group3, 3=Group4, 4=Group5, 5=Group6
    pub fn motor_start_move(
        &mut self,
        direction: u32,
        number_of_steps: u16,
        group: u32,
        wait_until_finished: bool,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u32 } else { 0u32 };
        self.quick_send(
            "Motor.StartMove",
            &[
                NanonisValue::U32(direction),
                NanonisValue::U16(number_of_steps),
                NanonisValue::U32(group),
                NanonisValue::U32(wait_flag),
            ],
            &["I", "H", "I", "I"],
            &[],
        )?;
        Ok(())
    }

    /// Move the coarse positioning device in closed loop
    /// absolute_relative: 0=relative, 1=absolute movement
    /// Group: 0=Group1, 1=Group2, 2=Group3, 3=Group4, 4=Group5, 5=Group6
    pub fn motor_start_closed_loop(
        &mut self,
        absolute_relative: u32,
        target_x_m: f64,
        target_y_m: f64,
        target_z_m: f64,
        wait_until_finished: bool,
        group: u32,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u32 } else { 0u32 };
        self.quick_send(
            "Motor.StartClosedLoop",
            &[
                NanonisValue::U32(absolute_relative),
                NanonisValue::F64(target_x_m),
                NanonisValue::F64(target_y_m),
                NanonisValue::F64(target_z_m),
                NanonisValue::U32(wait_flag),
                NanonisValue::U32(group),
            ],
            &["I", "d", "d", "d", "I", "I"],
            &[],
        )?;
        Ok(())
    }

    /// Stop the motor motion
    pub fn motor_stop_move(&mut self) -> Result<(), NanonisError> {
        self.quick_send("Motor.StopMove", &[], &[], &[])?;
        Ok(())
    }

    /// Get the positions of the motor control module
    /// Group: 0=Group1, 1=Group2, 2=Group3, 3=Group4, 4=Group5, 5=Group6
    /// timeout_ms: timeout in milliseconds (recommended: 500ms)
    pub fn motor_pos_get(&mut self, group: u32, timeout_ms: u32) -> Result<(f64, f64, f64), NanonisError> {
        let result = self.quick_send(
            "Motor.PosGet",
            &[NanonisValue::U32(group), NanonisValue::U32(timeout_ms)],
            &["I", "I"],
            &["d", "d", "d"],
        )?;

        if result.len() >= 3 {
            let x = result[0].as_f64()?;
            let y = result[1].as_f64()?;
            let z = result[2].as_f64()?;
            Ok((x, y, z))
        } else {
            Err(NanonisError::Protocol("Invalid motor position response".to_string()))
        }
    }

    /// Get step counter values and optionally reset them
    /// Available only on Attocube ANC150 devices
    pub fn motor_step_counter_get(
        &mut self,
        reset_x: bool,
        reset_y: bool,
        reset_z: bool,
    ) -> Result<(i32, i32, i32), NanonisError> {
        let reset_x_flag = if reset_x { 1u32 } else { 0u32 };
        let reset_y_flag = if reset_y { 1u32 } else { 0u32 };
        let reset_z_flag = if reset_z { 1u32 } else { 0u32 };

        let result = self.quick_send(
            "Motor.StepCounterGet",
            &[
                NanonisValue::U32(reset_x_flag),
                NanonisValue::U32(reset_y_flag),
                NanonisValue::U32(reset_z_flag),
            ],
            &["I", "I", "I"],
            &["i", "i", "i"],
        )?;

        if result.len() >= 3 {
            let step_x = result[0].as_i32()?;
            let step_y = result[1].as_i32()?;
            let step_z = result[2].as_i32()?;
            Ok((step_x, step_y, step_z))
        } else {
            Err(NanonisError::Protocol("Invalid step counter response".to_string()))
        }
    }

    /// Get frequency and amplitude of the motor control module
    /// Available only for PD5, PMD4, and Attocube ANC150 devices
    /// axis: 0=Default, 1=X, 2=Y, 3=Z
    pub fn motor_freq_amp_get(&mut self, axis: u16) -> Result<(f32, f32), NanonisError> {
        let result = self.quick_send(
            "Motor.FreqAmpGet",
            &[NanonisValue::U16(axis)],
            &["H"],
            &["f", "f"],
        )?;

        if result.len() >= 2 {
            let frequency = result[0].as_f32()?;
            let amplitude = result[1].as_f32()?;
            Ok((frequency, amplitude))
        } else {
            Err(NanonisError::Protocol("Invalid frequency/amplitude response".to_string()))
        }
    }

    /// Set frequency and amplitude of the motor control module
    /// Available only for PD5, PMD4, and Attocube ANC150 devices
    /// axis: 0=All, 1=X, 2=Y, 3=Z
    pub fn motor_freq_amp_set(&mut self, frequency_hz: f32, amplitude_v: f32, axis: u16) -> Result<(), NanonisError> {
        self.quick_send(
            "Motor.FreqAmpSet",
            &[
                NanonisValue::F32(frequency_hz),
                NanonisValue::F32(amplitude_v),
                NanonisValue::U16(axis),
            ],
            &["f", "f", "H"],
            &[],
        )?;
        Ok(())
    }
}
