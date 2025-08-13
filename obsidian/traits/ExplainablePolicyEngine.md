# ExplainablePolicyEngine

**Description**: Advanced trait for policies that can explain their decisions (future extension).

**Implementation**: 
```rust
pub trait ExplainablePolicyEngine: PolicyEngine {
    fn explain_decision(
        &self, 
        machine_state: &MachineState
    ) -> String;
    
    fn get_decision_factors(&self) -> Vec<String>;
    
    fn get_feature_importance(
        &self, 
        machine_state: &MachineState
    ) -> Vec<(String, f32)>;
}
```

**Notes**: 
- Extends [[PolicyEngine]] with interpretability features
- No current implementations - designed for explainable AI
- Enables human-readable decision explanations and debugging