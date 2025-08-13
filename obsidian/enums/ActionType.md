**Description**: Specific action types for advanced policy engines (future extension).

**Implementation**: 
```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ActionType {
    ContinueMonitoring,
    WithdrawTip,
    AdjustBias(f32),
    MoveToPosition(f64, f64),
    StartApproach,
    EmergencyStop,
    // ... more specific actions
}
```

**Notes**: 
- Future extension for detailed action specification
- Planned for use with [[LearningPolicyEngine]] experience collection
- Will enable complex multi-step control sequences