# LearningPolicyEngine

**Description**: Advanced trait for policies that can learn from experience (future extension).

**Implementation**: 
```rust
pub trait LearningPolicyEngine: PolicyEngine {
    fn learn_from_outcome(
        &mut self, 
        state: &MachineState, 
        action: &ActionType, 
        outcome: f32
    );
    
    fn update_policy(&mut self);
    fn get_learning_stats(&self) -> LearningStats;
}
```

**Notes**: 
- Extends [[PolicyEngine]] with learning capabilities
- No current implementations - designed for future ML/RL policies
- Enables reinforcement learning and neural network policies