use serde::{Deserialize, Serialize};

// Re-export nanonis-rs types instead of redefining them
pub use nanonis_rs::{
    Amplitude, Frequency, MotorAxis, MotorDirection, MotorDisplacement, MotorGroup,
    MotorMovement, MovementMode, NanonisValue, OsciData, OsciTriggerMode, OscilloscopeIndex,
    OversamplingIndex, Position, Position3D, PulseMode, SampleCount, ScanAction, ScanConfig,
    ScanDirection, ScanFrame, SignalFrame, SignalStats, StepCount, TCPLogStatus, TCPLoggerData,
    TimebaseIndex, TriggerConfig, TriggerLevel, TriggerMode, TriggerSlope, ZControllerHold,
};
// DataToGet is extended locally with Stable variant

use std::time::{Duration, Instant};

/// Simple tip shape - matches original controller
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TipShape {
    Blunt,
    Sharp,
    Stable,
}

/// Session metadata for signal tracking
#[derive(Debug, Clone)]
pub struct SessionMetadata {
    pub session_id: String,
    pub signal_names: Vec<String>,   // All signal names
    pub active_indices: Vec<usize>,  // Which signals are being monitored
    pub primary_signal_index: usize, // Index of the primary signal
    pub session_start: f64,          // Session start timestamp
}

/// Extended DataToGet with application-specific Stable variant
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DataToGet {
    Current,
    NextTrigger,
    Wait2Triggers,
    Stable { readings: u32, timeout: Duration },
}

/// Timestamped version of SignalFrame for efficient buffering
#[derive(Debug, Clone)]
pub struct TimestampedSignalFrame {
    /// The lightweight signal frame
    pub signal_frame: SignalFrame,
    /// High-resolution timestamp when frame was received
    pub timestamp: Instant,
    /// Time relative to collection start
    pub relative_time: Duration,
}

impl TimestampedSignalFrame {
    /// Create a new timestamped signal frame from lightweight signal frame
    /// Just adds high-resolution timestamp to existing SignalFrame
    pub fn new(signal_frame: SignalFrame, start_time: Instant) -> Self {
        let timestamp = Instant::now();
        Self {
            signal_frame,
            timestamp,
            relative_time: timestamp.duration_since(start_time),
        }
    }
}

// ==================== Experiment Data with Lightweight Frames ====================

/// Complete experiment result containing action outcome and synchronized signal data
/// Now uses lightweight SignalFrame structures for better memory efficiency
#[derive(Debug)]
pub struct ExperimentData {
    /// Result of the executed action
    pub action_result: crate::actions::ActionResult,
    /// Lightweight signal frames (much more memory efficient)
    pub signal_frames: Vec<TimestampedSignalFrame>,
    /// TCP logger configuration for context (stored once, not per frame)
    pub tcp_config: crate::action_driver::TCPReaderConfig,
    /// When the action started
    pub action_start: Instant,
    /// When the action ended  
    pub action_end: Instant,
    /// Total action duration
    pub total_duration: Duration,
}

/// Experiment data for action chain execution with timing for each action
#[derive(Debug)]
pub struct ChainExperimentData {
    /// Results of each action in the chain
    pub action_results: Vec<crate::actions::ActionResult>,
    /// All signal frames collected during the entire chain execution
    pub signal_frames: Vec<TimestampedSignalFrame>,
    /// TCP logger configuration for context
    pub tcp_config: crate::action_driver::TCPReaderConfig,
    /// Start and end times for each action in the chain
    pub action_timings: Vec<(Instant, Instant)>,
    /// When the entire chain started
    pub chain_start: Instant,
    /// When the entire chain ended
    pub chain_end: Instant,
    /// Duration of entire chain execution
    pub total_duration: Duration,
}

impl ExperimentData {
    /// Get signal data captured during action execution
    pub fn data_during_action(&self) -> Vec<&TimestampedSignalFrame> {
        self.signal_frames
            .iter()
            .filter(|frame| {
                frame.timestamp >= self.action_start && frame.timestamp <= self.action_end
            })
            .collect()
    }

    /// Get signal data before action execution
    pub fn data_before_action(&self, duration: Duration) -> Vec<&TimestampedSignalFrame> {
        let cutoff = self.action_start - duration;
        self.signal_frames
            .iter()
            .filter(|frame| frame.timestamp >= cutoff && frame.timestamp < self.action_start)
            .collect()
    }

    /// Get signal data after action execution
    pub fn data_after_action(&self, duration: Duration) -> Vec<&TimestampedSignalFrame> {
        let cutoff = self.action_end + duration;
        self.signal_frames
            .iter()
            .filter(|frame| frame.timestamp > self.action_end && frame.timestamp <= cutoff)
            .collect()
    }

    /// Get full TCPLoggerData for compatibility when needed
    /// This reconstructs the full data structures using stored TCP config
    pub fn get_tcp_logger_data(&self) -> Vec<TCPLoggerData> {
        self.signal_frames
            .iter()
            .map(|frame| TCPLoggerData {
                num_channels: self.tcp_config.channels.len() as u32,
                oversampling: self.tcp_config.oversampling as f32,
                counter: frame.signal_frame.counter,
                state: TCPLogStatus::Running,
                data: frame.signal_frame.data.clone(),
            })
            .collect()
    }
}

impl ChainExperimentData {
    /// Get signal data captured during a specific action in the chain
    ///
    /// # Arguments
    /// * `action_index` - Index of the action in the chain (0-based)
    ///
    /// # Returns
    /// Vector of signal frames collected during the specified action
    pub fn data_during_action(&self, action_index: usize) -> Vec<&TimestampedSignalFrame> {
        if let Some((start, end)) = self.action_timings.get(action_index) {
            self.signal_frames
                .iter()
                .filter(|frame| frame.timestamp >= *start && frame.timestamp <= *end)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get signal data captured between two actions in the chain
    ///
    /// # Arguments
    /// * `action1_index` - Index of first action (end time)
    /// * `action2_index` - Index of second action (start time)
    ///
    /// # Returns
    /// Vector of signal frames collected between the two specified actions
    pub fn data_between_actions(
        &self,
        action1_index: usize,
        action2_index: usize,
    ) -> Vec<&TimestampedSignalFrame> {
        if let (Some((_, end1)), Some((start2, _))) = (
            self.action_timings.get(action1_index),
            self.action_timings.get(action2_index),
        ) {
            self.signal_frames
                .iter()
                .filter(|frame| frame.timestamp > *end1 && frame.timestamp < *start2)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get signal data before the chain execution
    ///
    /// # Arguments
    /// * `duration` - How far back from chain start to collect data
    ///
    /// # Returns
    /// Vector of signal frames from before the chain started
    pub fn data_before_chain(&self, duration: Duration) -> Vec<&TimestampedSignalFrame> {
        let cutoff = self.chain_start - duration;
        self.signal_frames
            .iter()
            .filter(|frame| frame.timestamp >= cutoff && frame.timestamp < self.chain_start)
            .collect()
    }

    /// Get signal data after the chain execution
    ///
    /// # Arguments
    /// * `duration` - How far forward from chain end to collect data
    ///
    /// # Returns
    /// Vector of signal frames from after the chain ended
    pub fn data_after_chain(&self, duration: Duration) -> Vec<&TimestampedSignalFrame> {
        let cutoff = self.chain_end + duration;
        self.signal_frames
            .iter()
            .filter(|frame| frame.timestamp > self.chain_end && frame.timestamp <= cutoff)
            .collect()
    }

    /// Get all signal data for the entire chain execution
    ///
    /// # Returns
    /// Vector of signal frames from chain start to chain end
    pub fn data_for_entire_chain(&self) -> Vec<&TimestampedSignalFrame> {
        self.signal_frames
            .iter()
            .filter(|frame| {
                frame.timestamp >= self.chain_start && frame.timestamp <= self.chain_end
            })
            .collect()
    }

    /// Get timing information for a specific action
    ///
    /// # Arguments
    /// * `action_index` - Index of the action in the chain
    ///
    /// # Returns
    /// Optional tuple of (start_time, end_time, duration)
    pub fn action_timing(&self, action_index: usize) -> Option<(Instant, Instant, Duration)> {
        self.action_timings
            .get(action_index)
            .map(|(start, end)| (*start, *end, end.duration_since(*start)))
    }

    /// Get summary statistics for the chain execution
    ///
    /// # Returns
    /// Tuple of (total_actions, successful_actions, total_frames, chain_duration)
    pub fn chain_summary(&self) -> (usize, usize, usize, Duration) {
        let total_actions = self.action_results.len();
        let successful_actions = self
            .action_results
            .iter()
            .filter(|result| matches!(result, crate::actions::ActionResult::Success))
            .count();
        let total_frames = self.signal_frames.len();

        (
            total_actions,
            successful_actions,
            total_frames,
            self.total_duration,
        )
    }
}

/// Result of an auto-approach operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoApproachResult {
    /// Auto-approach completed successfully
    Success,
    /// Auto-approach timed out before completion
    Timeout,
    /// Auto-approach failed (e.g., hardware error, abnormal termination)
    Failed(String),
    /// Auto-approach was already running when attempted to start
    AlreadyRunning,
    /// Auto-approach was cancelled/stopped externally
    Cancelled,
}

impl AutoApproachResult {
    /// Check if the result represents a successful operation
    pub fn is_success(&self) -> bool {
        matches!(self, AutoApproachResult::Success)
    }

    /// Check if the result represents a failure
    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            AutoApproachResult::Failed(_) | AutoApproachResult::Timeout
        )
    }

    /// Get error description if this is a failure
    pub fn error_message(&self) -> Option<&str> {
        match self {
            AutoApproachResult::Failed(msg) => Some(msg),
            AutoApproachResult::Timeout => Some("Auto-approach timed out"),
            AutoApproachResult::AlreadyRunning => Some("Auto-approach already running"),
            AutoApproachResult::Cancelled => Some("Auto-approach was cancelled"),
            AutoApproachResult::Success => None,
        }
    }
}

/// Status information for auto-approach operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoApproachStatus {
    /// Auto-approach is not running
    Idle,
    /// Auto-approach is currently running
    Running,
    /// Auto-approach status is unknown (e.g., communication error)
    Unknown,
}

