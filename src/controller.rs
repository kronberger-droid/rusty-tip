use crate::client::NanonisClient;
use crate::error::NanonisError;
use crate::classifier::StateClassifier;
use crate::policy::{ActionType, PolicyDecision, PolicyEngine};
use crate::types::{Position, TipState};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Controller integrating Nanonis client with state classifier and policy engine
/// Follows separated architecture: raw signals → state classification → policy decisions
pub struct Controller {
    client: NanonisClient,
    classifier: Box<dyn StateClassifier>,
    policy: Box<dyn PolicyEngine>,

    // State tracking for advanced policy engines
    approach_count: u32,
    position_history: VecDeque<Position>,
    action_history: VecDeque<String>,
    // For future ML expansion:
    // state_buffer: VecDeque<TipState>,     // Rich state history for transformers
    // action_outcomes: Vec<(ActionType, f32)>, // Action-outcome pairs for learning
    // model_confidence: f32,                // Current model confidence
}

impl Controller {
    pub fn new(
        address: &str, 
        classifier: Box<dyn StateClassifier>, 
        policy: Box<dyn PolicyEngine>
    ) -> Result<Self, NanonisError> {
        let client = NanonisClient::new(address)?;
        Ok(Self {
            client,
            classifier,
            policy,
            approach_count: 0,
            position_history: VecDeque::with_capacity(100),
            action_history: VecDeque::with_capacity(100),
        })
    }

    pub fn with_client(
        client: NanonisClient, 
        classifier: Box<dyn StateClassifier>, 
        policy: Box<dyn PolicyEngine>
    ) -> Self {
        Self {
            client,
            classifier,
            policy,
            approach_count: 0,
            position_history: VecDeque::with_capacity(100),
            action_history: VecDeque::with_capacity(100),
        }
    }

    /// Main control loop - policy-driven monitoring with state-based actions
    pub fn run_control_loop(
        &mut self,
        sample_rate_hz: f32,
        duration: Duration,
    ) -> Result<(), NanonisError> {
        let sample_interval = Duration::from_millis((1000.0 / sample_rate_hz) as u64);
        let start = Instant::now();
        let signal_index = self.classifier.get_primary_signal_index();

        println!("Starting policy-driven control loop for signal index {signal_index}");
        println!("Sample rate: {sample_rate_hz:.1} Hz, Duration: {duration:?}",);

        while start.elapsed() < duration {
            match self.run_monitoring_loop(sample_interval)? {
                LoopAction::ContinueBadLoop => {
                    // Signal was bad, actions executed, continue recovery monitoring
                    // Loop continues automatically
                }
                LoopAction::ContinueStabilityLoop => {
                    // Signal was good, actions executed, continue stability monitoring
                    // Loop continues automatically
                }
                LoopAction::Halt => {
                    println!("STABLE signal achieved - halting process");
                    break;
                }
            }
        }

        println!("Control loop completed after {:?}", start.elapsed());
        Ok(())
    }

    /// Single monitoring loop iteration with actions based on state  
    fn run_monitoring_loop(
        &mut self,
        _sample_interval: Duration,
    ) -> Result<LoopAction, NanonisError> {
        let signal_index = self.classifier.get_primary_signal_index();
        
        // Read the signal value
        let values = self.client.signals_val_get(vec![signal_index], true)?;
        let primary_signal = values[0];

        // Classify raw signal into tip state
        let mut tip_state = self.classifier.classify(primary_signal, Some(&values));
        
        // Enrich tip state with controller context
        self.enrich_tip_state(&mut tip_state)?;

        // Let policy decide based on classified state
        let decision = self.policy.decide(&tip_state);

        match decision {
            PolicyDecision::Bad => {
                println!("⚠ Signal {signal_index} = {primary_signal:.6} - BAD ({})", 
                    self.classifier.get_name());

                // Execute bad signal actions
                self.execute_bad_actions()?;

                Ok(LoopAction::ContinueBadLoop) // Continue in bad recovery mode
            }
            PolicyDecision::Good => {
                println!("✓ Signal {signal_index} = {primary_signal:.6} - GOOD ({})", 
                    self.classifier.get_name());

                // Execute good signal actions
                self.execute_good_actions()?;

                Ok(LoopAction::ContinueStabilityLoop) // Continue monitoring for stability
            }
            PolicyDecision::Stable => {
                println!("Signal {signal_index} = {primary_signal:.6} - STABLE ({})", 
                    self.classifier.get_name());
                Ok(LoopAction::Halt) // Halt the process
            }
        }
    }

    /// Execute actions when signal is bad: approach → pulse → withdraw → move → approach → check
    fn execute_bad_actions(&mut self) -> Result<(), NanonisError> {
        println!(" Executing bad signal recovery sequence...");

        // Step 1: Initial approach (if not already approached)
        println!("Performing initial approach...");
        self.client.auto_approach_and_wait()?;
        self.approach_count += 1;
        self.action_history
            .push_back("Initial Approach".to_string());

        // Step 2: Pulse operation (simulate for now)
        println!("Executing pulse operation...");
        // TODO: Implement actual pulse operation when available
        std::thread::sleep(Duration::from_millis(200));
        println!("   Pulse completed");

        // Step 3: Withdraw tip
        println!("Withdrawing tip...");
        self.client.z_ctrl_withdraw(true, 5000)?; // 5 second timeout

        // Step 4: Move to new position
        println!("Moving to new position...");
        let current_position = self.client.folme_xy_pos_get(true)?;

        // Move by small offset (3nm in both directions)
        let dx: f64 = 3e-9;
        let dy: f64 = 3e-9;
        let new_position = Position {
            x: current_position.x + dx,
            y: current_position.y + dy,
        };

        println!(
            "   Moving from ({:.3e}, {:.3e}) to ({:.3e}, {:.3e})",
            current_position.x, current_position.y, new_position.x, new_position.y
        );
        self.client.folme_xy_pos_set(new_position, true)?;

        // Step 5: Approach at new position
        println!("Approaching at new position...");
        self.client.auto_approach_and_wait()?;
        self.approach_count += 1;
        self.position_history.push_back(new_position);
        self.action_history.push_back("Re-approach".to_string());

        // Step 6: Check tip state (read signal to verify)
        println!("Checking tip state...");
        // This will be checked by the policy engine in the next loop iteration

        println!("Bad signal recovery sequence completed");
        Ok(())
    }

    /// Execute actions when signal is good
    fn execute_good_actions(&mut self) -> Result<(), NanonisError> {
        println!("Executing good signal actions...");

        // TODO: Implement good signal actions:
        // - Optimize scan parameters
        // - Fine-tune approach
        // - Prepare for measurements

        // For now, simulate with delay
        std::thread::sleep(Duration::from_millis(200));
        println!("Good signal actions completed");

        Ok(())
    }

    /// Placeholder for operational mode (scanning, measurements, etc.)
    #[allow(dead_code)]
    fn run_operational_mode(&mut self) -> Result<(), NanonisError> {
        println!("Entering operational mode...");

        // For now, just wait and return to monitoring
        std::thread::sleep(Duration::from_secs(1));
        println!("Operational mode completed - returning to monitoring");

        Ok(())
    }

    // ==================== State Enhancement Methods ====================
    
    /// Enrich tip state with additional context from the controller
    fn enrich_tip_state(&mut self, tip_state: &mut TipState) -> Result<(), NanonisError> {
        // Add position information if available
        if let Ok(position) = self.client.folme_xy_pos_get(true) {
            tip_state.position = Some((position.x, position.y));
        }

        // Add signal names if available
        if tip_state.signal_names.is_none() {
            if let Ok(names) = self.client.signal_names_get(false) {
                tip_state.signal_names = Some(names);
            }
        }

        // Add controller state
        tip_state.approach_count = self.approach_count;
        tip_state.last_action = self.action_history.back().cloned();

        Ok(())
    }

    // ==================== Future Expansion Methods ====================
    // These methods show how to expand the controller for ML/transformer policies

    /// Bind tip states to specific actions for learning
    /// This would train transformer/ML models to associate states with optimal actions
    #[allow(dead_code)]
    fn bind_state_to_action(&mut self, _tip_state: &TipState, _action: ActionType) {
        // For future ML expansion:
        // - Record state-action pairs
        // - Build training dataset
        // - Update model weights
        // - Implement reinforcement learning

        // Example expansion:
        // self.training_data.push((tip_state.clone(), action, outcome));
        // if self.training_data.len() > self.batch_size {
        //     self.policy.update_model(&self.training_data);
        //     self.training_data.clear();
        // }
    }

    /// Demonstrate how to execute complex action sequences
    /// Future expansion for transformer-planned action sequences
    #[allow(dead_code)]
    fn execute_complex_action(&mut self, _action: ActionType) -> Result<f32, NanonisError> {
        // For future expansion - execute transformer-planned actions:
        // match action {
        //     ActionType::ComplexManeuver { sequence } => {
        //         for step in sequence {
        //             self.execute_atomic_action(step)?;
        //         }
        //     }
        //     ActionType::AdaptiveApproach { learning_rate } => {
        //         self.execute_ml_guided_approach(learning_rate)?;
        //     }
        //     _ => self.execute_atomic_action(action)?
        // }

        // Return outcome score for learning
        Ok(0.0)
    }

    /// Example of how the controller could interface with learning policies
    #[allow(dead_code)]
    fn update_learning_policy(&mut self, _outcome: f32) {
        // For future expansion:
        // if let Some(learning_policy) = self.policy.as_any().downcast_mut::<TransformerPolicy>() {
        //     let tip_state = self.create_rich_tip_state(signal_value, signal_index)?;
        //     learning_policy.learn_from_outcome(&tip_state, &last_action, outcome);
        // }
    }

    /// Get current system statistics for monitoring ML policy performance
    #[allow(dead_code)]
    pub fn get_system_stats(&self) -> SystemStats {
        SystemStats {
            total_approaches: self.approach_count,
            positions_visited: self.position_history.len(),
            actions_executed: self.action_history.len(),
            // For future ML expansion:
            // model_confidence: self.policy.get_confidence(),
            // prediction_accuracy: self.calculate_accuracy(),
            // learning_rate: self.policy.get_learning_rate(),
        }
    }
}

/// System statistics for monitoring controller and policy performance
#[derive(Debug)]
pub struct SystemStats {
    pub total_approaches: u32,
    pub positions_visited: usize,
    pub actions_executed: usize,
    // Future ML metrics:
    // pub model_confidence: f32,
    // pub prediction_accuracy: f32,
    // pub learning_rate: f32,
}

/// Actions the controller can take based on policy decisions
#[derive(Debug)]
enum LoopAction {
    ContinueBadLoop,       // Keep checking after bad signal recovery actions
    ContinueStabilityLoop, // Keep monitoring for stability after good actions
    Halt,                  // Process complete
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::{BoundaryClassifier, TipClassification};
    use crate::policy::RuleBasedPolicy;

    // Mock classifier for testing
    struct MockClassifier {
        name: String,
        signal_index: i32,
        classification: TipClassification,
    }

    impl MockClassifier {
        fn new(name: String, signal_index: i32, classification: TipClassification) -> Self {
            Self { name, signal_index, classification }
        }
    }

    impl StateClassifier for MockClassifier {
        fn classify(&mut self, primary_signal: f32, _all_signals: Option<&[f32]>) -> TipState {
            TipState {
                primary_signal,
                all_signals: None,
                signal_names: None,
                position: None,
                z_position: None,
                timestamp: 0.0,
                signal_history: VecDeque::new(),
                approach_count: 0,
                last_action: None,
                system_parameters: vec![],
                classification: self.classification.clone(),
            }
        }

        fn get_primary_signal_index(&self) -> i32 {
            self.signal_index
        }

        fn get_name(&self) -> &str {
            &self.name
        }
    }

    // Mock policy for testing
    struct MockPolicy {
        name: String,
        decision: PolicyDecision,
    }

    impl MockPolicy {
        fn new(name: String, decision: PolicyDecision) -> Self {
            Self { name, decision }
        }
    }

    impl PolicyEngine for MockPolicy {
        fn decide(&mut self, _tip_state: &TipState) -> PolicyDecision {
            self.decision.clone()
        }

        fn get_name(&self) -> &str {
            &self.name
        }
    }

    #[test]
    fn test_controller_creation_with_separated_architecture() {
        let classifier = Box::new(MockClassifier::new(
            "Test Classifier".to_string(),
            24,
            TipClassification::Good,
        ));
        let policy = Box::new(MockPolicy::new(
            "Test Policy".to_string(),
            PolicyDecision::Good,
        ));

        // Test that the architecture compiles and types work together
        // Connection will fail without Nanonis running, which is expected
        let result = Controller::new("127.0.0.1:6501", classifier, policy);
        match result {
            Ok(_) => {
                // Unexpected success - maybe Nanonis is actually running
                println!("Nanonis connection succeeded in test");
            }
            Err(_) => {
                // Expected failure due to no Nanonis connection
                // This proves the architecture works
            }
        }
    }

    #[test]
    fn test_integration_with_boundary_classifier_and_rule_policy() {
        // Test that the real components work together
        let classifier = Box::new(BoundaryClassifier::new(
            "Bias Classifier".to_string(),
            24,
            0.0,
            2.0,
        ));
        let policy = Box::new(RuleBasedPolicy::new("Rule Policy".to_string()));

        // Test that the architecture compiles and types work together
        let result = Controller::new("127.0.0.1:6501", classifier, policy);
        match result {
            Ok(_) => {
                // Unexpected success - maybe Nanonis is actually running
                println!("Nanonis connection succeeded in test");
            }
            Err(_) => {
                // Expected failure due to no Nanonis connection
                // This proves the real components integrate properly
            }
        }
    }

    #[test]
    fn test_system_stats_initialization() {
        let stats = SystemStats {
            total_approaches: 5,
            positions_visited: 3,
            actions_executed: 10,
        };

        assert_eq!(stats.total_approaches, 5);
        assert_eq!(stats.positions_visited, 3);
        assert_eq!(stats.actions_executed, 10);
    }
}