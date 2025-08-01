use std::collections::VecDeque;
use std::time::{Duration, Instant};

// ==================== Core Policy Engine ====================

/// Comprehensive tip state for advanced policy engines
/// Expandable for transformer/ML models that need rich context
#[derive(Debug, Clone)]
pub struct TipState {
    // Current signal readings
    pub primary_signal: f32,                    // The monitored signal (e.g., bias)
    pub all_signals: Option<Vec<f32>>,          // All available signals for context
    pub signal_names: Option<Vec<String>>,      // Signal identifications
    
    // Spatial context
    pub position: Option<(f64, f64)>,           // Current XY position
    pub z_position: Option<f64>,                // Z height
    
    // Temporal context
    pub timestamp: f64,                         // When this state was captured
    pub signal_history: VecDeque<f32>,          // Historical signal values
    
    // System state
    pub approach_count: u32,                    // Number of approaches performed
    pub last_action: Option<String>,            // Last action executed
    pub system_parameters: Vec<f32>,            // Configurable system params
    
    // For future ML/transformer expansion:
    // pub embedding: Option<Vec<f32>>,         // Learned state representation
    // pub attention_weights: Option<Vec<f32>>, // Transformer attention scores
    // pub confidence: f32,                     // Model confidence in decision
}

/// Action types that can be bound to tip states
/// Expandable for complex action sequences
#[derive(Debug, Clone)]
pub enum ActionType {
    // Current actions
    Approach,
    Withdraw,
    Move { dx: f64, dy: f64 },
    Pulse { parameters: Vec<f32> },
    
    // Future expansion:
    // ComplexManeuver { sequence: Vec<ActionType> },
    // AdaptiveApproach { learning_rate: f32 },
    // MultiSignalOptimization { targets: Vec<f32> },
}

/// Core trait for any decision-making system
/// Backward compatible with current implementation
pub trait PolicyEngine: Send + Sync {
    // Current simple interface (maintains backward compatibility)
    fn decide(&mut self, signal_value: f32) -> PolicyDecision;
    fn get_name(&self) -> &str;
    
    // Future expansion for advanced policy engines:
    // Default implementations maintain backward compatibility
    // fn decide_with_context(&mut self, tip_state: &TipState) -> PolicyDecision {
    //     self.decide(tip_state.primary_signal)
    // }
    // fn bind_state_to_action(&mut self, _tip_state: &TipState, _action: ActionType) {}
    // fn learn_from_outcome(&mut self, _state: &TipState, _action: &ActionType, _outcome: f32) {}
}

/// Enhanced policy decisions with confidence and reasoning
/// Expandable for complex ML-driven decisions
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyDecision {
    // Simple decisions (current implementation)
    Good,    // Signal within bounds
    Bad,     // Signal out of bounds
    Stable,  // Signal has been good and stable for required period
    
    // Future expansion for transformer/ML policies:
    // ComplexAction { 
    //     action_type: ActionType,
    //     confidence: f32,
    //     reasoning: String,
    //     parameters: Vec<f32>,
    // },
    // MultiStep {
    //     actions: Vec<ActionType>,
    //     expected_outcomes: Vec<f32>,
    // },
}

// ==================== Advanced Policy Engine Traits ====================

/// Trait for policies that can learn from experience
/// For transformer/ML-based policies
pub trait LearningPolicyEngine: PolicyEngine {
    // fn update_model(&mut self, training_data: &[(TipState, ActionType, f32)]);
    // fn save_model(&self, path: &str) -> Result<(), Box<dyn std::error::Error>>;
    // fn load_model(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>>;
}

/// Trait for policies that can explain their decisions
/// Useful for interpretable AI in scientific applications
pub trait ExplainablePolicyEngine: PolicyEngine {
    // fn explain_decision(&self, tip_state: &TipState) -> String;
    // fn get_attention_weights(&self) -> Option<Vec<f32>>;
    // fn get_confidence(&self) -> f32;
}

// ==================== Rule-Based Policy ====================

pub struct RuleBasedPolicy {
    name: String,
    signal_index: i32,
    min_bound: f32,
    max_bound: f32,
    buffer: VecDeque<f32>,
    buffer_size: usize,
    drop_front: usize,
    // Stability tracking
    consecutive_good_count: u32,
    stable_threshold: u32,  // Number of consecutive good decisions needed for stable
    last_decision: Option<PolicyDecision>,
}

impl RuleBasedPolicy {
    pub fn new(name: String, signal_index: i32, min_bound: f32, max_bound: f32) -> Self {
        Self {
            name,
            signal_index,
            min_bound,
            max_bound,
            buffer: VecDeque::new(),
            buffer_size: 10,    // Keep last 10 values
            drop_front: 2,      // Drop first 2 values
            consecutive_good_count: 0,
            stable_threshold: 3, // Default: 3 consecutive good for stable
            last_decision: None,
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

    pub fn signal_index(&self) -> i32 {
        self.signal_index
    }

    /// Get max value from buffer after dropping front values
    fn get_max_after_drop(&self) -> Option<f32> {
        if self.buffer.len() <= self.drop_front {
            return None;
        }
        
        self.buffer.iter()
            .skip(self.drop_front)
            .copied()
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Check if value is within bounds
    fn is_within_bounds(&self, value: f32) -> bool {
        value >= self.min_bound && value <= self.max_bound
    }
}

impl PolicyEngine for RuleBasedPolicy {
    fn decide(&mut self, signal_value: f32) -> PolicyDecision {
        // Add value to buffer
        if self.buffer.len() == self.buffer_size {
            self.buffer.pop_front();
        }
        self.buffer.push_back(signal_value);

        // Check bounds using max value after dropping front values
        let decision = if let Some(max_value) = self.get_max_after_drop() {
            if self.is_within_bounds(max_value) {
                PolicyDecision::Good
            } else {
                PolicyDecision::Bad
            }
        } else {
            // Not enough data yet - assume good
            PolicyDecision::Good
        };

        // Update stability tracking
        match decision {
            PolicyDecision::Good => {
                // Check if we previously had a good decision
                if matches!(self.last_decision, Some(PolicyDecision::Good) | Some(PolicyDecision::Stable)) {
                    self.consecutive_good_count += 1;
                } else {
                    // First good after bad, reset counter
                    self.consecutive_good_count = 1;
                }

                // Check if we've reached stability threshold
                if self.consecutive_good_count >= self.stable_threshold {
                    self.last_decision = Some(PolicyDecision::Stable);
                    PolicyDecision::Stable
                } else {
                    self.last_decision = Some(PolicyDecision::Good);
                    PolicyDecision::Good
                }
            }
            PolicyDecision::Bad => {
                // Reset stability tracking on bad decision
                self.consecutive_good_count = 0;
                self.last_decision = Some(PolicyDecision::Bad);
                PolicyDecision::Bad
            }
            PolicyDecision::Stable => unreachable!(), // We don't create Stable directly above
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}