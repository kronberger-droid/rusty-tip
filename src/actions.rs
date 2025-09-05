use crate::{
    types::{
        ActionCondition, MovementMode, Position, Position3D, ScanAction, SignalIndex, SignalValue,
    },
    MotorDirection,
};
use std::time::Duration;

/// Enhanced Action enum representing all possible SPM operations
/// Properly separates motor (step-based) and piezo (continuous) movements
#[derive(Debug, Clone)]
pub enum Action {
    // === Signal Operations ===
    /// Read single signal value
    ReadSignal {
        signal: SignalIndex,
        wait_for_newest: bool,
    },

    /// Read multiple signal values
    ReadSignals {
        signals: Vec<SignalIndex>,
        wait_for_newest: bool,
    },

    /// Read all available signal names
    ReadSignalNames,

    // === Bias Operations ===
    /// Read current bias voltage
    ReadBias,

    /// Set bias voltage to specific value
    SetBias { voltage: f32 },

    // === Fine Positioning Operations (Piezo) ===
    /// Read current piezo position (continuous coordinates)
    ReadPiezoPosition { wait_for_newest_data: bool },

    /// Set piezo position (absolute)
    SetPiezoPosition {
        position: Position,
        wait_until_finished: bool,
    },

    /// Move piezo position (relative to current)
    MovePiezoRelative { delta: Position },

    // === Coarse Positioning Operations (Motor) ===
    /// Move motor in steps (discrete positioning)
    MoveMotor {
        direction: MotorDirection,
        steps: u16,
    },

    /// Move motor using closed-loop to target position
    MoveMotorClosedLoop {
        target: Position3D,
        mode: MovementMode,
    },

    /// Stop all motor movement
    StopMotor,

    // === Control Operations ===
    /// Perform auto-approach and wait for completion
    AutoApproach,

    /// Withdraw tip with timeout
    Withdraw {
        wait_until_finished: bool,
        timeout_ms: Duration,
    },

    // === Scan Operations ===
    /// Control scan operations
    ScanControl { action: ScanAction },

    /// Read scan status
    ReadScanStatus,

    // === Advanced Operations ===
    /// Execute bias pulse with parameters
    BiasPulse {
        wait_until_done: bool,
        pulse_width_s: Duration,
        bias_value_v: f32,
        z_controller_hold: u16,
        pulse_mode: u16,
    },

    /// Wait for a specific duration
    Wait { duration: Duration },

    // === Conditional Operations (Future Extension) ===
    /// Execute action only if condition is met
    Conditional {
        condition: ActionCondition,
        action: Box<Action>,
    },

    // === Data Management ===
    /// Store result value with key for later retrieval
    Store { key: String, action: Box<Action> },

    /// Retrieve previously stored value
    Retrieve { key: String },
}

/// Enhanced ActionResult with proper typing and units
#[derive(Debug, Clone)]
pub enum ActionResult {
    // === Signal Results ===
    /// Multiple signal values with proper typing
    Signals(Vec<SignalValue>),

    /// List of available signal names
    SignalNames(Vec<String>),

    // === Position Results ===
    /// Piezo position (continuous coordinates)
    PiezoPosition(Position),

    // === Single Value Results ===
    /// Bias voltage value
    BiasVoltage(f32),

    /// Scan status
    ScanStatus(bool),

    /// Stored value retrieved by key
    StoredValue(Box<ActionResult>),

    // === Status Results ===
    /// Operation completed successfully (no return value)
    Success,

    /// No result (e.g., Wait operations)
    None,

    /// Partial success (some actions in sequence succeeded)
    PartialSuccess(Vec<ActionResult>),
}

impl ActionResult {
    /// Convert to f64 if possible (for numerical results)
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ActionResult::BiasVoltage(v) => Some(*v as f64),
            ActionResult::Signals(signals) => {
                if signals.len() == 1 {
                    Some(signals[0].value())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Convert to Position if possible
    pub fn as_position(&self) -> Option<Position> {
        match self {
            ActionResult::PiezoPosition(pos) => Some(*pos),
            _ => None,
        }
    }
}

// === Action Categorization ===

impl Action {
    /// Check if this is a positioning action
    pub fn is_positioning_action(&self) -> bool {
        matches!(
            self,
            Action::SetPiezoPosition { .. }
                | Action::MovePiezoRelative { .. }
                | Action::MoveMotor { .. }
                | Action::MoveMotorClosedLoop { .. }
        )
    }

    /// Check if this is a read-only action
    pub fn is_read_action(&self) -> bool {
        matches!(
            self,
            Action::ReadSignal { .. }
                | Action::ReadSignals { .. }
                | Action::ReadSignalNames
                | Action::ReadBias
                | Action::ReadPiezoPosition { .. }
                | Action::ReadScanStatus
                | Action::Retrieve { .. }
        )
    }

    /// Check if this is a control action
    pub fn is_control_action(&self) -> bool {
        matches!(
            self,
            Action::AutoApproach
                | Action::Withdraw { .. }
                | Action::ScanControl { .. }
                | Action::StopMotor
        )
    }

    /// Check if this action modifies bias voltage
    pub fn modifies_bias(&self) -> bool {
        matches!(self, Action::SetBias { .. } | Action::BiasPulse { .. })
    }

    /// Check if this action involves motor movement
    pub fn involves_motor(&self) -> bool {
        matches!(
            self,
            Action::MoveMotor { .. } | Action::MoveMotorClosedLoop { .. } | Action::StopMotor
        )
    }

    /// Check if this action involves piezo movement
    pub fn involves_piezo(&self) -> bool {
        matches!(
            self,
            Action::SetPiezoPosition { .. }
                | Action::MovePiezoRelative { .. }
                | Action::ReadPiezoPosition { .. }
        )
    }

    /// Get a human-readable description of the action
    pub fn description(&self) -> String {
        match self {
            Action::ReadSignal { signal, .. } => {
                format!("Read signal {}", signal.0)
            }
            Action::ReadSignals { signals, .. } => {
                let indices: Vec<i32> = signals.iter().map(|s| s.0).collect();
                format!("Read signals: {:?}", indices)
            }
            Action::SetBias { voltage } => {
                format!("Set bias to {:.3}V", voltage)
            }
            Action::SetPiezoPosition { position, .. } => {
                format!(
                    "Set piezo position to ({:.3e}, {:.3e})",
                    position.x, position.y
                )
            }
            Action::MoveMotor { direction, steps } => {
                format!("Move motor {:?} {} steps", direction, steps)
            }
            Action::AutoApproach => "Auto approach".to_string(),
            Action::Withdraw { timeout_ms, .. } => {
                format!("Withdraw tip (timeout: {}ms)", timeout_ms.as_micros())
            }
            Action::Wait { duration } => {
                format!("Wait {:.1}s", duration.as_secs_f64())
            }
            Action::BiasPulse {
                wait_until_done: _,
                pulse_width_s,
                bias_value_v,
                z_controller_hold: _,
                pulse_mode: _,
            } => {
                format!("Bias pulse {:.3}V for {:?}ms", bias_value_v, pulse_width_s)
            }
            _ => format!("{:?}", self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_result_extraction() {
        let bias_result = ActionResult::BiasVoltage(2.5);
        assert_eq!(bias_result.as_f64(), Some(2.5));

        let position_result = ActionResult::PiezoPosition(Position { x: 1e-9, y: 2e-9 });
        assert_eq!(
            position_result.as_position(),
            Some(Position { x: 1e-9, y: 2e-9 })
        );
    }
}

/// A sequence of actions with simple Vec<Action> foundation
#[derive(Debug, Clone)]
pub struct ActionChain {
    actions: Vec<Action>,
    name: Option<String>,
}

impl ActionChain {
    /// Create a new ActionChain from a vector of actions
    pub fn new(actions: Vec<Action>) -> Self {
        Self {
            actions,
            name: None,
        }
    }

    /// Create a new ActionChain from any iterator of actions
    pub fn from_actions(actions: impl IntoIterator<Item = Action>) -> Self {
        Self::new(actions.into_iter().collect())
    }

    /// Create a new ActionChain with a name
    pub fn named(actions: Vec<Action>, name: impl Into<String>) -> Self {
        Self {
            actions,
            name: Some(name.into()),
        }
    }

    /// Create an empty ActionChain
    pub fn empty() -> Self {
        Self::new(vec![])
    }

    // === Direct Vec<Action> Access ===

    /// Get immutable reference to actions
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// Get mutable reference to actions vector for direct manipulation
    pub fn actions_mut(&mut self) -> &mut Vec<Action> {
        &mut self.actions
    }

    /// Add an action to the end of the chain
    pub fn push(&mut self, action: Action) {
        self.actions.push(action);
    }

    /// Add multiple actions to the end of the chain
    pub fn extend(&mut self, actions: impl IntoIterator<Item = Action>) {
        self.actions.extend(actions);
    }

    /// Insert an action at a specific index
    pub fn insert(&mut self, index: usize, action: Action) {
        self.actions.insert(index, action);
    }

    /// Remove and return the action at index
    pub fn remove(&mut self, index: usize) -> Action {
        self.actions.remove(index)
    }

    /// Remove the last action and return it
    pub fn pop(&mut self) -> Option<Action> {
        self.actions.pop()
    }

    /// Clear all actions
    pub fn clear(&mut self) {
        self.actions.clear();
    }

    /// Create a new chain by appending another chain
    pub fn chain_with(mut self, other: ActionChain) -> Self {
        self.actions.extend(other.actions);
        self
    }

    /// Get an iterator over actions
    pub fn iter(&self) -> std::slice::Iter<'_, Action> {
        self.actions.iter()
    }

    /// Get a mutable iterator over actions
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Action> {
        self.actions.iter_mut()
    }

    // === Metadata Access ===

    /// Get the name of this chain
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Set the name of this chain
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = Some(name.into());
    }

    /// Get the number of actions in this chain
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Check if this chain is empty
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    // === Analysis Methods ===

    /// Get actions that match a specific category
    pub fn positioning_actions(&self) -> Vec<&Action> {
        self.actions
            .iter()
            .filter(|a| a.is_positioning_action())
            .collect()
    }

    pub fn read_actions(&self) -> Vec<&Action> {
        self.actions.iter().filter(|a| a.is_read_action()).collect()
    }

    pub fn control_actions(&self) -> Vec<&Action> {
        self.actions
            .iter()
            .filter(|a| a.is_control_action())
            .collect()
    }

    /// Check if chain contains any motor movements
    pub fn involves_motor(&self) -> bool {
        self.actions.iter().any(|a| a.involves_motor())
    }

    /// Check if chain contains any piezo movements
    pub fn involves_piezo(&self) -> bool {
        self.actions.iter().any(|a| a.involves_piezo())
    }

    /// Check if chain contains any bias modifications
    pub fn modifies_bias(&self) -> bool {
        self.actions.iter().any(|a| a.modifies_bias())
    }

    /// Get a summary description of the chain
    pub fn summary(&self) -> String {
        if let Some(name) = &self.name {
            format!("{} ({} actions)", name, self.len())
        } else {
            format!("Action chain with {} actions", self.len())
        }
    }

    /// Get detailed analysis of the chain
    pub fn analysis(&self) -> ChainAnalysis {
        ChainAnalysis {
            total_actions: self.len(),
            positioning_actions: self.positioning_actions().len(),
            read_actions: self.read_actions().len(),
            control_actions: self.control_actions().len(),
            involves_motor: self.involves_motor(),
            involves_piezo: self.involves_piezo(),
            modifies_bias: self.modifies_bias(),
        }
    }
}

/// Analysis result for an ActionChain
#[derive(Debug, Clone)]
pub struct ChainAnalysis {
    pub total_actions: usize,
    pub positioning_actions: usize,
    pub read_actions: usize,
    pub control_actions: usize,
    pub involves_motor: bool,
    pub involves_piezo: bool,
    pub modifies_bias: bool,
}

// === Iterator Support ===

impl IntoIterator for ActionChain {
    type Item = Action;
    type IntoIter = std::vec::IntoIter<Action>;

    fn into_iter(self) -> Self::IntoIter {
        self.actions.into_iter()
    }
}

impl<'a> IntoIterator for &'a ActionChain {
    type Item = &'a Action;
    type IntoIter = std::slice::Iter<'a, Action>;

    fn into_iter(self) -> Self::IntoIter {
        self.actions.iter()
    }
}

impl FromIterator<Action> for ActionChain {
    fn from_iter<T: IntoIterator<Item = Action>>(iter: T) -> Self {
        Self::from_actions(iter)
    }
}

impl From<Vec<Action>> for ActionChain {
    fn from(actions: Vec<Action>) -> Self {
        Self::new(actions)
    }
}

// ==================== Pre-built Common Patterns ====================

impl ActionChain {
    /// Comprehensive system status check
    pub fn system_status_check() -> Self {
        ActionChain::named(
            vec![
                Action::ReadSignalNames,
                Action::ReadBias,
                Action::ReadPiezoPosition {
                    wait_for_newest_data: true,
                },
            ],
            "System status check",
        )
    }

    /// Safe tip approach with verification
    pub fn safe_tip_approach() -> Self {
        ActionChain::named(
            vec![
                Action::ReadPiezoPosition {
                    wait_for_newest_data: true,
                },
                Action::AutoApproach,
                Action::Wait {
                    duration: Duration::from_millis(500),
                },
                Action::ReadSignal {
                    signal: SignalIndex(24),
                    wait_for_newest: true,
                }, // Typical bias voltage
                Action::ReadSignal {
                    signal: SignalIndex(0),
                    wait_for_newest: true,
                }, // Typical current
            ],
            "Safe tip approach",
        )
    }

    /// Move to position and approach
    pub fn move_and_approach(target: Position) -> Self {
        ActionChain::named(
            vec![
                Action::SetPiezoPosition {
                    position: target,
                    wait_until_finished: true,
                },
                Action::Wait {
                    duration: Duration::from_millis(100),
                },
                Action::AutoApproach,
                Action::ReadSignal {
                    signal: SignalIndex(24),
                    wait_for_newest: true,
                },
            ],
            format!("Move to ({:.1e}, {:.1e}) and approach", target.x, target.y),
        )
    }

    /// Bias pulse sequence with restoration
    pub fn bias_pulse_sequence(voltage: f32, duration_ms: u32) -> Self {
        ActionChain::named(
            vec![
                Action::ReadBias,
                Action::SetBias { voltage },
                Action::Wait {
                    duration: Duration::from_millis(50),
                },
                Action::Wait {
                    duration: Duration::from_millis(duration_ms as u64),
                },
                Action::SetBias { voltage: 0.0 },
            ],
            format!("Bias pulse {:.3}V for {}ms", voltage, duration_ms),
        )
    }

    /// Survey multiple positions
    pub fn position_survey(positions: Vec<Position>) -> Self {
        let position_count = positions.len(); // Store length before moving
        let mut actions = Vec::new();

        for pos in positions {
            actions.extend([
                Action::SetPiezoPosition {
                    position: pos,
                    wait_until_finished: true,
                },
                Action::Wait {
                    duration: Duration::from_millis(100),
                },
                Action::AutoApproach,
                Action::ReadSignal {
                    signal: SignalIndex(24),
                    wait_for_newest: true,
                }, // Bias voltage
                Action::ReadSignal {
                    signal: SignalIndex(0),
                    wait_for_newest: true,
                }, // Current
                Action::Withdraw {
                    wait_until_finished: true,
                    timeout_ms: Duration::from_secs(5),
                },
            ]);
        }

        ActionChain::named(
            actions,
            format!("Position survey ({} points)", position_count),
        )
    }

    /// Complete tip recovery sequence
    pub fn tip_recovery_sequence() -> Self {
        ActionChain::named(
            vec![
                Action::Withdraw {
                    wait_until_finished: true,
                    timeout_ms: Duration::from_secs(5),
                },
                Action::MovePiezoRelative {
                    delta: Position { x: 3e-9, y: 3e-9 },
                },
                Action::Wait {
                    duration: Duration::from_millis(200),
                },
                Action::AutoApproach,
                Action::ReadSignal {
                    signal: SignalIndex(24),
                    wait_for_newest: true,
                },
            ],
            "Tip recovery sequence",
        )
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;
    use crate::types::MotorDirection;

    #[test]
    fn test_vec_foundation() {
        // Test direct Vec<Action> usage
        let mut chain = ActionChain::new(vec![Action::ReadBias, Action::SetBias { voltage: 1.0 }]);

        assert_eq!(chain.len(), 2);

        // Test Vec operations
        chain.push(Action::AutoApproach);
        assert_eq!(chain.len(), 3);

        let action = chain.pop().unwrap();
        assert!(matches!(action, Action::AutoApproach));
        assert_eq!(chain.len(), 2);

        // Test extension
        chain.extend([Action::Wait { duration: Duration::from_millis(100) }, Action::ReadBias]);
        assert_eq!(chain.len(), 4);
    }

    #[test]
    fn test_simple_construction() {
        let chain = ActionChain::named(
            vec![
                Action::ReadBias,
                Action::SetBias { voltage: 1.0 },
                Action::Wait { duration: Duration::from_millis(100) },
                Action::AutoApproach,
            ],
            "Test chain",
        );

        assert_eq!(chain.name(), Some("Test chain"));
        assert_eq!(chain.len(), 4);

        let analysis = chain.analysis();
        assert_eq!(analysis.total_actions, 4);
        assert_eq!(analysis.read_actions, 1);
        assert_eq!(analysis.control_actions, 1);
        assert!(analysis.modifies_bias);
    }

    #[test]
    fn test_programmatic_generation() {
        // Test building chains programmatically
        let mut chain = ActionChain::empty();

        for _ in 0..3 {
            chain.push(Action::MoveMotor { direction: MotorDirection::XPlus, steps: 10 });
            chain.push(Action::Wait { duration: Duration::from_millis(100) });
        }

        assert_eq!(chain.len(), 6);
        assert!(chain.involves_motor());

        // Test iterator construction
        let actions: Vec<Action> = (0..5).map(|_| Action::ReadBias).collect();

        let iter_chain: ActionChain = actions.into_iter().collect();
        assert_eq!(iter_chain.len(), 5);
    }

    #[test]
    fn test_pre_built_patterns() {
        let status_check = ActionChain::system_status_check();
        assert!(status_check.name().is_some());
        assert!(status_check.len() > 0);

        let approach = ActionChain::safe_tip_approach();
        assert!(approach.control_actions().len() > 0);

        let positions = vec![Position { x: 1e-9, y: 1e-9 }, Position { x: 2e-9, y: 2e-9 }];
        let survey = ActionChain::position_survey(positions);
        assert_eq!(survey.len(), 12); // 6 actions per position × 2 positions
    }

    #[test]
    fn test_chain_analysis() {
        let chain = ActionChain::new(vec![
            Action::MoveMotor { direction: MotorDirection::XPlus, steps: 100 },
            Action::SetPiezoPosition {
                position: Position { x: 1e-9, y: 1e-9 },
                wait_until_finished: true,
            },
            Action::ReadBias,
            Action::AutoApproach,
            Action::SetBias { voltage: 1.5 },
        ]);

        let analysis = chain.analysis();
        assert_eq!(analysis.total_actions, 5);
        assert_eq!(analysis.positioning_actions, 2);
        assert_eq!(analysis.read_actions, 1);
        assert_eq!(analysis.control_actions, 1);
        assert!(analysis.involves_motor);
        assert!(analysis.involves_piezo);
        assert!(analysis.modifies_bias);
    }

    #[test]
    fn test_iteration() {
        let chain = ActionChain::new(vec![
            Action::ReadBias,
            Action::AutoApproach,
            Action::Wait { duration: Duration::from_millis(100) },
        ]);

        // Test iterator
        let mut count = 0;
        for _ in &chain {
            count += 1;
            // Can access action here
        }
        assert_eq!(count, 3);

        // Test into_iter
        let actions: Vec<Action> = chain.into_iter().collect();
        assert_eq!(actions.len(), 3);
    }

    #[test]
    fn test_from_vec_action() {
        // Test From<Vec<Action>> trait
        let actions = vec![
            Action::ReadBias,
            Action::SetBias { voltage: 1.5 },
            Action::AutoApproach,
        ];
        
        let chain: ActionChain = actions.into();
        assert_eq!(chain.len(), 3);
        assert!(chain.name().is_none());
        
        // Test that it's usable with Into<ActionChain> parameters
        let vec_actions = vec![
            Action::ReadBias,
            Action::Wait { duration: Duration::from_millis(50) },
        ];
        
        // This should compile thanks to Into<ActionChain>
        fn accepts_into_action_chain(_chain: impl Into<ActionChain>) {
            // This function would be called by execute methods
        }
        
        accepts_into_action_chain(vec_actions);
    }
}
