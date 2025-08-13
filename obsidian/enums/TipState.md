**Description**: Classification result representing tip condition.

**Implementation**: 
```rust
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum TipState {
    #[default]
    Bad,     // Signal out of bounds - unsafe condition
    Good,    // Signal within bounds - normal operation  
    Stable,  // Signal good and stable for required period
}
```

**Notes**: 
- Output of [[StateClassifier]], stored in [[MachineState]]
- Used by [[PolicyEngine]] to make [[PolicyDecision]]
- State progression: Bad → Good → Stable