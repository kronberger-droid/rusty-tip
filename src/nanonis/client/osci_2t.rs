use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::NanonisValue;

impl NanonisClient {
    /// Set the channels to display in the Oscilloscope 2-Channels
    /// channel_a_index: 0-23, channel A signal index  
    /// channel_b_index: 0-23, channel B signal index
    pub fn osci2t_ch_set(
        &mut self,
        channel_a_index: i32,
        channel_b_index: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci2T.ChSet",
            vec![
                NanonisValue::I32(channel_a_index),
                NanonisValue::I32(channel_b_index),
            ],
            vec!["i", "i"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the channels displayed in the Oscilloscope 2-Channels
    /// Returns: (channel_a_index, channel_b_index)
    pub fn osci2t_ch_get(&mut self) -> Result<(i32, i32), NanonisError> {
        let result =
            self.quick_send("Osci2T.ChGet", vec![], vec![], vec!["i", "i"])?;
        if result.len() >= 2 {
            let channel_a = result[0].as_i32()?;
            let channel_b = result[1].as_i32()?;
            Ok((channel_a, channel_b))
        } else {
            Err(NanonisError::Protocol(
                "Invalid channel response".to_string(),
            ))
        }
    }

    /// Set the timebase in the Oscilloscope 2-Channels
    /// Use osci2t_timebase_get() first to obtain available timebases, then use the index
    pub fn osci2t_timebase_set(
        &mut self,
        timebase_index: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci2T.TimebaseSet",
            vec![NanonisValue::U16(timebase_index)],
            vec!["H"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the timebase in the Oscilloscope 2-Channels
    /// Returns: (timebase_index, timebases_array)
    pub fn osci2t_timebase_get(&mut self) -> Result<(u16, Vec<f32>), NanonisError> {
        let result = self.quick_send(
            "Osci2T.TimebaseGet",
            vec![],
            vec![],
            vec!["H", "i", "*f"],
        )?;
        if result.len() >= 3 {
            let timebase_index = result[0].as_u16()?;
            let timebases = result[2].as_f32_array()?.to_vec();
            Ok((timebase_index, timebases))
        } else {
            Err(NanonisError::Protocol(
                "Invalid timebase response".to_string(),
            ))
        }
    }

    /// Set the oversampling in the Oscilloscope 2-Channels
    /// oversampling_index: 0=50 samples, 1=20, 2=10, 3=5, 4=2, 5=1 sample (no averaging)
    pub fn osci2t_oversampl_set(
        &mut self,
        oversampling_index: u16,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci2T.OversamplSet",
            vec![NanonisValue::U16(oversampling_index)],
            vec!["H"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the oversampling in the Oscilloscope 2-Channels
    /// Returns: oversampling index (0=50 samples, 1=20, 2=10, 3=5, 4=2, 5=1 sample)
    pub fn osci2t_oversampl_get(&mut self) -> Result<u16, NanonisError> {
        let result =
            self.quick_send("Osci2T.OversamplGet", vec![], vec![], vec!["H"])?;
        match result.first() {
            Some(value) => Ok(value.as_u16()?),
            None => Err(NanonisError::Protocol(
                "No oversampling index returned".to_string(),
            )),
        }
    }

    /// Set the trigger configuration in the Oscilloscope 2-Channels
    /// trigger_mode: 0 = Immediate, 1 = Level, 2 = Auto
    /// trig_channel: trigger channel
    /// trigger_slope: 0 = Falling, 1 = Rising
    pub fn osci2t_trig_set(
        &mut self,
        trigger_mode: u16,
        trig_channel: u16,
        trigger_slope: u16,
        trigger_level: f64,
        trigger_hysteresis: f64,
        trig_position: f64,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci2T.TrigSet",
            vec![
                NanonisValue::U16(trigger_mode),
                NanonisValue::U16(trig_channel),
                NanonisValue::U16(trigger_slope),
                NanonisValue::F64(trigger_level),
                NanonisValue::F64(trigger_hysteresis),
                NanonisValue::F64(trig_position),
            ],
            vec!["H", "H", "H", "d", "d", "d"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the trigger configuration in the Oscilloscope 2-Channels
    /// Returns: (trigger_mode, trig_channel, trigger_slope, trigger_level, trigger_hysteresis, trig_position)
    pub fn osci2t_trig_get(
        &mut self,
    ) -> Result<(u16, u16, u16, f64, f64, f64), NanonisError> {
        let result = self.quick_send(
            "Osci2T.TrigGet",
            vec![],
            vec![],
            vec!["H", "H", "H", "d", "d", "d"],
        )?;
        if result.len() >= 6 {
            let trigger_mode = result[0].as_u16()?;
            let trig_channel = result[1].as_u16()?;
            let trigger_slope = result[2].as_u16()?;
            let trigger_level = result[3].as_f64()?;
            let trigger_hysteresis = result[4].as_f64()?;
            let trig_position = result[5].as_f64()?;
            Ok((
                trigger_mode,
                trig_channel,
                trigger_slope,
                trigger_level,
                trigger_hysteresis,
                trig_position,
            ))
        } else {
            Err(NanonisError::Protocol(
                "Invalid trigger configuration response".to_string(),
            ))
        }
    }

    /// Start the Oscilloscope 2-Channels
    pub fn osci2t_run(&mut self) -> Result<(), NanonisError> {
        self.quick_send("Osci2T.Run", vec![], vec![], vec![])?;
        Ok(())
    }

    /// Get the graph data from the Oscilloscope 2-Channels
    /// data_to_get: 0 = Current, 1 = Next trigger, 2 = Wait 2 triggers
    /// Returns: (t0, dt, channel_a_data, channel_b_data)
    pub fn osci2t_data_get(
        &mut self,
        data_to_get: u16,
    ) -> Result<(f64, f64, Vec<f64>, Vec<f64>), NanonisError> {
        let result = self.quick_send(
            "Osci2T.DataGet",
            vec![NanonisValue::U16(data_to_get)],
            vec!["H"],
            vec!["d", "d", "i", "*d", "i", "*d"],
        )?;

        if result.len() >= 6 {
            let t0 = result[0].as_f64()?;
            let dt = result[1].as_f64()?;
            let channel_a_data = result[3].as_f64_array()?.to_vec();
            let channel_b_data = result[5].as_f64_array()?.to_vec();
            Ok((t0, dt, channel_a_data, channel_b_data))
        } else {
            Err(NanonisError::Protocol(
                "Invalid oscilloscope 2T data response".to_string(),
            ))
        }
    }
}
