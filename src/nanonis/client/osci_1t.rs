use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::NanonisValue;

impl NanonisClient {
    /// Set the channel to display in the Oscilloscope 1-Channel
    /// channel_index: 0-23, corresponds to signals assigned to the 24 slots in the Signals Manager
    pub fn osci1t_ch_set(&mut self, channel_index: i32) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci1T.ChSet",
            vec![NanonisValue::I32(channel_index)],
            vec!["i"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the channel displayed in the Oscilloscope 1-Channel
    /// Returns: channel index (0-23)
    pub fn osci1t_ch_get(&mut self) -> Result<i32, NanonisError> {
        let result = self.quick_send("Osci1T.ChGet", vec![], vec![], vec!["i"])?;
        match result.first() {
            Some(value) => Ok(value.as_i32()?),
            None => Err(NanonisError::Protocol(
                "No channel index returned".to_string(),
            )),
        }
    }

    /// Set the timebase in the Oscilloscope 1-Channel
    /// Use osci1t_timebase_get() first to obtain available timebases, then use the index
    pub fn osci1t_timebase_set(
        &mut self,
        timebase_index: i32,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci1T.TimebaseSet",
            vec![NanonisValue::I32(timebase_index)],
            vec!["i"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the timebase in the Oscilloscope 1-Channel
    /// Returns: (timebase_index, timebases_array)
    pub fn osci1t_timebase_get(&mut self) -> Result<(i32, Vec<f32>), NanonisError> {
        let result = self.quick_send(
            "Osci1T.TimebaseGet",
            vec![],
            vec![],
            vec!["i", "i", "*f"],
        )?;
        if result.len() >= 3 {
            let timebase_index = result[0].as_i32()?;
            let timebases = result[2].as_f32_array()?.to_vec();
            Ok((timebase_index, timebases))
        } else {
            Err(NanonisError::Protocol(
                "Invalid timebase response".to_string(),
            ))
        }
    }

    /// Set the trigger configuration in the Oscilloscope 1-Channel
    /// trigger_mode: 0 = Immediate, 1 = Level, 2 = Auto
    /// trigger_slope: 0 = Falling, 1 = Rising
    pub fn osci1t_trig_set(
        &mut self,
        trigger_mode: u16,
        trigger_slope: u16,
        trigger_level: f64,
        trigger_hysteresis: f64,
    ) -> Result<(), NanonisError> {
        self.quick_send(
            "Osci1T.TrigSet",
            vec![
                NanonisValue::U16(trigger_mode),
                NanonisValue::U16(trigger_slope),
                NanonisValue::F64(trigger_level),
                NanonisValue::F64(trigger_hysteresis),
            ],
            vec!["H", "H", "d", "d"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the trigger configuration in the Oscilloscope 1-Channel
    /// Returns: (trigger_mode, trigger_slope, trigger_level, trigger_hysteresis)
    pub fn osci1t_trig_get(&mut self) -> Result<(u16, u16, f64, f64), NanonisError> {
        let result = self.quick_send(
            "Osci1T.TrigGet",
            vec![],
            vec![],
            vec!["H", "H", "d", "d"],
        )?;
        if result.len() >= 4 {
            let trigger_mode = result[0].as_u16()?;
            let trigger_slope = result[1].as_u16()?;
            let trigger_level = result[2].as_f64()?;
            let trigger_hysteresis = result[3].as_f64()?;
            Ok((
                trigger_mode,
                trigger_slope,
                trigger_level,
                trigger_hysteresis,
            ))
        } else {
            Err(NanonisError::Protocol(
                "Invalid trigger configuration response".to_string(),
            ))
        }
    }

    /// Start the Oscilloscope 1-Channel
    pub fn osci1t_run(&mut self) -> Result<(), NanonisError> {
        self.quick_send("Osci1T.Run", vec![], vec![], vec![])?;
        Ok(())
    }

    /// Get the graph data from the Oscilloscope 1-Channel
    /// data_to_get: 0 = Current, 1 = Next trigger, 2 = Wait 2 triggers
    /// Returns: (t0, dt, size, data_values)
    pub fn osci1t_data_get(
        &mut self,
        data_to_get: u16,
    ) -> Result<(f64, f64, i32, Vec<f64>), NanonisError> {
        let result = self.quick_send(
            "Osci1T.DataGet",
            vec![NanonisValue::U16(data_to_get)],
            vec!["H"],
            vec!["d", "d", "i", "*d"],
        )?;

        if result.len() >= 4 {
            let t0 = result[0].as_f64()?;
            let dt = result[1].as_f64()?;
            let size = result[2].as_i32()?;
            let data = result[3].as_f64_array()?.to_vec();
            Ok((t0, dt, size, data))
        } else {
            Err(NanonisError::Protocol(
                "Invalid oscilloscope 1T data response".to_string(),
            ))
        }
    }
}
