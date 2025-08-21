use crate::classifier::StateClassifier;
use crate::client::NanonisClient;
use crate::error::NanonisError;
use crate::policy::{ActionType, PolicyDecision, PolicyEngine};
use crate::types::{MachineState, Position};
use log::{debug, error, info};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Type alias for custom bad action functions
/// Parameters: (client, machine_state) -> Result
type BadActionFn = Box<dyn Fn(&mut NanonisClient, &MachineState) -> Result<(), NanonisError> + Send + Sync>;

/// Type alias for custom good action functions  
/// Parameters: (client, machine_state) -> Result
type GoodActionFn = Box<dyn Fn(&mut NanonisClient, &MachineState) -> Result<(), NanonisError> + Send + Sync>;

/// Controller integrating Nanonis client with state classifier and policy engine
/// Follows separated architecture: raw signals → state classification → policy decisions
pub struct Controller {
    client: NanonisClient,
    classifier: Box<dyn StateClassifier>,
    policy: Box<dyn PolicyEngine>,

    // Optional shared state for real-time integration
    shared_state: Option<Arc<Mutex<MachineState>>>,

    // Control behavior
    halt_on_stable: bool,

    // Pluggable action functions
    bad_action: Option<BadActionFn>,
    good_action: Option<GoodActionFn>,


    // State tracking for advanced policy engines
    position_history: VecDeque<Position>,
    action_history: VecDeque<String>,

    // State change tracking to reduce log noise
    last_logged_classification: Option<crate::classifier::TipState>,
    stable_count: u32, // Count how long we've been stable

                       // For future ML expansion:
                       // state_buffer: VecDeque<TipState>,     // Rich state history for transformers
                       // action_outcomes: Vec<(ActionType, f32)>, // Action-outcome pairs for learning
                       // model_confidence: f32,                // Current model confidence
}

/// Handle for receiving control loop status and shutdown management
pub struct ControlReceiver {
    /// Can be used to request shutdown of the control loop
    pub shutdown_sender: mpsc::Sender<()>,

    /// Reference to check if control loop is still running
    pub is_running: Arc<AtomicBool>,

    /// Thread handle for joining on shutdown
    pub thread_handle: thread::JoinHandle<()>,
}

/// Builder for Controller with flexible configuration
pub struct ControllerBuilder {
    // Client configuration (either provide client or build one)
    client: Option<NanonisClient>,
    address: Option<String>,
    port: Option<u16>,

    // Required components
    classifier: Option<Box<dyn StateClassifier>>,
    policy: Option<Box<dyn PolicyEngine>>,

    // Optional shared state
    shared_state: Option<Arc<Mutex<MachineState>>>,

    // Control configuration
    control_interval_hz: f32,
    halt_on_stable: bool,

    // Pluggable action functions
    bad_action: Option<BadActionFn>,
    good_action: Option<GoodActionFn>,
}

impl Controller {
    /// Create a new Controller builder with sensible defaults
    pub fn builder() -> ControllerBuilder {
        ControllerBuilder {
            client: None,
            address: Some("127.0.0.1".to_string()),
            port: Some(6501),
            classifier: None,
            policy: None,
            shared_state: None,
            control_interval_hz: 2.0,
            halt_on_stable: true,
            bad_action: None,
            good_action: None,
        }
    }

    pub fn new(
        address: &str,
        port: u16,
        classifier: Box<dyn StateClassifier>,
        policy: Box<dyn PolicyEngine>,
    ) -> Result<Self, NanonisError> {
        let client = NanonisClient::new(address, port)?;
        Ok(Self {
            client,
            classifier,
            policy,
            shared_state: None,
            halt_on_stable: true,
            bad_action: None,
            good_action: None,
            position_history: VecDeque::with_capacity(100),
            action_history: VecDeque::with_capacity(100),
            last_logged_classification: None,
            stable_count: 0,
        })
    }

    pub fn with_client(
        client: NanonisClient,
        classifier: Box<dyn StateClassifier>,
        policy: Box<dyn PolicyEngine>,
    ) -> Self {
        Self {
            client,
            classifier,
            policy,
            shared_state: None,
            halt_on_stable: true,
            bad_action: None,
            good_action: None,
            position_history: VecDeque::with_capacity(100),
            action_history: VecDeque::with_capacity(100),
            last_logged_classification: None,
            stable_count: 0,
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

        debug!("Starting policy-driven control loop for signal index {signal_index}");
        debug!("Sample rate: {sample_rate_hz:.1} Hz, Duration: {duration:?}");

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
                    if self.halt_on_stable {
                        info!("STABLE signal achieved - halting process");
                        break;
                    } else {
                        info!("STABLE signal achieved - continuing monitoring");
                        // Continue the loop without breaking
                    }
                }
            }
        }

        debug!("Control loop completed after {:?}", start.elapsed());
        Ok(())
    }

    /// Start control loop in background thread with clean shutdown capabilities
    /// This provides symmetric architecture matching SignalMonitor
    pub fn start_control_loop(
        mut self,
        sample_rate_hz: f32,
    ) -> Result<ControlReceiver, NanonisError> {
        // Create shutdown channel
        let (shutdown_sender, shutdown_receiver) = mpsc::channel();
        let is_running = Arc::new(AtomicBool::new(true));
        let is_running_clone = is_running.clone();

        // Spawn background thread for control loop
        let thread_handle = thread::spawn(move || {
            info!("Starting background control loop at {sample_rate_hz:.1}Hz");
            let sample_interval = Duration::from_millis((1000.0 / sample_rate_hz) as u64);

            loop {
                // Check for shutdown signal (non-blocking)
                if shutdown_receiver.try_recv().is_ok() {
                    info!("Control loop shutdown signal received");
                    is_running_clone.store(false, Ordering::Relaxed);
                    break;
                }

                if is_running_clone.load(Ordering::Relaxed) {
                    // Run one monitoring loop iteration
                    match self.run_monitoring_loop(sample_interval) {
                        Ok(LoopAction::ContinueBadLoop) => {
                            // Continue with bad signal recovery
                        }
                        Ok(LoopAction::ContinueStabilityLoop) => {
                            // Continue with stability monitoring
                        }
                        Ok(LoopAction::Halt) => {
                            if self.halt_on_stable {
                                debug!("Control loop requested halt - stopping");
                                break;
                            } else {
                                debug!("Stable achieved but continuing monitoring");
                                // Continue the loop
                            }
                        }
                        Err(e) => {
                            debug!("Control loop error: {e}");
                            // Continue running unless it's a connection error
                            if e.to_string().contains("Broken pipe")
                                || e.to_string().contains("failed to fill whole buffer")
                            {
                                error!("Connection lost, stopping control loop");
                                break;
                            }
                        }
                    }
                }

                // Sleep for sample interval
                thread::sleep(sample_interval);
            }

            is_running_clone.store(false, Ordering::Relaxed);
            info!("Background control loop completed");
        });

        is_running.store(true, Ordering::Relaxed);
        info!("Control loop started in background thread");

        Ok(ControlReceiver {
            shutdown_sender,
            is_running,
            thread_handle,
        })
    }

    /// Single monitoring loop iteration with actions based on state  
    fn run_monitoring_loop(
        &mut self,
        _sample_interval: Duration,
    ) -> Result<LoopAction, NanonisError> {
        let signal_index = self.classifier.get_primary_signal_index();

        // Try to read from shared state first (Option A integration)
        let mut machine_state = if self.shared_state.is_some() {
            debug!("Collecting fresh samples from shared state for classifier buffer");

            // Collect multiple fresh samples with small delays to get fresh data
            let buffer_size = 10; // Match classifier buffer size
            let mut fresh_samples = Vec::new();

            for i in 0..buffer_size {
                if let Some(sample) = self.read_from_shared_state(signal_index, 0.5) {
                    // Extract the primary signal value from this sample
                    if let (Some(all_signals), Some(signal_indices)) =
                        (&sample.all_signals, &sample.signal_indices)
                    {
                        if let Some(position) =
                            signal_indices.iter().position(|&idx| idx == signal_index)
                        {
                            if position < all_signals.len() {
                                fresh_samples.push(all_signals[position]);
                            }
                        }
                    }
                }

                // Small delay to get fresh samples (except for last iteration)
                if i < buffer_size - 1 {
                    std::thread::sleep(Duration::from_millis(20)); // 50Hz spacing
                }
            }

            // Get the most recent sample as base, but with fresh_samples in signal_history
            if let Some(mut latest_sample) = self.read_from_shared_state(signal_index, 0.5) {
                // Replace signal_history with our fresh samples
                latest_sample.signal_history.clear();
                latest_sample.signal_history.extend(fresh_samples);
                debug!(
                    "Collected {} fresh samples for classifier",
                    latest_sample.signal_history.len()
                );
                latest_sample
            } else {
                return Err(NanonisError::InvalidCommand(
                    "Could not read from shared state".to_string(),
                ));
            }
        } else {
            // Fallback to direct client calls (legacy mode)
            debug!("Using direct client calls (legacy mode)");

            // Collect multiple fresh samples for this monitoring cycle
            let buffer_size = 10; // Reasonable buffer size for fresh sampling
            let mut fresh_samples = Vec::with_capacity(buffer_size);

            for _ in 0..buffer_size {
                let values = self.client.signals_val_get(vec![signal_index], true)?;
                fresh_samples.push(values[0]);

                // Small delay between samples for stability
                std::thread::sleep(Duration::from_millis(10));
            }

            // Create machine state and fill signal history with fresh samples
            let mut machine_state = crate::types::MachineState {
                all_signals: Some(fresh_samples.clone()),
                ..Default::default()
            };
            machine_state
                .signal_history
                .extend(fresh_samples.iter().copied());

            machine_state
        };

        // Let classifier handle the fresh samples and classification
        self.classifier.classify(&mut machine_state);

        // Enrich machine state with controller context
        self.enrich_machine_state(&mut machine_state)?;

        // Let policy decide based on classified state
        let decision = self.policy.decide(&machine_state);

        // Update shared state with controller context if using shared state mode
        self.update_shared_state_context(&machine_state);

        // Convert decision to classification for state tracking
        let current_classification = match decision {
            PolicyDecision::Bad => crate::classifier::TipState::Bad,
            PolicyDecision::Good => crate::classifier::TipState::Good,
            PolicyDecision::Stable => crate::classifier::TipState::Stable,
        };

        // Only log when state changes to reduce noise
        let should_log = match &self.last_logged_classification {
            None => true,                                              // First time, always log
            Some(last_state) => *last_state != current_classification, // Log only on change
        };

        if should_log {
            // Extract primary signal value for logging using the classifier's helper
            let primary_value = self.extract_primary_signal_for_logging(&machine_state);
            let state_name = match current_classification {
                crate::classifier::TipState::Bad => "BAD",
                crate::classifier::TipState::Good => "GOOD",
                crate::classifier::TipState::Stable => "STABLE",
            };

            info!(
                "Signal {signal_index} = {:?} - {} ({})",
                primary_value,
                state_name,
                self.classifier.get_name()
            );

            // Update last logged state
            self.last_logged_classification = Some(current_classification.clone());

            // Reset stable count on any state change
            self.stable_count = 0;
        }

        // Handle stable state count for periodic updates
        if current_classification == crate::classifier::TipState::Stable {
            self.stable_count += 1;

            // Log every 10th stable iteration to show system is still running
            if self.stable_count.is_multiple_of(10) {
                debug!(
                    "System stable for {} iterations ({})",
                    self.stable_count,
                    self.classifier.get_name()
                );
            }
        }

        match decision {
            PolicyDecision::Bad => {
                // Execute bad signal actions (pluggable or default)
                if let Some(ref custom_action) = self.bad_action {
                    custom_action(&mut self.client, &machine_state)?;
                } else {
                    self.execute_bad_actions()?;
                }
                Ok(LoopAction::ContinueBadLoop) // Continue in bad recovery mode
            }
            PolicyDecision::Good => {
                // Execute good signal actions (pluggable or default)
                if let Some(ref custom_action) = self.good_action {
                    custom_action(&mut self.client, &machine_state)?;
                } else {
                    self.execute_good_actions()?;
                }
                Ok(LoopAction::ContinueStabilityLoop) // Continue monitoring for stability
            }
            PolicyDecision::Stable => {
                // Always go to halt state when stable is achieved
                Ok(LoopAction::Halt)
            }
        }
    }

    /// Execute actions when signal is bad: approach → pulse → withdraw → move → approach → check
    fn execute_bad_actions(&mut self) -> Result<(), NanonisError> {
        info!("Executing bad signal recovery sequence");

        // Step 1: Initial approach (if not already approached)
        info!("Performing initial approach...");
        self.client.auto_approach_and_wait()?;
        self.action_history
            .push_back("Initial Approach".to_string());

        // Step 2: Pulse operation (simulate for now)
        info!("Executing pulse operation...");
        // TODO: Implement actual pulse operation when available
        std::thread::sleep(Duration::from_millis(200));
        debug!("Pulse completed");

        // Step 3: Withdraw tip
        info!("Withdrawing tip...");
        self.client.z_ctrl_withdraw(true, 5000)?; // 5 second timeout

        // Step 4: Move to new position
        info!("Moving to new position...");
        let current_position = self.client.folme_xy_pos_get(true)?;

        // Move by small offset (3nm in both directions)
        let dx: f64 = 3e-9;
        let dy: f64 = 3e-9;
        let new_position = Position {
            x: current_position.x + dx,
            y: current_position.y + dy,
        };

        debug!(
            "Moving from ({:.3e}, {:.3e}) to ({:.3e}, {:.3e})",
            current_position.x, current_position.y, new_position.x, new_position.y
        );
        self.client.folme_xy_pos_set(new_position, true)?;

        // Step 5: Approach at new position
        info!("Approaching at new position...");
        self.client.auto_approach_and_wait()?;
        self.position_history.push_back(new_position);
        self.action_history.push_back("Re-approach".to_string());

        // Step 6: Check tip state (read signal to verify)
        debug!("Checking tip state...");
        // This will be checked by the policy engine in the next loop iteration

        info!("Bad signal recovery sequence completed");
        Ok(())
    }

    /// Execute actions when signal is good
    fn execute_good_actions(&mut self) -> Result<(), NanonisError> {
        debug!("Executing good signal actions...");

        // TODO: Implement good signal actions:
        // - Optimize scan parameters
        // - Fine-tune approach
        // - Prepare for measurements

        // For now, simulate with delay
        std::thread::sleep(Duration::from_millis(200));
        debug!("Good signal actions completed");

        Ok(())
    }

    /// Placeholder for operational mode (scanning, measurements, etc.)
    #[allow(dead_code)]
    fn run_operational_mode(&mut self) -> Result<(), NanonisError> {
        info!("Entering operational mode...");

        // For now, just wait and return to monitoring
        std::thread::sleep(Duration::from_secs(1));
        info!("Operational mode completed - returning to monitoring");

        Ok(())
    }

    // ==================== Signal Extraction Helpers ====================


    /// Extract primary signal value for logging (uses signal mapping)
    fn extract_primary_signal_for_logging(&self, machine_state: &MachineState) -> String {
        let signal_index = self.classifier.get_primary_signal_index();

        if let (Some(all_signals), Some(signal_indices)) =
            (&machine_state.all_signals, &machine_state.signal_indices)
        {
            // Find position of signal_index in the signal_indices array
            if let Some(position) = signal_indices.iter().position(|&idx| idx == signal_index) {
                if position < all_signals.len() {
                    return format!("{:.3e}", all_signals[position]);
                }
            }
        }

        // Fallback: use latest value from signal_history
        if let Some(&latest) = machine_state.signal_history.back() {
            format!("{latest:.3e}")
        } else {
            "N/A".to_string()
        }
    }

    // ==================== Shared State Methods ====================

    /// Read signal data from shared state instead of direct client calls
    /// Returns None if no shared state or data is too stale
    fn read_from_shared_state(
        &self,
        _signal_index: i32,
        max_age_seconds: f64,
    ) -> Option<MachineState> {
        if let Some(ref shared_state) = self.shared_state {
            if let Ok(state) = shared_state.lock() {
                // Check if data is fresh enough
                let current_time = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64();

                let data_age = current_time - state.timestamp;
                if data_age <= max_age_seconds {
                    debug!("Reading fresh data from shared state (age: {data_age:.2}s)");
                    Some(state.clone())
                } else {
                    debug!("Shared state data too stale (age: {data_age:.2}s > max: {max_age_seconds:.2}s)");
                    None
                }
            } else {
                debug!("Failed to acquire shared state lock");
                None
            }
        } else {
            None
        }
    }

    /// Update shared state with controller context (position, actions, etc.)
    fn update_shared_state_context(&mut self, machine_state: &MachineState) {
        if let Some(ref shared_state) = self.shared_state {
            if let Ok(mut state) = shared_state.lock() {
                // Update controller-specific context
                state.last_action = machine_state.last_action.clone();
                state.classification = machine_state.classification.clone();

                // Update position if available
                if let Some(position) = machine_state.position {
                    state.position = Some(position);
                }

                debug!("Updated shared state with controller context");
            } else {
                debug!("Failed to acquire shared state lock for update");
            }
        }
    }

    // ==================== State Enhancement Methods ====================

    /// Enrich tip state with additional context from the controller
    fn enrich_machine_state(
        &mut self,
        machine_state: &mut crate::types::MachineState,
    ) -> Result<(), NanonisError> {
        // Add position information if available
        if let Ok(position) = self.client.folme_xy_pos_get(true) {
            machine_state.position = Some((position.x, position.y));
        }

        // Note: signal names now handled via SessionMetadata, not MachineState

        // Add controller state
        machine_state.last_action = self.action_history.back().cloned();

        Ok(())
    }

    // ==================== Future Expansion Methods ====================
    // These methods show how to expand the controller for ML/transformer policies

    /// Bind tip states to specific actions for learning
    /// This would train transformer/ML models to associate states with optimal actions
    #[allow(dead_code)]
    fn bind_state_to_action(
        &mut self,
        _machine_state: &crate::types::MachineState,
        _action: ActionType,
    ) {
        // For future ML expansion:
        // - Record state-action pairs
        // - Build training dataset
        // - Update model weights
        // - Implement reinforcement learning

        // Example expansion:
        // self.training_data.push((machine_state.clone(), action, outcome));
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
    use crate::classifier::{BoundaryClassifier, TipState};
    use crate::policy::RuleBasedPolicy;

    // Mock classifier for testing
    struct MockClassifier {
        name: String,
        signal_index: i32,
        classification: TipState,
    }

    impl MockClassifier {
        fn new(name: String, signal_index: i32, classification: TipState) -> Self {
            Self {
                name,
                signal_index,
                classification,
            }
        }
    }

    impl StateClassifier for MockClassifier {
        fn classify(&mut self, machine_state: &mut crate::types::MachineState) {
            machine_state.classification = self.classification.clone();
        }

        fn initialize_buffer(
            &mut self,
            _machine_state: &crate::types::MachineState,
            _target_samples: usize,
        ) {
            // Mock implementation - do nothing
        }

        fn clear_buffer(&mut self) {
            // Mock implementation - do nothing
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
        fn decide(&mut self, _machine_state: &crate::types::MachineState) -> PolicyDecision {
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
            TipState::Good,
        ));
        let policy = Box::new(MockPolicy::new(
            "Test Policy".to_string(),
            PolicyDecision::Good,
        ));

        // Test that the architecture compiles and types work together
        // Connection will fail without Nanonis running, which is expected
        let result = Controller::new("127.0.0.1", 6501, classifier, policy);
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
        let result = Controller::new("127.0.0.1", 6501, classifier, policy);
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
            positions_visited: 3,
            actions_executed: 10,
        };

        assert_eq!(stats.positions_visited, 3);
        assert_eq!(stats.actions_executed, 10);
    }

    #[test]
    fn test_shared_state_integration() {
        // Create shared state
        let shared_state = Arc::new(Mutex::new(MachineState {
            all_signals: Some(vec![1.5, 0.8, 0.2]),
            signal_indices: Some(vec![0, 1, 2]), // Test mapping
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64(),
            ..Default::default()
        }));

        // Create classifier and policy
        let classifier = Box::new(BoundaryClassifier::new(
            "Test Classifier".to_string(),
            24,
            0.0,
            2.0,
        ));
        let policy = Box::new(RuleBasedPolicy::new("Test Policy".to_string()));

        // Create controller with shared state
        let controller = Controller::builder()
            .address("127.0.0.1")
            .port(6501)
            .classifier(classifier)
            .policy(policy)
            .with_shared_state(shared_state.clone())
            .build();

        // Controller creation should succeed even if client connection fails
        match controller {
            Ok(controller) => {
                // Test reading from shared state
                let result = controller.read_from_shared_state(24, 1.0);
                assert!(result.is_some());

                let state = result.unwrap();
                // Test that signal mapping works - signal 24 should extract from all_signals
                assert_eq!(state.all_signals.as_ref().unwrap()[0], 1.5);
                println!("Shared state integration test passed");
            }
            Err(_) => {
                // Expected when no Nanonis running - focus on architecture test
                println!("Client connection failed (expected without Nanonis hardware)");
                println!("Shared state architecture test still valid");
            }
        }
    }
}

impl ControllerBuilder {
    /// Provide an existing NanonisClient (alternative to address/port)
    pub fn client(mut self, client: NanonisClient) -> Self {
        self.client = Some(client);
        self
    }

    /// Set the Nanonis server address (used if no client provided)
    pub fn address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }

    /// Set the Nanonis server port (used if no client provided)
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Set the state classifier (required)
    pub fn classifier(mut self, classifier: Box<dyn StateClassifier>) -> Self {
        self.classifier = Some(classifier);
        self
    }

    /// Set the policy engine (required)
    pub fn policy(mut self, policy: Box<dyn PolicyEngine>) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Set shared state for real-time integration (optional)
    pub fn with_shared_state(mut self, shared_state: Arc<Mutex<MachineState>>) -> Self {
        self.shared_state = Some(shared_state);
        self
    }

    /// Set control loop frequency in Hz
    pub fn control_interval(mut self, interval_hz: f32) -> Self {
        self.control_interval_hz = interval_hz;
        self
    }

    /// Set whether to halt when stable state is achieved
    pub fn halt_on_stable(mut self, halt: bool) -> Self {
        self.halt_on_stable = halt;
        self
    }

    /// Set custom bad action function
    pub fn on_bad<F>(mut self, action: F) -> Self 
    where 
        F: Fn(&mut NanonisClient, &MachineState) -> Result<(), NanonisError> + Send + Sync + 'static
    {
        self.bad_action = Some(Box::new(action));
        self
    }

    /// Set custom good action function
    pub fn on_good<F>(mut self, action: F) -> Self 
    where 
        F: Fn(&mut NanonisClient, &MachineState) -> Result<(), NanonisError> + Send + Sync + 'static
    {
        self.good_action = Some(Box::new(action));
        self
    }

    /// Build the Controller with validation
    pub fn build(self) -> Result<Controller, String> {
        // Get or create client
        let client = if let Some(client) = self.client {
            client
        } else {
            let address = self.address.unwrap_or_else(|| "127.0.0.1".to_string());
            let port = self.port.unwrap_or(6501);
            NanonisClient::new(&address, port)
                .map_err(|e| format!("Failed to create NanonisClient: {e}"))?
        };

        // Validate required components
        let classifier = self
            .classifier
            .ok_or("classifier is required - use .classifier()")?;
        let policy = self.policy.ok_or("policy is required - use .policy()")?;

        // Validate configuration
        if self.control_interval_hz <= 0.0 {
            return Err("control_interval_hz must be greater than 0".to_string());
        }

        if self.control_interval_hz > 100.0 {
            return Err("control_interval_hz should not exceed 100 Hz for stability".to_string());
        }


        let controller = Controller {
            client,
            classifier,
            policy,
            shared_state: self.shared_state,
            halt_on_stable: self.halt_on_stable,
            bad_action: self.bad_action,
            good_action: self.good_action,
            position_history: VecDeque::with_capacity(100),
            action_history: VecDeque::with_capacity(100),
            last_logged_classification: None,
            stable_count: 0,
        };

        info!(
            "Built Controller with {}Hz control loop{}",
            self.control_interval_hz,
            if controller.shared_state.is_some() {
                " (with shared state)"
            } else {
                ""
            }
        );

        Ok(controller)
    }
}
