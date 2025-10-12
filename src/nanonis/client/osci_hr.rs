use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::{
    NanonisValue, OscilloscopeIndex, SampleCount, SignalIndex, TriggerLevel,
    TriggerMode,
};

impl NanonisClient {
    /// Set the measured signal index of the selected channel from the Oscilloscope High Resolution
    pub fn osci_hr_ch_set(
        &mut self,
        osci_index: impl Into<OscilloscopeIndex>,
        signal_index: impl Into<SignalIndex>,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.ChSet",
            vec![
                NanonisValue::I32(osci_index.into().into()),
                NanonisValue::I32(signal_index.into().into()),
            ],
            vec!["i", "i"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the measured signal index of the selected channel from the Oscilloscope High Resolution
    pub fn osci_hr_ch_get(
        &mut self,
        osci_index: impl Into<OscilloscopeIndex>,
    ) -> Result<SignalIndex, NanonisError> {
        let result = self.quick_send(
            "OsciHR.ChGet",
            vec![NanonisValue::I32(osci_index.into().into())],
            vec!["i"],
            vec!["i"],
        )?;
        match result.first() {
            Some(value) => Ok(SignalIndex::new(value.as_i32()? as u8)),
            None => Err(NanonisError::Protocol(
                "No signal index returned".to_string(),
            )),
        }
    }

    /// Set the oversampling index of the Oscilloscope High Resolution
    pub fn osci_hr_oversampl_set(
        &mut self,
        oversampling_index: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.OversamplSet",
            vec![NanonisValue::I32(oversampling_index)],
            vec!["i"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the oversampling index of the Oscilloscope High Resolution
    pub fn osci_hr_oversampl_get(&mut self) -> Result<i32, NanonisError> {
        let result =
            self.quick_send("OsciHR.OversamplGet", vec![], vec![], vec!["i"])?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No oversampling index returned".to_string(),
            )),
        }
    }

    /// Set the calibration mode of the selected channel from the Oscilloscope High Resolution
    /// calibration_mode: 0 = Raw values, 1 = Calibrated values
    pub fn osci_hr_calibr_mode_set(
        &mut self,
        osci_index: i32,
        calibration_mode: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.CalibrModeSet",
            vec![
                NanonisValue::I32(osci_index),
                NanonisValue::U16(calibration_mode),
            ],
            vec!["i", "H"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the calibration mode of the selected channel from the Oscilloscope High Resolution
    /// Returns: 0 = Raw values, 1 = Calibrated values
    pub fn osci_hr_calibr_mode_get(
        &mut self,
        osci_index: i32,
    ) -> Result<u16, NanonisError> {
        let result = self.quick_send(
            "OsciHR.CalibrModeGet",
            vec![NanonisValue::I32(osci_index)],
            vec!["i"],
            vec!["H"],
        )?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No calibration mode returned".to_string(),
            )),
        }
    }

    /// Set the number of samples to acquire in the Oscilloscope High Resolution
    pub fn osci_hr_samples_set(
        &mut self,
        number_of_samples: impl Into<SampleCount>,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.SamplesSet",
            vec![NanonisValue::I32(number_of_samples.into().into())],
            vec!["i"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the number of samples to acquire in the Oscilloscope High Resolution
    pub fn osci_hr_samples_get(&mut self) -> Result<SampleCount, NanonisError> {
        let result =
            self.quick_send("OsciHR.SamplesGet", vec![], vec![], vec!["i"])?;
        match result.first() {
            Some(value) => Ok(SampleCount::new(value.as_i32()?)),
            None => Err(NanonisError::Protocol(
                "No sample count returned".to_string(),
            )),
        }
    }

    /// Set the Pre-Trigger Samples or Seconds in the Oscilloscope High Resolution
    pub fn osci_hr_pre_trig_set(
        &mut self,
        pre_trigger_samples: u32,
        pre_trigger_s: f64,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.PreTrigSet",
            vec![
                NanonisValue::U32(pre_trigger_samples),
                NanonisValue::F64(pre_trigger_s),
            ],
            vec!["I", "d"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the Pre-Trigger Samples in the Oscilloscope High Resolution
    pub fn osci_hr_pre_trig_get(&mut self) -> Result<i32, NanonisError> {
        let result =
            self.quick_send("OsciHR.PreTrigGet", vec![], vec![], vec!["i"])?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No pre-trigger samples returned".to_string(),
            )),
        }
    }

    /// Start the Oscilloscope High Resolution module
    pub fn osci_hr_run(&mut self) -> Result<(), NanonisError> {
        self.quick_send("OsciHR.Run", vec![], vec![], vec![])?;
        Ok(())
    }

    /// Get the graph data of the selected channel from the Oscilloscope High Resolution
    /// data_to_get: 0 = Current returns the currently displayed data, 1 = Next trigger waits for the next trigger
    /// Returns: (timestamp, time_delta, data_values, timeout_occurred)
    pub fn osci_hr_osci_data_get(
        &mut self,
        osci_index: i32,
        data_to_get: u16,
        timeout_s: f64,
    ) -> Result<(String, f64, Vec<f32>, bool), NanonisError> {
        let result = self.quick_send(
            "OsciHR.OsciDataGet",
            vec![
                NanonisValue::I32(osci_index),
                NanonisValue::U16(data_to_get),
                NanonisValue::F64(timeout_s),
            ],
            vec!["i", "H", "d"],
            vec!["i", "*-c", "d", "i", "*f", "I"],
        )?;

        if result.len() >= 6 {
            let timestamp = result[1].as_string()?.to_string();
            let time_delta = result[2].as_f64()?;
            let data_values = result[4].as_f32_array()?.to_vec();
            let timeout_occurred = result[5].as_u32()? == 1;
            Ok((timestamp, time_delta, data_values, timeout_occurred))
        } else {
            Err(NanonisError::Protocol(
                "Invalid oscilloscope data response".to_string(),
            ))
        }
    }

    /// Set the trigger mode in the Oscilloscope High Resolution
    pub fn osci_hr_trig_mode_set(
        &mut self,
        trigger_mode: impl Into<TriggerMode>,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigModeSet",
            vec![NanonisValue::U16(trigger_mode.into().into())],
            vec!["H"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the trigger mode in the Oscilloscope High Resolution
    pub fn osci_hr_trig_mode_get(&mut self) -> Result<TriggerMode, NanonisError> {
        let result =
            self.quick_send("OsciHR.TrigModeGet", vec![], vec![], vec!["H"])?;
        match result.first() {
            Some(value) => {
                let mode_val = value.as_u16()?;
                match mode_val {
                    0 => Ok(TriggerMode::Immediate),
                    1 => Ok(TriggerMode::Level),
                    2 => Ok(TriggerMode::Digital),
                    _ => Err(NanonisError::Protocol(format!(
                        "Unknown trigger mode: {}",
                        mode_val
                    ))),
                }
            }
            None => Err(NanonisError::Protocol(
                "No trigger mode returned".to_string(),
            )),
        }
    }

    /// Set the Level Trigger Channel index in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_ch_set(
        &mut self,
        level_trigger_channel_index: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigLevChSet",
            vec![NanonisValue::I32(level_trigger_channel_index)],
            vec!["i"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the Level Trigger Channel index in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_ch_get(&mut self) -> Result<i32, NanonisError> {
        let result =
            self.quick_send("OsciHR.TrigLevChGet", vec![], vec![], vec!["i"])?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No level trigger channel returned".to_string(),
            )),
        }
    }

    /// Set the Level Trigger value in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_val_set(
        &mut self,
        level_trigger_value: impl Into<TriggerLevel>,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigLevValSet",
            vec![NanonisValue::F64(level_trigger_value.into().into())],
            vec!["d"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the Level Trigger value in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_val_get(
        &mut self,
    ) -> Result<TriggerLevel, NanonisError> {
        let result =
            self.quick_send("OsciHR.TrigLevValGet", vec![], vec![], vec!["d"])?;
        match result.first() {
            Some(value) => Ok(TriggerLevel(value.as_f64()?)),
            None => Err(NanonisError::Protocol(
                "No level trigger value returned".to_string(),
            )),
        }
    }

    /// Set the Trigger Arming Mode in the Oscilloscope High Resolution
    pub fn osci_hr_trig_arm_mode_set(
        &mut self,
        trigger_arming_mode: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigArmModeSet",
            vec![NanonisValue::U16(trigger_arming_mode)],
            vec!["H"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the Trigger Arming Mode in the Oscilloscope High Resolution
    pub fn osci_hr_trig_arm_mode_get(&mut self) -> Result<u16, NanonisError> {
        let result =
            self.quick_send("OsciHR.TrigArmModeGet", vec![], vec![], vec!["H"])?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No trigger arming mode returned".to_string(),
            )),
        }
    }

    /// Set the Level Trigger Hysteresis in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_hyst_set(
        &mut self,
        hysteresis: f64,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigLevHystSet",
            vec![NanonisValue::F64(hysteresis)],
            vec!["d"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the Level Trigger Hysteresis in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_hyst_get(&mut self) -> Result<f64, NanonisError> {
        let result =
            self.quick_send("OsciHR.TrigLevHystGet", vec![], vec![], vec!["d"])?;
        match result.first() {
            Some(value) => Ok(value.as_f64()?),
            None => Err(NanonisError::Protocol(
                "No trigger hysteresis returned".to_string(),
            )),
        }
    }

    /// Set the Level Trigger Slope in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_slope_set(
        &mut self,
        slope: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "OsciHR.TrigLevSlopeSet",
            vec![NanonisValue::U16(slope)],
            vec!["H"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the Level Trigger Slope in the Oscilloscope High Resolution
    pub fn osci_hr_trig_lev_slope_get(&mut self) -> Result<u16, NanonisError> {
        let result =
            self.quick_send("OsciHR.TrigLevSlopeGet", vec![], vec![], vec!["H"])?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No trigger slope returned".to_string(),
            )),
        }
    }
}
