use std::collections::HashMap;
use std::time::Duration;

use nanonis_rs::{
    motor::{MotorDirection, MotorDisplacement, MotorGroup, MovementMode, Position3D},
    oscilloscope::OsciData,
    scan::{ScanAction, ScanConfig, ScanDirection, ScanProps, ScanPropsBuilder},
    tip_recovery::TipShaperConfig,
    NanonisClient, Position,
};

use std::collections::HashSet;

use crate::buffered_tcp_reader::BufferedTCPReader;
use crate::spm_controller::{AcquisitionMode, Capability, DataStreamStatus, Result, SpmController, TriggerSetup, ZControllerStatus, ZHomeMode};
use crate::spm_error::SpmError;
use crate::utils::{poll_until, PollError};

/// Configuration consumed by `NanonisController::prepare()`.
///
/// Captures all the vendor-specific setup values that `prepare` needs so
/// the binary doesn't have to poke the controller directly.
pub struct NanonisSetupConfig {
    /// Nanonis layout file to load (absolute or relative path). `None` to skip.
    pub layout_file: Option<String>,
    /// Nanonis settings file to load. `None` to skip.
    pub settings_file: Option<String>,
    /// Z-controller home mode.
    pub z_home_mode: ZHomeMode,
    /// Z-controller home position in metres.
    pub z_home_position_m: f64,
    /// Safe-tip current threshold in amperes.
    pub safe_tip_threshold_a: f64,
    /// Which User Output index to toggle for the TCP channel list refresh
    /// workaround.  `None` skips the workaround entirely.  Default is
    /// `Some(3)`.  Pick an output that is not driving anything critical.
    pub tcp_refresh_output: Option<i32>,
}

impl Default for NanonisSetupConfig {
    fn default() -> Self {
        Self {
            layout_file: None,
            settings_file: None,
            z_home_mode: ZHomeMode::Absolute,
            z_home_position_m: 50e-9,
            safe_tip_threshold_a: 1e-9,
            tcp_refresh_output: Some(3),
        }
    }
}

pub struct NanonisController {
    client: NanonisClient,
    setup: NanonisSetupConfig,
    tcp_reader: Option<BufferedTCPReader>,
    /// Maps Nanonis signal index -> position in SignalFrame.data array.
    /// Set by the caller via `set_channel_mapping` before starting the TCP reader.
    signal_to_data_position: HashMap<u32, usize>,
    /// Number of channels configured in the TCP data stream.
    /// Set by `data_stream_configure`, used by `start_tcp_reader`.
    configured_channel_count: Option<u32>,
    /// Guards against double-teardown (manual call + Drop).
    torn_down: bool,
}

impl NanonisController {
    pub fn new(client: NanonisClient, setup: NanonisSetupConfig) -> Self {
        Self {
            client,
            setup,
            tcp_reader: None,
            signal_to_data_position: HashMap::new(),
            configured_channel_count: None,
            torn_down: false,
        }
    }

    /// Access the underlying NanonisClient directly for operations
    /// not covered by the SpmController trait.
    pub fn client(&self) -> &NanonisClient {
        &self.client
    }

    pub fn client_mut(&mut self) -> &mut NanonisClient {
        &mut self.client
    }

    /// Collect `num_samples` data points from the TCP stream for a given data position.
    ///
    /// Uses timestamp tracking to avoid re-reading already-seen frames.
    /// Timeout scales with sample count (minimum 5s, +1s per 100 samples).
    fn collect_tcp_samples(
        reader: &BufferedTCPReader,
        data_position: usize,
        num_samples: usize,
    ) -> Result<Vec<f32>> {
        let timeout_secs = 5 + (num_samples as u64 / 100);
        let timeout = Duration::from_secs(timeout_secs);
        let start = std::time::Instant::now();
        let mut collected: Vec<f32> = Vec::with_capacity(num_samples);

        // Track the timestamp of the last consumed frame to avoid duplicates.
        let mut cursor = std::time::Instant::now();

        while collected.len() < num_samples && start.elapsed() < timeout {
            // Check if the stream reader thread died before waiting the
            // full timeout — provides an immediate, descriptive error.
            if !reader.is_buffering() {
                if let Some(err_msg) = reader.stream_error() {
                    return Err(SpmError::Io {
                        source: std::io::Error::new(
                            std::io::ErrorKind::ConnectionAborted,
                            err_msg.clone(),
                        ),
                        context: format!(
                            "TCP data stream died during sample collection: {err_msg}"
                        ),
                    });
                }
            }

            let new_frames = reader.get_data_since(cursor);
            for frame in &new_frames {
                cursor = frame.timestamp + Duration::from_nanos(1);
                if let Some(&value) = frame.signal_frame.data.get(data_position) {
                    collected.push(value);
                    if collected.len() >= num_samples {
                        break;
                    }
                }
            }
            if collected.len() < num_samples {
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        if collected.is_empty() {
            return Err(SpmError::Timeout(
                "No TCP stream data collected within timeout".into(),
            ));
        }

        if collected.len() < num_samples {
            log::warn!(
                "TCP sample collection: requested {} samples but only got {} within timeout",
                num_samples,
                collected.len()
            );
        }

        Ok(collected)
    }

    /// Set the mapping from Nanonis signal indices to TCP data array positions.
    ///
    /// The caller should compute this from their `SignalRegistry`: for each signal
    /// of interest, find its `tcp_channel` and then its position in the configured
    /// channel list (the order passed to `data_stream_configure`).
    pub fn set_channel_mapping(&mut self, mapping: HashMap<u32, usize>) {
        self.signal_to_data_position = mapping;
        log::debug!(
            "Channel mapping set: {} signals mapped to data positions",
            self.signal_to_data_position.len()
        );
    }

    /// Start the background TCP data stream for stable signal reading.
    ///
    /// Call `set_channel_mapping` and `data_stream_configure` before this.
    /// Connects to the TCP logger data port and spawns the background
    /// buffering thread.
    pub fn start_tcp_reader(
        &mut self,
        host: &str,
        data_port: u16,
        buffer_size: usize,
    ) -> Result<()> {
        if self.tcp_reader.is_some() {
            log::warn!("TCP reader already running, stopping previous instance");
            self.stop_tcp_reader()?;
        }

        let num_channels = self.configured_channel_count.ok_or_else(|| {
            SpmError::Protocol(
                "No channels configured. Call data_stream_configure before start_tcp_reader".into(),
            )
        })?;

        if num_channels == 0 {
            return Err(SpmError::Protocol(
                "Channel count is zero. Configure at least one channel".into(),
            ));
        }

        let reader = BufferedTCPReader::new(
            host,
            data_port,
            buffer_size,
            num_channels,
            1.0,
        )
        .map_err(|e| SpmError::Protocol(format!("Failed to start TCP reader: {}", e)))?;

        self.tcp_reader = Some(reader);
        log::info!(
            "TCP reader started on {}:{} with {} channels",
            host, data_port, num_channels
        );
        Ok(())
    }

    /// Stop the background TCP reader if running.
    pub fn stop_tcp_reader(&mut self) -> Result<()> {
        if let Some(mut reader) = self.tcp_reader.take() {
            reader
                .stop()
                .map_err(|e| SpmError::Protocol(format!("Failed to stop TCP reader: {}", e)))?;
        }
        Ok(())
    }

    /// Clear the TCP reader buffer (discard stale data before a fresh measurement).
    pub fn clear_tcp_buffer(&self) {
        if let Some(reader) = &self.tcp_reader {
            reader.clear_buffer();
        }
    }

    /// Nanonis workaround: toggle a User Output mode to force the TCP
    /// channel list to refresh.  Works around a known Nanonis bug where
    /// signal slot assignments are stale until any User Output is modified.
    ///
    /// `output_index` selects which User Output to toggle.  Pick one that
    /// is not driving anything critical (default: 3).
    fn refresh_tcp_channel_list(&mut self, output_index: i32) -> Result<()> {
        use nanonis_rs::user_out::OutputMode;

        let current_mode = self.client.user_out_mode_get(output_index)
            .map_err(|e| SpmError::Protocol(format!("user_out_mode_get failed: {}", e)))?;
        let toggle_to = match current_mode {
            OutputMode::UserOutput => OutputMode::Monitor,
            OutputMode::Monitor => OutputMode::CalcSignal,
            _ => OutputMode::Monitor,
        };
        self.client.user_out_mode_set(output_index, toggle_to)
            .map_err(|e| SpmError::Protocol(format!("user_out_mode_set failed: {}", e)))?;
        self.client.user_out_mode_set(output_index, current_mode)
            .map_err(|e| SpmError::Protocol(format!("user_out_mode_set failed: {}", e)))?;
        log::debug!("TCP channel list refresh workaround applied (output {})", output_index);
        Ok(())
    }
}

/// Validate that an f64 value is finite and representable as f32.
///
/// Rejects NaN, infinity, and values that overflow f32.  Warns if the
/// value underflows to zero or becomes subnormal in f32, since this
/// almost always indicates a unit mismatch in SPM parameters.
fn validate_f32(value: f64, name: &str) -> Result<f32> {
    if !value.is_finite() {
        return Err(SpmError::Protocol(format!(
            "{} must be finite, got {}",
            name, value
        )));
    }
    let v = value as f32;
    if !v.is_finite() {
        return Err(SpmError::Protocol(format!(
            "{} value {} overflows f32",
            name, value
        )));
    }
    if value != 0.0 && (v == 0.0 || v.is_subnormal()) {
        log::warn!(
            "{} value {} underflows to f32 {} (possible unit mismatch?)",
            name, value, v
        );
    }
    Ok(v)
}

impl SpmController for NanonisController {
    fn capabilities(&self) -> HashSet<Capability> {
        HashSet::from([
            Capability::Signals,
            Capability::Bias,
            Capability::ZController,
            Capability::PiezoPosition,
            Capability::Motor,
            Capability::Scanning,
            Capability::Oscilloscope,
            Capability::TipShaper,
            Capability::Pll,
            Capability::DataStream,
            Capability::SafeTip,
        ])
    }

    // -- Lifecycle --

    fn prepare(&mut self) -> Result<()> {
        // Load layout file if specified
        if let Some(ref path) = self.setup.layout_file {
            let abs = std::path::Path::new(path)
                .canonicalize()
                .map_err(|e| SpmError::Protocol(format!("Layout file not found: {} ({})", path, e)))?;
            self.client.util_layout_load(&abs.to_string_lossy(), false)?;
            log::info!("Layout loaded: {}", abs.display());
        }

        // Load settings file if specified
        if let Some(ref path) = self.setup.settings_file {
            let abs = std::path::Path::new(path)
                .canonicalize()
                .map_err(|e| SpmError::Protocol(format!("Settings file not found: {} ({})", path, e)))?;
            self.client.util_settings_load(&abs.to_string_lossy(), false)?;
            log::info!("Settings loaded: {}", abs.display());
        }

        // Z-controller home position
        self.set_z_home(self.setup.z_home_mode, self.setup.z_home_position_m)?;
        log::info!(
            "Z home: mode={:?}, pos={:.0} nm",
            self.setup.z_home_mode,
            self.setup.z_home_position_m * 1e9
        );

        // Safe-tip protection (auto_recovery off, auto_pause_scan on)
        self.safe_tip_configure(false, true, self.setup.safe_tip_threshold_a)?;
        log::info!("Safe-tip threshold: {:.2e} A", self.setup.safe_tip_threshold_a);

        // Nanonis workaround: toggle a User Output mode to refresh TCP channel list
        if let Some(output_index) = self.setup.tcp_refresh_output {
            self.refresh_tcp_channel_list(output_index)?;
        }

        Ok(())
    }

    fn teardown(&mut self) {
        if self.torn_down {
            return;
        }
        self.torn_down = true;

        if let Err(e) = self.data_stream_stop() {
            log::warn!("Data stream stop: {}", e);
        }
        if let Err(e) = self.stop_tcp_reader() {
            log::warn!("TCP reader stop: {}", e);
        }
        // Disable safe-tip overrides entirely: auto_recovery=false,
        // auto_pause_scan=false.  Keep the threshold from config so if
        // the user re-enables safe-tip manually, it starts at a known level.
        if let Err(e) = self.safe_tip_configure(false, false, self.setup.safe_tip_threshold_a) {
            log::warn!("Failed to reset safe-tip config: {}", e);
        }
    }

    fn is_connected(&self) -> bool {
        !self.client.is_poisoned()
    }

    fn reconnect(&mut self) -> Result<()> {
        log::info!("Attempting to reconnect to Nanonis...");
        self.client.reconnect()?;
        log::info!("Reconnected successfully");
        Ok(())
    }

    // -- Signals --

    fn read_signal(&mut self, index: u32, wait_for_newest: bool) -> Result<f64> {
        let index_u8 = u8::try_from(index).map_err(|_| {
            SpmError::Protocol(format!("Signal index {} exceeds u8 range (max 255)", index))
        })?;
        let val = self.client.signal_val_get(index_u8, wait_for_newest)?;
        Ok(val as f64)
    }

    fn read_signals(&mut self, indices: &[u32], wait_for_newest: bool) -> Result<Vec<f64>> {
        let indices_i32: Vec<i32> = indices
            .iter()
            .map(|&i| {
                i32::try_from(i).map_err(|_| {
                    SpmError::Protocol(format!("Signal index {} exceeds i32 range", i))
                })
            })
            .collect::<Result<Vec<i32>>>()?;
        let vals = self.client.signals_vals_get(indices_i32, wait_for_newest)?;
        Ok(vals.into_iter().map(|v| v as f64).collect())
    }

    fn signal_names(&mut self) -> Result<Vec<String>> {
        Ok(self.client.signal_names_get()?)
    }

    // -- Bias --

    fn get_bias(&mut self) -> Result<f64> {
        Ok(self.client.bias_get()? as f64)
    }

    fn set_bias(&mut self, voltage: f64) -> Result<()> {
        let v = validate_f32(voltage, "bias voltage")?;
        Ok(self.client.bias_set(v)?)
    }

    fn bias_pulse(
        &mut self,
        voltage: f64,
        width: Duration,
        z_hold: bool,
        absolute: bool,
    ) -> Result<()> {
        let z_controller_hold = if z_hold {
            nanonis_rs::z_ctrl::ZControllerHold::Hold
        } else {
            nanonis_rs::z_ctrl::ZControllerHold::NoChange
        };
        let pulse_mode = if absolute {
            nanonis_rs::bias::PulseMode::Absolute
        } else {
            nanonis_rs::bias::PulseMode::Relative
        };
        let v = validate_f32(voltage, "pulse voltage")?;
        Ok(self.client.bias_pulse(
            true, // always wait for pulse to complete
            width.as_secs_f32(),
            v,
            z_controller_hold,
            pulse_mode,
        )?)
    }

    // -- Z-Controller --

    fn withdraw(&mut self, wait: bool, timeout: Duration) -> Result<()> {
        Ok(self.client.z_ctrl_withdraw(wait, timeout)?)
    }

    fn auto_approach(&mut self, wait: bool, timeout: Duration) -> Result<()> {
        // Check if already running
        match self.client.auto_approach_on_off_get() {
            Ok(true) => {
                log::warn!("Auto-approach already running");
                return Ok(());
            }
            Ok(false) => {}
            Err(e) => {
                log::warn!("Could not check auto-approach status: {}, proceeding anyway", e);
            }
        }

        // Open the module (ignore error if already open)
        match self.client.auto_approach_open() {
            Ok(_) => log::debug!("Opened auto-approach module"),
            Err(_) => log::debug!("Auto-approach module already open"),
        }

        // Wait for module initialization
        std::thread::sleep(Duration::from_millis(500));

        // Start auto-approach
        self.client.auto_approach_on_off_set(true).map_err(|e| {
            SpmError::Protocol(format!("Failed to start auto-approach: {}", e))
        })?;

        if !wait {
            return Ok(());
        }

        // Poll until approach completes or timeout
        let poll_interval = Duration::from_millis(100);
        match poll_until(
            || self.client.auto_approach_on_off_get().map(|running| !running),
            timeout,
            poll_interval,
        ) {
            Ok(()) => {
                log::debug!("Auto-approach completed successfully");
                Ok(())
            }
            Err(PollError::Timeout) => {
                log::warn!("Auto-approach timed out after {:?}", timeout);
                let _ = self.client.auto_approach_on_off_set(false);
                Err(SpmError::Timeout("Auto-approach timed out".to_string()))
            }
            Err(PollError::ConditionError(e)) => {
                log::error!("Auto-approach polling error: {}", e);
                Err(SpmError::from(e))
            }
        }
    }

    fn set_z_setpoint(&mut self, setpoint: f64) -> Result<()> {
        let s = validate_f32(setpoint, "Z setpoint")?;
        Ok(self.client.z_ctrl_setpoint_set(s)?)
    }

    fn set_z_home(&mut self, mode: ZHomeMode, position: f64) -> Result<()> {
        let p = validate_f32(position, "Z home position")?;
        Ok(self.client.z_ctrl_home_props_set(mode, p)?)
    }

    fn go_z_home(&mut self) -> Result<()> {
        Ok(self.client.z_ctrl_home()?)
    }

    fn z_controller_status(&mut self) -> Result<ZControllerStatus> {
        Ok(self.client.z_ctrl_status_get()?)
    }

    // -- Piezo Positioning (FolMe) --

    fn get_position(&mut self, wait_for_newest: bool) -> Result<Position> {
        Ok(self.client.folme_xy_pos_get(wait_for_newest)?)
    }

    fn set_position(&mut self, pos: Position, wait: bool) -> Result<()> {
        Ok(self.client.folme_xy_pos_set(pos, wait)?)
    }

    // -- Motor (Coarse Positioning) --

    fn move_motor(
        &mut self,
        direction: MotorDirection,
        steps: u16,
        wait: bool,
    ) -> Result<()> {
        Ok(self.client.motor_start_move(direction, steps, MotorGroup::Group1, wait)?)
    }

    fn move_motor_3d(
        &mut self,
        displacement: MotorDisplacement,
        wait: bool,
    ) -> Result<()> {
        let axes: [(i16, MotorDirection, MotorDirection); 3] = [
            (displacement.x, MotorDirection::XPlus, MotorDirection::XMinus),
            (displacement.y, MotorDirection::YPlus, MotorDirection::YMinus),
            (displacement.z, MotorDirection::ZPlus, MotorDirection::ZMinus),
        ];
        for (steps, positive, negative) in axes {
            if steps != 0 {
                let dir = if steps > 0 { positive } else { negative };
                self.client.motor_start_move(
                    dir,
                    steps.unsigned_abs(),
                    MotorGroup::Group1,
                    wait,
                )?;
            }
        }
        Ok(())
    }

    fn move_motor_closed_loop(
        &mut self,
        target: Position3D,
        mode: MovementMode,
    ) -> Result<()> {
        Ok(self
            .client
            .motor_start_closed_loop(mode, target, true, MotorGroup::Group1)?)
    }

    fn stop_motor(&mut self) -> Result<()> {
        Ok(self.client.motor_stop_move()?)
    }

    // -- Scanning --

    fn scan_action(&mut self, action: ScanAction, direction: ScanDirection) -> Result<()> {
        Ok(self.client.scan_action(action, direction)?)
    }

    fn scan_status(&mut self) -> Result<bool> {
        Ok(self.client.scan_status_get()?)
    }

    fn scan_props_get(&mut self) -> Result<ScanProps> {
        Ok(self.client.scan_props_get()?)
    }

    fn scan_props_set(&mut self, props: ScanPropsBuilder) -> Result<()> {
        Ok(self.client.scan_props_set(props)?)
    }

    fn scan_speed_get(&mut self) -> Result<ScanConfig> {
        Ok(self.client.scan_speed_get()?)
    }

    fn scan_speed_set(&mut self, config: ScanConfig) -> Result<()> {
        Ok(self.client.scan_config_set(config)?)
    }

    fn scan_frame_data_grab(
        &mut self,
        channel_index: u32,
        forward: bool,
    ) -> Result<(String, Vec<Vec<f32>>, bool)> {
        Ok(self.client.scan_frame_data_grab(channel_index, forward)?)
    }

    // -- Oscilloscope --

    fn osci_read(
        &mut self,
        channel: i32,
        trigger: Option<&TriggerSetup>,
        mode: AcquisitionMode,
    ) -> Result<OsciData> {
        self.client.osci1t_ch_set(channel)?;

        if let Some(trig) = trigger {
            self.client.osci1t_trig_set(
                trig.mode.into(),
                trig.slope.into(),
                trig.level,
                trig.hysteresis,
            )?;
        }

        self.client.osci1t_run()?;

        let data_to_get: u16 = match mode {
            AcquisitionMode::Current => 0,
            AcquisitionMode::NextTrigger => 1,
            AcquisitionMode::WaitTwoTriggers => 2,
        };
        let (t0, dt, size, data) = self.client.osci1t_data_get(data_to_get)?;

        Ok(OsciData::new(t0, dt, size, data))
    }

    // -- Tip Shaper --

    fn tip_shaper(
        &mut self,
        config: &TipShaperConfig,
        wait: bool,
        timeout: Duration,
    ) -> Result<()> {
        self.client.tip_shaper_props_set(config.clone())?;
        Ok(self.client.tip_shaper_start(wait, timeout)?)
    }

    // -- PLL --

    fn pll_center_freq_shift(&mut self) -> Result<()> {
        // Modulator index 1 is the standard PLL modulator
        Ok(self.client.pll_freq_shift_auto_center(1)?)
    }

    // -- Safe Tip --

    fn safe_tip_configure(
        &mut self,
        auto_recovery: bool,
        auto_pause_scan: bool,
        threshold: f64,
    ) -> Result<()> {
        let t = validate_f32(threshold, "safe-tip threshold")?;
        Ok(self.client.safe_tip_props_set(auto_recovery, auto_pause_scan, t)?)
    }

    fn safe_tip_status(&mut self) -> Result<(bool, bool, f64)> {
        let (recovery, pause, threshold) = self.client.safe_tip_props_get()?;
        Ok((recovery, pause, threshold as f64))
    }

    fn safe_tip_set_enabled(&mut self, enabled: bool) -> Result<()> {
        Ok(self.client.safe_tip_on_off_set(enabled)?)
    }

    fn safe_tip_enabled(&mut self) -> Result<bool> {
        Ok(self.client.safe_tip_on_off_get()?)
    }

    // -- Data Stream (TCP Logger) --

    fn data_stream_configure(&mut self, channels: &[i32], oversampling: i32) -> Result<()> {
        if channels.is_empty() {
            return Err(SpmError::Protocol(
                "data_stream_configure: at least one channel is required".into(),
            ));
        }
        self.client.tcplog_chs_set(channels.to_vec())?;
        self.client.tcplog_oversampl_set(oversampling)?;
        self.configured_channel_count = Some(channels.len() as u32);
        Ok(())
    }

    fn data_stream_start(&mut self) -> Result<()> {
        Ok(self.client.tcplog_start()?)
    }

    fn data_stream_stop(&mut self) -> Result<()> {
        Ok(self.client.tcplog_stop()?)
    }

    fn data_stream_status(&mut self) -> Result<DataStreamStatus> {
        Ok(self.client.tcplog_status_get()?)
    }

    fn clear_data_buffer(&mut self) {
        self.clear_tcp_buffer();
    }

    // -- Signal Reading (TCP stream override) --

    fn read_signal_samples(
        &mut self,
        index: u32,
        num_samples: usize,
    ) -> Result<Vec<f64>> {
        if num_samples == 0 {
            return Err(SpmError::Protocol(
                "read_signal_samples: num_samples must be > 0".into(),
            ));
        }
        let reader = match &self.tcp_reader {
            Some(r) => r,
            None => {
                // Fall back to polling read_signal if no TCP reader
                log::debug!("No TCP reader available, falling back to polling read_signal");
                let mut samples = Vec::with_capacity(num_samples);
                for _ in 0..num_samples {
                    samples.push(self.read_signal(index, true)?);
                }
                return Ok(samples);
            }
        };

        let &data_position = self.signal_to_data_position.get(&index).ok_or_else(|| {
            SpmError::Protocol(format!(
                "Signal index {} has no TCP channel mapping. \
                 Call set_channel_mapping with a mapping that includes this signal.",
                index
            ))
        })?;

        let collected = Self::collect_tcp_samples(reader, data_position, num_samples)?;
        Ok(collected.into_iter().map(|v| v as f64).collect())
    }
}

impl Drop for NanonisController {
    fn drop(&mut self) {
        self.teardown();
    }
}
