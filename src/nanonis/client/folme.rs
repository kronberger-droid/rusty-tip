use crate::error::NanonisError;
use crate::types::{NanonisValue, Position};
use super::NanonisClient;

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
            vec!["f", "f"],
        )?;

        if result.len() >= 2 {
            Ok(Position {
                x: result[0].as_f32()? as f64,
                y: result[1].as_f32()? as f64,
            })
        } else {
            Err(NanonisError::Protocol("Invalid position response".to_string()))
        }
    }

    /// Set the x-y position
    pub fn folme_xy_pos_set(
        &mut self,
        position: Position,
        wait_until_finished: bool,
    ) -> Result<(), NanonisError> {
        let wait_flag = if wait_until_finished { 1u16 } else { 0u16 };
        self.quick_send(
            "FolMe.XYPosSet",
            vec![
                NanonisValue::F32(position.x as f32),
                NanonisValue::F32(position.y as f32),
                NanonisValue::U16(wait_flag),
            ],
            vec!["f", "f", "H"],
            vec![],
        )?;
        Ok(())
    }
}