use std::time::Duration;

use nanonis_rs::{
    motor::{MotorDirection, MotorDisplacement, MotorGroup, MovementMode, Position3D},
    oscilloscope::OsciData,
    scan::{ScanAction, ScanDirection},
    tip_recovery::TipShaperConfig,
    NanonisClient, Position,
};

use std::collections::HashSet;

use crate::spm_controller::{AcquisitionMode, Capability, DataStreamStatus, Result, SpmController, TriggerSetup};
use crate::spm_error::SpmError;
use crate::utils::{poll_until, PollError};

pub struct NanonisController {
    client: NanonisClient,
}

impl NanonisController {
    pub fn new(client: NanonisClient) -> Self {
        Self { client }
    }

    /// Access the underlying NanonisClient directly for operations
    /// not covered by the SpmController trait.
    pub fn client(&self) -> &NanonisClient {
        &self.client
    }

    pub fn client_mut(&mut self) -> &mut NanonisClient {
        &mut self.client
    }
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
        ])
    }

    // -- Signals --

    fn read_signal(&mut self, index: u32, wait_for_newest: bool) -> Result<f64> {
        let val = self.client.signal_val_get(index as u8, wait_for_newest)?;
        Ok(val as f64)
    }

    fn read_signals(&mut self, indices: &[u32], wait_for_newest: bool) -> Result<Vec<f64>> {
        let indices_i32: Vec<i32> = indices.iter().map(|&i| i as i32).collect();
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
        Ok(self.client.bias_set(voltage as f32)?)
    }

    fn bias_pulse(
        &mut self,
        voltage: f64,
        width: Duration,
        z_hold: bool,
        absolute: bool,
    ) -> Result<()> {
        let z_controller_hold: u16 = if z_hold { 1 } else { 0 };
        let pulse_mode: u16 = if absolute { 2 } else { 1 };
        Ok(self.client.bias_pulse(
            true, // always wait for pulse to complete
            width.as_secs_f32(),
            voltage as f32,
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
        Ok(self.client.z_ctrl_setpoint_set(setpoint as f32)?)
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
        // Decompose 3D displacement into sequential single-axis moves.
        // Each axis is moved independently since motor_start_move only
        // handles one axis at a time.
        if displacement.x != 0 {
            let dir = if displacement.x > 0 {
                MotorDirection::XPlus
            } else {
                MotorDirection::XMinus
            };
            self.client.motor_start_move(
                dir,
                displacement.x.unsigned_abs(),
                MotorGroup::Group1,
                wait,
            )?;
        }
        if displacement.y != 0 {
            let dir = if displacement.y > 0 {
                MotorDirection::YPlus
            } else {
                MotorDirection::YMinus
            };
            self.client.motor_start_move(
                dir,
                displacement.y.unsigned_abs(),
                MotorGroup::Group1,
                wait,
            )?;
        }
        if displacement.z != 0 {
            let dir = if displacement.z > 0 {
                MotorDirection::ZPlus
            } else {
                MotorDirection::ZMinus
            };
            self.client.motor_start_move(
                dir,
                displacement.z.unsigned_abs(),
                MotorGroup::Group1,
                wait,
            )?;
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

    // -- Data Stream (TCP Logger) --

    fn data_stream_configure(&mut self, channels: &[i32], oversampling: i32) -> Result<()> {
        self.client.tcplog_chs_set(channels.to_vec())?;
        self.client.tcplog_oversampl_set(oversampling)?;
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
}
