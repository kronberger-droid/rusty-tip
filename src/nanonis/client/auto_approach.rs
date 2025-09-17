use super::NanonisClient;
use crate::error::NanonisError;
use crate::types::{AutoApproachResult, AutoApproachStatus, NanonisValue};
use std::time::{Duration, Instant};

impl NanonisClient {
    /// Open the Auto-Approach module
    pub fn auto_approach_open(&mut self) -> Result<(), NanonisError> {
        self.quick_send("AutoApproach.Open", vec![], vec![], vec![])?;
        Ok(())
    }

    /// Start or stop the Z auto-approach procedure
    pub fn auto_approach_on_off_set(
        &mut self,
        on_off: bool,
    ) -> Result<(), NanonisError> {
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
        let result =
            self.quick_send("AutoApproach.OnOffGet", vec![], vec![], vec!["H"])?;
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

    /// Get current auto-approach status
    pub fn auto_approach_status(&mut self) -> Result<AutoApproachStatus, NanonisError> {
        match self.auto_approach_on_off_get() {
            Ok(true) => Ok(AutoApproachStatus::Running),
            Ok(false) => Ok(AutoApproachStatus::Idle),
            Err(_) => Ok(AutoApproachStatus::Unknown),
        }
    }

    /// Execute auto-approach with timeout and proper error handling
    pub fn auto_approach_with_timeout(
        &mut self,
        wait: bool,
        timeout: Duration,
    ) -> Result<AutoApproachResult, NanonisError> {
        log::debug!("Starting auto-approach (wait: {}, timeout: {:?})", wait, timeout);

        // Check if already running
        match self.auto_approach_status()? {
            AutoApproachStatus::Running => {
                log::warn!("Auto-approach already running");
                return Ok(AutoApproachResult::AlreadyRunning);
            }
            AutoApproachStatus::Unknown => {
                log::warn!("Auto-approach status unknown, attempting to proceed");
            }
            AutoApproachStatus::Idle => {
                log::debug!("Auto-approach is idle, proceeding to start");
            }
        }

        // Open auto-approach module
        if let Err(e) = self.auto_approach_open() {
            log::error!("Failed to open auto-approach module: {}", e);
            return Ok(AutoApproachResult::Failed(format!("Failed to open module: {}", e)));
        }

        // Wait for module initialization
        std::thread::sleep(Duration::from_millis(500));

        // Start auto-approach
        if let Err(e) = self.auto_approach_on_off_set(true) {
            log::error!("Failed to start auto-approach: {}", e);
            return Ok(AutoApproachResult::Failed(format!("Failed to start: {}", e)));
        }

        if !wait {
            log::debug!("Auto-approach started, not waiting for completion");
            return Ok(AutoApproachResult::Success);
        }

        // Wait for completion with timeout
        log::debug!("Waiting for auto-approach to complete...");
        let start_time = Instant::now();
        let poll_interval = Duration::from_millis(100);

        loop {
            // Check timeout
            if start_time.elapsed() >= timeout {
                log::warn!("Auto-approach timed out after {:?}", timeout);
                // Try to stop the auto-approach
                let _ = self.auto_approach_on_off_set(false);
                return Ok(AutoApproachResult::Timeout);
            }

            // Check status
            match self.auto_approach_status() {
                Ok(AutoApproachStatus::Idle) => {
                    log::debug!("Auto-approach completed successfully");
                    return Ok(AutoApproachResult::Success);
                }
                Ok(AutoApproachStatus::Running) => {
                    // Still running, continue waiting
                    log::trace!("Auto-approach still running, waiting...");
                }
                Ok(AutoApproachStatus::Unknown) => {
                    log::warn!("Auto-approach status unknown during execution");
                    return Ok(AutoApproachResult::Failed(
                        "Lost communication during auto-approach".to_string(),
                    ));
                }
                Err(e) => {
                    log::error!("Error checking auto-approach status: {}", e);
                    return Ok(AutoApproachResult::Failed(format!("Status check error: {}", e)));
                }
            }

            std::thread::sleep(poll_interval);
        }
    }

    /// Auto-approach and wait until completion (legacy compatibility)
    pub fn auto_approach_and_wait(&mut self) -> Result<(), NanonisError> {
        let result = self.auto_approach_with_timeout(true, Duration::from_secs(300))?;
        match result {
            AutoApproachResult::Success => Ok(()),
            _ => Err(NanonisError::InvalidCommand(format!(
                "Auto-approach failed: {}",
                result.error_message().unwrap_or("Unknown error")
            ))),
        }
    }
}
