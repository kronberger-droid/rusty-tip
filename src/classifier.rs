use crate::types::MachineState;
use log::{debug, trace};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Trait for classifying raw signal data into interpreted tip states
pub trait StateClassifier: Send + Sync {
    /// Update machine state classification based on new signal reading
    fn classify(&mut self, machine_state: &mut MachineState);

    /// Pre-fill the classifier's buffer by collecting multiple samples
    /// This should be called before starting regular classification to avoid
    /// making decisions on insufficient data
    fn initialize_buffer(&mut self, machine_state: &MachineState, target_samples: usize);

    /// Clear the internal buffer to start fresh sampling
    fn clear_buffer(&mut self);

    /// Get the signal index this classifier is monitoring
    fn get_primary_signal_index(&self) -> i32;

    /// Get classifier name for debugging
    fn get_name(&self) -> &str;
}

/// Boundary-based state classifier using drop-front analysis on signal_history
pub struct BoundaryClassifier {
    name: String,
    signal_index: i32,
    min_bound: f32,
    max_bound: f32,
    drop_front: usize,
    buffer_size: usize,
    // State tracking
    consecutive_good_count: u32,
    stable_threshold: u32,
    last_classification: Option<TipState>,
    // Classifier maintains its own signal history
    own_signal_history: VecDeque<f32>,
}

/// Classification result for tip state
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum TipState {
    #[default]
    Bad, // Signal out of bounds
    Good,   // Signal within bounds
    Stable, // Signal has been good and stable for required period
}

/// Builder for BoundaryClassifier with sensible defaults
pub struct BoundaryClassifierBuilder {
    name: String,
    signal_index: Option<i32>,
    min_bound: Option<f32>,
    max_bound: Option<f32>,
    buffer_size: usize,
    drop_front: usize,
    stable_threshold: u32,
}

impl BoundaryClassifier {
    /// Create a new BoundaryClassifier builder with sensible defaults
    pub fn builder() -> BoundaryClassifierBuilder {
        BoundaryClassifierBuilder {
            name: "Boundary Classifier".to_string(),
            signal_index: None,
            min_bound: None,
            max_bound: None,
            buffer_size: 10,
            drop_front: 2,
            stable_threshold: 3,
        }
    }

    pub fn new(name: String, signal_index: i32, min_bound: f32, max_bound: f32) -> Self {
        Self {
            name,
            signal_index,
            min_bound,
            max_bound,
            drop_front: 2,
            buffer_size: 10, // Default buffer size
            consecutive_good_count: 0,
            stable_threshold: 3,
            last_classification: None,
            own_signal_history: VecDeque::with_capacity(10),
        }
    }

    pub fn with_buffer_config(mut self, buffer_size: usize, drop_front: usize) -> Self {
        // Now classifier manages its own buffer size
        self.buffer_size = buffer_size;
        self.drop_front = drop_front;
        self.own_signal_history = VecDeque::with_capacity(buffer_size);
        self
    }

    pub fn with_stability_config(mut self, stable_threshold: u32) -> Self {
        self.stable_threshold = stable_threshold;
        self
    }

    fn get_max_after_drop(&self) -> Option<f32> {
        if self.own_signal_history.len() <= self.drop_front {
            return None;
        }

        self.own_signal_history
            .iter()
            .skip(self.drop_front)
            .copied()
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    }

    fn is_within_bounds(&self, value: f32) -> bool {
        value >= self.min_bound && value <= self.max_bound
    }
    
    /// Extract primary signal value from all_signals using signal_indices mapping
    fn extract_primary_signal_value(&self, machine_state: &MachineState) -> Option<f32> {
        if let (Some(all_signals), Some(signal_indices)) = 
            (&machine_state.all_signals, &machine_state.signal_indices) {
            
            // Find position of our signal_index in the signal_indices array
            if let Some(position) = signal_indices.iter().position(|&idx| idx == self.signal_index) {
                if position < all_signals.len() {
                    return Some(all_signals[position]);
                }
            }
        }
        None
    }
}

impl BoundaryClassifierBuilder {
    /// Set the name for the classifier
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the signal index to monitor (required)
    pub fn signal_index(mut self, index: i32) -> Self {
        self.signal_index = Some(index);
        self
    }

    /// Set the boundary bounds (required)
    pub fn bounds(mut self, min_bound: f32, max_bound: f32) -> Self {
        self.min_bound = Some(min_bound);
        self.max_bound = Some(max_bound);
        self
    }

    /// Set buffer configuration (optional, has defaults)
    pub fn buffer_config(mut self, buffer_size: usize, drop_front: usize) -> Self {
        self.buffer_size = buffer_size;
        self.drop_front = drop_front;
        self
    }

    /// Set stability threshold (optional, has default)
    pub fn stability_threshold(mut self, threshold: u32) -> Self {
        self.stable_threshold = threshold;
        self
    }

    /// Build the BoundaryClassifier with validation
    pub fn build(self) -> Result<BoundaryClassifier, String> {
        let signal_index = self.signal_index
            .ok_or("signal_index is required - use .signal_index()")?;
        let min_bound = self.min_bound
            .ok_or("min_bound is required - use .bounds(min, max)")?;
        let max_bound = self.max_bound
            .ok_or("max_bound is required - use .bounds(min, max)")?;

        if min_bound >= max_bound {
            return Err(format!("min_bound ({min_bound}) must be less than max_bound ({max_bound})"));
        }

        // Buffer size validation removed - it's now managed by SignalMonitor
        // Just validate that drop_front is reasonable
        if self.drop_front > 50 {
            return Err(format!("drop_front ({}) seems unreasonably large", self.drop_front));
        }

        if self.stable_threshold == 0 {
            return Err("stable_threshold must be greater than 0".to_string());
        }

        Ok(BoundaryClassifier {
            name: self.name,
            signal_index,
            min_bound,
            max_bound,
            drop_front: self.drop_front,
            buffer_size: self.buffer_size,
            consecutive_good_count: 0,
            stable_threshold: self.stable_threshold,
            last_classification: None,
            own_signal_history: VecDeque::with_capacity(self.buffer_size),
        })
    }
}

impl StateClassifier for BoundaryClassifier {
    fn classify(&mut self, machine_state: &mut MachineState) {
        // Use the fresh samples provided by the controller in signal_history
        self.own_signal_history.clear();
        
        if !machine_state.signal_history.is_empty() {
            // Controller has pre-filled signal_history with fresh samples
            self.own_signal_history.extend(machine_state.signal_history.iter().copied());
            debug!("Using {} pre-collected fresh samples from controller", self.own_signal_history.len());
        } else {
            // Fallback: extract current primary signal value and fill buffer
            if let Some(primary_value) = self.extract_primary_signal_value(machine_state) {
                trace!("Primary signal {} extracted from all_signals: {}", self.signal_index, primary_value);
                
                // Fill buffer with current signal value as fallback
                debug!("Fallback: filling classifier buffer with current value ({primary_value})");
                for _ in 0..self.buffer_size {
                    self.own_signal_history.push_back(primary_value);
                }
            } else {
                debug!("Could not extract primary signal, returning Bad");
                machine_state.classification = TipState::Bad;
                machine_state.timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64();
                return;
            }
        }
        
        trace!(
            "Analyzing classifier's own signal_history with {} samples, drop_front: {}",
            self.own_signal_history.len(),
            self.drop_front
        );

        let classification = if let Some(max_value) = self.get_max_after_drop() {
            // Store the decision value in history
            machine_state.decision_value_history.push_back(max_value);
            // Keep decision history buffer at reasonable size (match signal_history size)
            if machine_state.decision_value_history.len() > self.buffer_size {
                machine_state.decision_value_history.pop_front();
            }
            
            if self.is_within_bounds(max_value) {
                TipState::Good
            } else {
                TipState::Bad
            }
        } else {
            // Not enough samples yet for reliable classification
            TipState::Bad
        };

        // Update stability tracking
        let final_classification = match classification {
            TipState::Good => {
                // Check if we previously had a good classification
                if matches!(
                    self.last_classification,
                    Some(TipState::Good) | Some(TipState::Stable)
                ) {
                    self.consecutive_good_count += 1;
                } else {
                    self.consecutive_good_count = 1;
                }

                // Check if we've reached stability threshold
                if self.consecutive_good_count >= self.stable_threshold {
                    debug!(
                        "STABILITY ACHIEVED: {} consecutive good readings",
                        self.consecutive_good_count
                    );
                    self.last_classification = Some(TipState::Stable);
                    TipState::Stable
                } else {
                    trace!(
                        "Good count: {}/{}",
                        self.consecutive_good_count,
                        self.stable_threshold
                    );
                    self.last_classification = Some(TipState::Good);
                    TipState::Good
                }
            }
            TipState::Bad => {
                // Reset consecutive count on bad signal
                self.consecutive_good_count = 0;
                self.last_classification = Some(TipState::Bad);
                TipState::Bad
            }
            TipState::Stable => unreachable!(), // We don't create Stable directly above
        };

        // Update machine state with new classification
        machine_state.timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        machine_state.classification = final_classification;
    }

    fn initialize_buffer(&mut self, machine_state: &MachineState, target_samples: usize) {
        // Pre-fill buffer by duplicating the current primary signal value
        // This provides immediate classification capability while maintaining
        // the same signal characteristics
        if let Some(primary_value) = self.extract_primary_signal_value(machine_state) {
            let needed_samples = target_samples.saturating_sub(self.own_signal_history.len());
            
            for _ in 0..needed_samples {
                self.own_signal_history.push_back(primary_value);
                if self.own_signal_history.len() > self.buffer_size {
                    self.own_signal_history.pop_front();
                }
            }
            
            debug!(
                "Initialized classifier buffer with {} samples (value: {})",
                self.own_signal_history.len(),
                primary_value
            );
        }
    }

    fn clear_buffer(&mut self) {
        // Clear the classifier's own signal history buffer
        self.own_signal_history.clear();
        self.consecutive_good_count = 0;
        self.last_classification = None;
    }

    fn get_primary_signal_index(&self) -> i32 {
        self.signal_index
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boundary_classifier_creation() {
        let classifier = BoundaryClassifier::new("Test Classifier".to_string(), 24, 0.0, 2.0);

        assert_eq!(classifier.get_name(), "Test Classifier");
        assert_eq!(classifier.get_primary_signal_index(), 24);
    }

    #[test]
    fn test_boundary_classifier_good_signal() {
        let mut classifier = BoundaryClassifier::new(
            "Test".to_string(),
            24,
            0.0, // min
            2.0, // max
        );

        // Simulate multiple signal readings to build classifier's internal history
        // The classifier needs enough samples for drop-front analysis
        for i in 0..5 {
            let mut machine_state = crate::types::MachineState {
                all_signals: Some(vec![1.0 + i as f32 * 0.1, 1.5, 1.2]), // Simulated signals, signal 24 varies slightly
                signal_indices: Some(vec![24, 25, 26]), // Signal index 24 is at position 0
                ..Default::default()
            };
            
            classifier.classify(&mut machine_state);
            
            // On the last iteration, we should have reached stable state (3 consecutive good = stable)
            if i == 4 {
                assert_eq!(machine_state.classification, TipState::Stable);
            }
        }
    }

    #[test]
    fn test_boundary_classifier_bad_signal() {
        let mut classifier = BoundaryClassifier::new(
            "Test".to_string(),
            24,
            0.0, // min
            2.0, // max
        )
        .with_buffer_config(5, 2); // Need enough buffer

        use std::collections::VecDeque;

        // Create machine state with signal history containing values where max after drop is bad
        let mut signal_history = VecDeque::new();
        signal_history.push_back(1.0); // First value (dropped)
        signal_history.push_back(1.5); // Second value (dropped)
        signal_history.push_back(3.0); // Above max - this should trigger Bad

        let mut machine_state = crate::types::MachineState {
            signal_history,
            ..Default::default()
        };
        classifier.classify(&mut machine_state);
        assert_eq!(machine_state.classification, TipState::Bad);
    }

    #[test]
    fn test_boundary_classifier_stability_tracking() {
        let mut classifier =
            BoundaryClassifier::new("Test".to_string(), 24, 0.0, 2.0).with_stability_config(3); // 3 consecutive good for stable

        // First good signal
        let mut signal_history1 = VecDeque::new();
        signal_history1.push_back(1.0); // Good signal within bounds [0.0, 2.0]
        signal_history1.push_back(1.2);
        signal_history1.push_back(0.8);
        
        let mut machine_state1 = crate::types::MachineState {
            signal_history: signal_history1,
            ..Default::default()
        };
        classifier.classify(&mut machine_state1);
        assert_eq!(machine_state1.classification, TipState::Good);

        // Second good signal
        let mut signal_history2 = VecDeque::new();
        signal_history2.push_back(1.1);
        signal_history2.push_back(1.3);
        signal_history2.push_back(0.9);
        
        let mut machine_state2 = crate::types::MachineState {
            signal_history: signal_history2,
            ..Default::default()
        };
        classifier.classify(&mut machine_state2);
        assert_eq!(machine_state2.classification, TipState::Good);

        // Third good signal should trigger stable
        let mut signal_history3 = VecDeque::new();
        signal_history3.push_back(1.2);
        signal_history3.push_back(1.4);
        signal_history3.push_back(1.0);
        
        let mut machine_state3 = crate::types::MachineState {
            signal_history: signal_history3,
            ..Default::default()
        };
        classifier.classify(&mut machine_state3);
        assert_eq!(machine_state3.classification, TipState::Stable);
    }

    #[test]
    fn test_boundary_classifier_buffer_and_drop_front() {
        let mut classifier =
            BoundaryClassifier::new("Test".to_string(), 24, 0.0, 2.0).with_buffer_config(5, 2); // Buffer 5, drop first 2

        use std::collections::VecDeque;

        // Add values: 1.0, 1.5, 3.0 (3.0 is above max)
        // With drop_front=2, only 3.0 is considered -> Bad
        let mut signal_history = VecDeque::new();
        signal_history.push_back(1.0); // First value (dropped)
        signal_history.push_back(1.5); // Second value (dropped)
        signal_history.push_back(3.0); // Third value (max after drop) - should be Bad

        let mut machine_state = crate::types::MachineState {
            signal_history,
            ..Default::default()
        };
        classifier.classify(&mut machine_state);

        assert_eq!(machine_state.classification, TipState::Bad);
    }

    #[test]
    fn test_tip_state_enrichment() {
        let mut classifier = BoundaryClassifier::new("Test".to_string(), 42, -1.0, 1.0);

        let all_signals = vec![0.1, 0.2, 0.3, 0.4];
        let mut signal_history = VecDeque::new();
        signal_history.push_back(0.5); // Good signal within bounds [-1.0, 1.0]
        signal_history.push_back(0.3);
        signal_history.push_back(0.7);
        
        let mut machine_state = crate::types::MachineState {
            all_signals: Some(all_signals.clone()),
            signal_history,
            ..Default::default()
        };
        classifier.classify(&mut machine_state);
        assert_eq!(machine_state.all_signals, Some(all_signals));
        assert!(machine_state.timestamp > 0.0);
        assert_eq!(machine_state.classification, TipState::Good);
    }
}
