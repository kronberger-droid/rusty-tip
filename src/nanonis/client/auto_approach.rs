use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::NanonisValue;

impl NanonisClient {
    /// Open the Auto-Approach module
    pub fn auto_approach_open(&mut self) -> Result<(), NanonisError> {
        self.quick_send("AutoApproach.Open", vec![], vec![], vec![])?;
        Ok(())
    }

    /// Start or stop the Z auto-approach procedure
    pub fn auto_approach_on_off_set(&mut self, on_off: bool) -> Result<(), NanonisError> {
        let value = if on_off { 1u16 } else { 0u16 };
        self.quick_send(
            "AutoApproach.OnOffSet",
            vec![NanonisValue::U16(value)],
            vec!["H"],
            vec![],
        )?;
        Ok(())
    }

    /// Get the on-off status of the Z auto-approach procedure
    pub fn auto_approach_on_off_get(&mut self) -> Result<bool, NanonisError> {
        let result = self.quick_send("AutoApproach.OnOffGet", vec![], vec![], vec!["H"])?;
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
        log::debug!("Starting auto-approach...");

        // Open auto-approach module
        self.auto_approach_open()?;

        // Wait a bit for module to initialize
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Start auto-approach
        self.auto_approach_on_off_set(true)?;

        log::debug!("Waiting for auto-approach to complete...");

        // Wait until auto-approach completes
        loop {
            let is_running = self.auto_approach_on_off_get()?;
            if !is_running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        log::debug!("Auto-approach finished");
        Ok(())
    }
}
