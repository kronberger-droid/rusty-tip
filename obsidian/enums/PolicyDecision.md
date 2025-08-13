# PolicyDecision

**Description**: Policy engine output representing recommended actions.

**Implementation**: 
```rust
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyDecision {
    Good,   // Continue normal operation
    Bad,    // Execute safety/withdrawal actions
    Stable, // Execute stable operation actions
}
```

**Notes**: 
- Output of [[PolicyEngine]], consumed by [[Controller]]
- Maps to control actions: Bad→withdrawal, Good→monitoring, Stable→optimization
- Future extensions planned for complex multi-step actions