use crate::action_driver::ActionDriver;
use crate::actions::{Action, ActionResult};
use crate::error::NanonisError;
use crate::types::{Position, SignalIndex};
use log::{debug, info, warn};
use std::time::{Duration, Instant};

/// Simple tip state - matches original controller
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TipState {
    Bad,
    Good,
    Stable,
}

/// Loop types based on tip state - simple and direct
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoopType {
    BadLoop,    // Recovery actions
    GoodLoop,   // Monitoring, building to stable
    StableLoop, // Success condition
}

/// Simple tip controller - minimal replication of original controller/policy behavior
pub struct TipController {
    driver: ActionDriver,
    signal_index: SignalIndex,
    min_bound: f32,
    max_bound: f32,
    // Simple state tracking
    good_count: u32,
    stable_threshold: u32,
    #[allow(dead_code)]
    move_count: u32,
    #[allow(dead_code)]
    max_moves: u32,
}

impl TipController {
    /// Create new tip controller with signal bounds
    pub fn new(
        driver: ActionDriver,
        signal_index: SignalIndex,
        min_bound: f32,
        max_bound: f32,
    ) -> Self {
        Self {
            driver,
            signal_index,
            min_bound,
            max_bound,
            good_count: 0,
            stable_threshold: 3, // 3 consecutive good readings = stable
            move_count: 0,
            max_moves: 10, // Max moves before withdraw/approach
        }
    }

    /// Main control loop - simple read/classify/execute pattern
    pub fn run_loop(&mut self, timeout: Duration) -> Result<TipState, NanonisError> {
        info!("Starting tip control loop (timeout: {:?})", timeout);

        let start = Instant::now();
        let mut cycle = 0;

        // First, verify signal exists to avoid IO errors
        let signal_names = self.driver.execute(Action::ReadSignalNames)?;
        info!("Read Signal Names");
        match signal_names {
            ActionResult::SignalNames(names) => {
                info!("Found {} signals", names.len());
                if names.len() <= self.signal_index.0 as usize {
                    return Err(NanonisError::InvalidCommand(format!(
                        "Signal index {} not available (only {} signals)",
                        self.signal_index.0,
                        names.len()
                    )));
                }
            }
            _ => {
                return Err(NanonisError::Protocol(
                    "Failed to get signal names".to_string(),
                ))
            }
        }

        while start.elapsed() < timeout {
            cycle += 1;

            // 1. Read signal (with error handling)
            let signal = match self.read_signal() {
                Ok(s) => s,
                Err(e) => {
                    warn!("Cycle {}: Failed to read signal: {}", cycle, e);
                    std::thread::sleep(Duration::from_millis(500));
                    continue; // Skip this cycle
                }
            };
            debug!("Cycle {}: Signal = {:.6}", cycle, signal);

            // 2. Classify
            let state = self.classify(signal);
            debug!("Cycle {}: State = {:?}", cycle, state);

            // 3. Execute based on state (original controller behavior)
            match state {
                TipState::Bad => {
                    self.bad_loop(cycle)?; // Execute full recovery sequence
                }
                TipState::Good => {
                    self.good_loop(cycle)?; // Monitor and count
                }
                TipState::Stable => {
                    info!("STABLE achieved after {} cycles!", cycle);
                    return Ok(TipState::Stable);
                }
            }

            std::thread::sleep(Duration::from_millis(500)); // Match original timing
        }

        warn!("Tip control loop timed out");
        Err(NanonisError::InvalidCommand("Loop timeout".to_string()))
    }

    /// Bad loop - execute original controller recovery sequence
    /// Sequence: approach → pulse → withdraw → move → approach → check
    fn bad_loop(&mut self, cycle: u32) -> Result<(), NanonisError> {
        info!("Cycle {}: Executing bad signal recovery sequence", cycle);

        // Reset good count
        self.good_count = 0;

        // Step 1: Initial approach
        info!("Cycle {}: Step 1 - Initial approach", cycle);
        self.driver.execute(Action::AutoApproach)?;

        // Step 2: Pulse operation (simulate like original)
        info!("Cycle {}: Step 2 - Pulse operation", cycle);
        std::thread::sleep(Duration::from_millis(200));
        debug!("Pulse simulation completed");

        // Step 3: Withdraw tip
        info!("Cycle {}: Step 3 - Withdrawing tip", cycle);
        self.driver.execute(Action::Withdraw {
            wait_until_finished: true,
            timeout_ms: 5000,
        })?;

        // Step 4: Move to new position (3nm like original)
        info!("Cycle {}: Step 4 - Moving to new position", cycle);
        self.driver.execute(Action::MovePiezoRelative {
            delta: Position::new(3e-9, 3e-9), // 3nm in both directions
        })?;

        // Step 5: Approach at new position
        info!("Cycle {}: Step 5 - Approach at new position", cycle);
        self.driver.execute(Action::AutoApproach)?;

        // Step 6: Check tip state (will happen in next loop iteration)
        info!(
            "Cycle {}: Recovery sequence completed - checking tip state",
            cycle
        );

        Ok(())
    }

    /// Good loop - monitoring, increment good count
    fn good_loop(&mut self, cycle: u32) -> Result<(), NanonisError> {
        self.good_count += 1;
        debug!("Cycle {}: Good signal (count: {})", cycle, self.good_count);
        // Just wait and continue monitoring
        Ok(())
    }

    /// Simple classification based on bounds
    fn classify(&mut self, signal: f32) -> TipState {
        if signal < self.min_bound || signal > self.max_bound {
            TipState::Bad
        } else if self.good_count >= self.stable_threshold {
            TipState::Stable
        } else {
            TipState::Good
        }
    }

    /// Read signal value
    fn read_signal(&mut self) -> Result<f32, NanonisError> {
        let result = self.driver.execute(Action::ReadSignal {
            signal: self.signal_index,
            wait_for_newest: true,
        })?;

        match result {
            ActionResult::Signals(values) => Ok(values[0].value() as f32),
            _ => Err(NanonisError::Protocol("Invalid signal result".to_string())),
        }
    }
}
