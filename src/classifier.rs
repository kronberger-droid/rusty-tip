use crate::types::MachineState;
use std::collections::VecDeque;

/// Trait for classifying raw signal data into interpreted tip states
pub trait StateClassifier: Send + Sync {
    /// Update machine state classification based on new signal reading
    fn classify(&mut self, machine_state: &mut MachineState);

    /// Clear the internal buffer to start fresh sampling
    fn clear_buffer(&mut self);

    /// Get the signal index this classifier is monitoring
    fn get_primary_signal_index(&self) -> i32;

    /// Get classifier name for debugging
    fn get_name(&self) -> &str;
}

/// Boundary-based state classifier using buffering and drop-front analysis
pub struct BoundaryClassifier {
    name: String,
    signal_index: i32,
    min_bound: f32,
    max_bound: f32,
    buffer: VecDeque<f32>,
    buffer_size: usize,
    drop_front: usize,
    // State tracking
    consecutive_good_count: u32,
    stable_threshold: u32,
    last_classification: Option<TipState>,
}

/// Classification result for tip state
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TipState {
    #[default]
    Bad,    // Signal out of bounds
    Good,   // Signal within bounds
    Stable, // Signal has been good and stable for required period
}

impl BoundaryClassifier {
    pub fn new(name: String, signal_index: i32, min_bound: f32, max_bound: f32) -> Self {
        Self {
            name,
            signal_index,
            min_bound,
            max_bound,
            buffer: VecDeque::new(),
            buffer_size: 10,
            drop_front: 2,
            consecutive_good_count: 0,
            stable_threshold: 3,
            last_classification: None,
        }
    }

    pub fn with_buffer_config(mut self, buffer_size: usize, drop_front: usize) -> Self {
        self.buffer_size = buffer_size;
        self.drop_front = drop_front;
        self
    }

    pub fn with_stability_config(mut self, stable_threshold: u32) -> Self {
        self.stable_threshold = stable_threshold;
        self
    }

    fn get_max_after_drop(&self) -> Option<f32> {
        if self.buffer.len() <= self.drop_front {
            return None;
        }

        self.buffer
            .iter()
            .skip(self.drop_front)
            .copied()
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    }

    fn is_within_bounds(&self, value: f32) -> bool {
        value >= self.min_bound && value <= self.max_bound
    }

}

impl StateClassifier for BoundaryClassifier {
    fn classify(&mut self, machine_state: &mut MachineState) {
        // If machine state has fresh samples in signal_history, use those to fill buffer
        if !machine_state.signal_history.is_empty() {
            self.buffer.clear();
            for &sample in machine_state.signal_history.iter().take(self.buffer_size) {
                self.buffer.push_back(sample);
            }
            println!("  Buffer filled with {} fresh samples: {:?}", 
                    self.buffer.len(), 
                    self.buffer.iter().collect::<Vec<_>>());
        }
        
        let classification = if let Some(max_value) = self.get_max_after_drop() {
            if self.is_within_bounds(max_value) {
                TipState::Good
            } else {
                TipState::Bad
            }
        } else {
            TipState::Good
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
                    println!("  STABILITY ACHIEVED: {} consecutive good readings", self.consecutive_good_count);
                    self.last_classification = Some(TipState::Stable);
                    TipState::Stable
                } else {
                    println!("  Good count: {}/{}", self.consecutive_good_count, self.stable_threshold);
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

    fn clear_buffer(&mut self) {
        self.buffer.clear();
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

        let mut machine_state = crate::types::MachineState {
            primary_signal: 1.0,
            ..Default::default()
        };
        classifier.classify(&mut machine_state);
        assert_eq!(machine_state.classification, TipState::Good);
        assert_eq!(machine_state.primary_signal, 1.0);
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
            primary_signal: 3.0,
            signal_history,
            ..Default::default()
        };
        classifier.classify(&mut machine_state);
        assert_eq!(machine_state.classification, TipState::Bad);
        assert_eq!(machine_state.primary_signal, 3.0);
    }

    #[test]
    fn test_boundary_classifier_stability_tracking() {
        let mut classifier =
            BoundaryClassifier::new("Test".to_string(), 24, 0.0, 2.0).with_stability_config(3); // 3 consecutive good for stable

        // First good signal
        let mut machine_state1 = crate::types::MachineState {
            primary_signal: 1.0,
            ..Default::default()
        };
        classifier.classify(&mut machine_state1);
        assert_eq!(machine_state1.classification, TipState::Good);

        // Second good signal
        let mut machine_state2 = crate::types::MachineState {
            primary_signal: 1.0,
            ..Default::default()
        };
        classifier.classify(&mut machine_state2);
        assert_eq!(machine_state2.classification, TipState::Good);

        // Third good signal should trigger stable
        let mut machine_state3 = crate::types::MachineState {
            primary_signal: 1.0,
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
            primary_signal: 3.0,
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
        let mut machine_state = crate::types::MachineState {
            primary_signal: 0.5,
            all_signals: Some(all_signals.clone()),
            ..Default::default()
        };
        classifier.classify(&mut machine_state);

        assert_eq!(machine_state.primary_signal, 0.5);
        assert_eq!(machine_state.all_signals, Some(all_signals));
        assert!(machine_state.timestamp > 0.0);
        assert_eq!(machine_state.classification, TipState::Good);
    }
}
