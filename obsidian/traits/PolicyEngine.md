**Description**: Core trait for making decisions based on classified machine states.

**Implementation**: 
```rust
pub trait PolicyEngine: Send + Sync {
    fn decide(
        &mut self, 
        machine_state: &MachineState
    ) -> PolicyDecision;
    
    fn get_name(&self) -> &str;
}
```

**Notes**: 
- Takes classified [[MachineState]] and produces [[PolicyDecision]]
- Implemented by [[RuleBasedPolicy]] for simple rule-based decisions
- Part of: [[StateClassifier]] → [[MachineState]] → PolicyEngine → [[Controller]] pipeline