use crate::actions::{Action, ActionChain, ActionResult};
use crate::action_sequence::ActionSequence;
use crate::action_driver::ActionDriver;
use crate::error::NanonisError;

/// Execution priority levels for action sequences
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionPriority {
    Normal,
    High,
    Emergency,
}

/// High-level machine representation managing action execution and validation
pub struct MachineRepresentation {
    driver: ActionDriver,
}

impl MachineRepresentation {
    /// Create new machine representation with ActionDriver
    pub fn new(driver: ActionDriver) -> Self {
        Self { driver }
    }
    
    /// Execute any ActionSequence (built-in or custom)
    pub fn execute_sequence(&mut self, sequence: ActionSequence) -> Result<Vec<ActionResult>, NanonisError> {
        log::info!("Executing sequence: {}", sequence.name());
        self.driver.execute_chain(sequence.to_action_chain())
    }
    
    /// Execute ActionChain by converting to Custom sequence
    pub fn execute_chain(&mut self, chain: ActionChain) -> Result<Vec<ActionResult>, NanonisError> {
        let actions: Vec<Action> = chain.into_iter().collect();
        let sequence = ActionSequence::Custom(actions);
        self.execute_sequence(sequence)
    }
    
    /// Execute ActionSequence with priority and optional reason (future: implement priority handling)
    pub fn execute_with_priority(
        &mut self, 
        sequence: ActionSequence, 
        priority: ExecutionPriority,
        reason: Option<String>
    ) -> Result<Vec<ActionResult>, NanonisError> {
        let reason_str = reason.unwrap_or_else(|| "No reason provided".to_string());
        log::info!(
            "Executing sequence: {} (priority: {:?}, reason: {})", 
            sequence.name(), 
            priority, 
            reason_str
        );
        
        // Future: Handle priority-based queuing/scheduling here
        self.execute_sequence(sequence)
    }
    
    /// Direct access to underlying ActionDriver when needed
    pub fn driver(&mut self) -> &mut ActionDriver {
        &mut self.driver
    }
    
    /// Get reference to underlying ActionDriver
    pub fn driver_ref(&self) -> &ActionDriver {
        &self.driver
    }
}