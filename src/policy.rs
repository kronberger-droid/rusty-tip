use crate::classifier::TipState;
use crate::types::MachineState;

// ==================== Core Policy Engine ====================

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
/// Takes interpreted tip states and makes policy decisions
pub trait PolicyEngine: Send + Sync {
    /// Make a policy decision based on interpreted tip state
    fn decide(&mut self, machine_state: &MachineState) -> PolicyDecision;
    fn get_name(&self) -> &str;

    // Future expansion for advanced policy engines:
    // fn bind_state_to_action(&mut self, _tip_state: &TipState, _action: ActionType) {}
    // fn learn_from_outcome(&mut self, _state: &TipState, _action: &ActionType, _outcome: f32) {}
}

/// Enhanced policy decisions with confidence and reasoning
/// Expandable for complex ML-driven decisions
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyDecision {
    // Simple decisions based on tip state classification
    Good, // Tip state is classified as good
    Bad,  // Tip state is classified as bad
    Stable, // Tip state is classified as stable

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

/// Simple rule-based policy that makes decisions based on tip state classifications
pub struct RuleBasedPolicy {
    name: String,
}

impl RuleBasedPolicy {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

impl PolicyEngine for RuleBasedPolicy {
    fn decide(&mut self, machine_state: &MachineState) -> PolicyDecision {
        // Simple policy: directly map classification to decision
        match machine_state.classification {
            TipState::Good => PolicyDecision::Good,
            TipState::Bad => PolicyDecision::Bad,
            TipState::Stable => PolicyDecision::Stable,
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_based_policy_mapping() {
        let mut policy = RuleBasedPolicy::new("Test Policy".to_string());

        use std::collections::VecDeque;
        
        // Create mock tip states with different classifications
        let good_state = MachineState {
            primary_signal: 1.0,
            all_signals: None,
            position: None,
            z_position: None,
            timestamp: 0.0,
            signal_history: VecDeque::new(),
            approach_count: 0,
            last_action: None,
            system_parameters: vec![],
            classification: TipState::Good,
        };

        let bad_state = MachineState {
            classification: TipState::Bad,
            ..good_state.clone()
        };

        let stable_state = MachineState {
            classification: TipState::Stable,
            ..good_state.clone()
        };

        // Test policy mappings
        assert_eq!(policy.decide(&good_state), PolicyDecision::Good);
        assert_eq!(policy.decide(&bad_state), PolicyDecision::Bad);
        assert_eq!(policy.decide(&stable_state), PolicyDecision::Stable);
        assert_eq!(policy.get_name(), "Test Policy");
    }
}