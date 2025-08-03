use std::collections::VecDeque;
use crate::types::TipState;

/// Trait for classifying raw signal data into interpreted tip states
pub trait StateClassifier: Send + Sync {
    /// Convert raw signal readings into a comprehensive tip state
    fn classify(&mut self, primary_signal: f32, all_signals: Option<&[f32]>) -> TipState;

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
    last_classification: Option<TipClassification>,
}

/// Classification result for tip state
#[derive(Debug, Clone, PartialEq)]
pub enum TipClassification {
    Good,   // Signal within bounds
    Bad,    // Signal out of bounds
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

    fn classify_signal(&mut self, signal_value: f32) -> TipClassification {
        // Add value to buffer
        if self.buffer.len() == self.buffer_size {
            self.buffer.pop_front();
        }
        self.buffer.push_back(signal_value);

        // Check bounds using max value after dropping front values
        let classification = if let Some(max_value) = self.get_max_after_drop() {
            if self.is_within_bounds(max_value) {
                TipClassification::Good
            } else {
                TipClassification::Bad
            }
        } else {
            // Not enough data yet - assume good
            TipClassification::Good
        };

        // Update stability tracking
        match classification {
            TipClassification::Good => {
                // Check if we previously had a good classification
                if matches!(
                    self.last_classification,
                    Some(TipClassification::Good) | Some(TipClassification::Stable)
                ) {
                    self.consecutive_good_count += 1;
                } else {
                    // First good after bad, reset counter
                    self.consecutive_good_count = 1;
                }

                // Check if we've reached stability threshold
                if self.consecutive_good_count >= self.stable_threshold {
                    self.last_classification = Some(TipClassification::Stable);
                    TipClassification::Stable
                } else {
                    self.last_classification = Some(TipClassification::Good);
                    TipClassification::Good
                }
            }
            TipClassification::Bad => {
                // Reset stability tracking on bad classification
                self.consecutive_good_count = 0;
                self.last_classification = Some(TipClassification::Bad);
                TipClassification::Bad
            }
            TipClassification::Stable => unreachable!(), // We don't create Stable directly above
        }
    }
}

impl StateClassifier for BoundaryClassifier {
    fn classify(&mut self, primary_signal: f32, all_signals: Option<&[f32]>) -> TipState {
        let classification = self.classify_signal(primary_signal);

        // Create comprehensive tip state
        let mut signal_history = VecDeque::with_capacity(50);
        signal_history.extend(self.buffer.iter().copied());

        TipState {
            primary_signal,
            all_signals: all_signals.map(|s| s.to_vec()),
            signal_names: None, // Will be populated by controller if available
            position: None,     // Will be populated by controller if available
            z_position: None,   // Will be populated by controller if available
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64(),
            signal_history,
            approach_count: 0, // Will be populated by controller
            last_action: None, // Will be populated by controller
            system_parameters: vec![],
            classification,
        }
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

        let tip_state = classifier.classify(1.0, None);
        assert_eq!(tip_state.classification, TipClassification::Good);
        assert_eq!(tip_state.primary_signal, 1.0);
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

        // Add values to fill buffer past drop_front
        classifier.classify(1.0, None); // First value (dropped)
        classifier.classify(1.5, None); // Second value (dropped)
        let tip_state = classifier.classify(3.0, None); // Above max
        assert_eq!(tip_state.classification, TipClassification::Bad);
        assert_eq!(tip_state.primary_signal, 3.0);
    }

    #[test]
    fn test_boundary_classifier_stability_tracking() {
        let mut classifier =
            BoundaryClassifier::new("Test".to_string(), 24, 0.0, 2.0).with_stability_config(3); // 3 consecutive good for stable

        // First good signal
        let tip_state1 = classifier.classify(1.0, None);
        assert_eq!(tip_state1.classification, TipClassification::Good);

        // Second good signal
        let tip_state2 = classifier.classify(1.0, None);
        assert_eq!(tip_state2.classification, TipClassification::Good);

        // Third good signal should trigger stable
        let tip_state3 = classifier.classify(1.0, None);
        assert_eq!(tip_state3.classification, TipClassification::Stable);
    }

    #[test]
    fn test_boundary_classifier_buffer_and_drop_front() {
        let mut classifier =
            BoundaryClassifier::new("Test".to_string(), 24, 0.0, 2.0).with_buffer_config(5, 2); // Buffer 5, drop first 2

        // Add values: 1.0, 1.5, 3.0 (3.0 is above max)
        // With drop_front=2, only 3.0 is considered -> Bad
        classifier.classify(1.0, None); // First value (dropped)
        classifier.classify(1.5, None); // Second value (dropped)
        let tip_state = classifier.classify(3.0, None); // Third value (max after drop)

        assert_eq!(tip_state.classification, TipClassification::Bad);
    }

    #[test]
    fn test_tip_state_enrichment() {
        let mut classifier = BoundaryClassifier::new("Test".to_string(), 42, -1.0, 1.0);

        let all_signals = vec![0.1, 0.2, 0.3, 0.4];
        let tip_state = classifier.classify(0.5, Some(&all_signals));

        assert_eq!(tip_state.primary_signal, 0.5);
        assert_eq!(tip_state.all_signals, Some(all_signals));
        assert!(tip_state.timestamp > 0.0);
        assert!(!tip_state.signal_history.is_empty());
        assert_eq!(tip_state.classification, TipClassification::Good);
    }
}