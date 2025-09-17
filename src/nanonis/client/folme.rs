use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::{NanonisValue, Position};

impl NanonisClient {
    /// Get the current x-y position
    pub fn folme_xy_pos_get(
        &mut self,
        wait_for_newest_data: bool,
    ) -> Result<Position, NanonisError> {
        let wait_flag = if wait_for_newest_data { 1u32 } else { 0u32 };
        let result = self.quick_send(
            "FolMe.XYPosGet",
            vec![NanonisValue::U32(wait_flag)],
            vec!["I"],
            vec!["d", "d"],
        )?;

        if result.len() >= 2 {
            Ok(Position {
                x: result[0].as_f64()?,
                y: result[1].as_f64()?,
            })
        } else {
            Err(NanonisError::Protocol(
                "Invalid position response".to_string(),
            ))
        }
    }

    /// Set the x-y position
    pub fn folme_xy_pos_set(
        &mut self,
        position: Position,
        wait_until_finished: bool,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u32 } else { 0u32 };
        self.quick_send(
            "FolMe.XYPosSet",
            vec![
                NanonisValue::F64(position.x),
                NanonisValue::F64(position.y),
                NanonisValue::U32(wait_flag),
            ],
            vec!["d", "d", "I"],
            vec![],
        )?;
        Ok(())
    }
    pub fn folme_speed_set(
        &mut self,
        speed: f32,
        custom_speed: bool,
    ) -> Result<(), NanonisError> {
        let custom_speed_flag = if custom_speed { 1u32 } else { 0u32 };
        self.quick_send(
            "FolMe.Speed",
            vec![
                NanonisValue::F32(speed),
                NanonisValue::U32(custom_speed_flag),
            ],
            vec!["f", "I"],
            vec![],
        )?;
        Ok(())
    }
}
