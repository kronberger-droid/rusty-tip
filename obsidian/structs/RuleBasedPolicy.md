# RuleBasedPolicy

**Description**: Simple rule-based policy mapping TipState to PolicyDecision.

**Implementation**: 
```rust
pub struct RuleBasedPolicy {
    name: String,
    // Simple 1:1 mapping: TipState -> PolicyDecision
}
```

**Notes**: 
- Primary implementation of [[PolicyEngine]] trait
- Direct 1:1 mapping from [[TipState]] to [[PolicyDecision]]
- Used by [[Controller]] for simple rule-based decisions