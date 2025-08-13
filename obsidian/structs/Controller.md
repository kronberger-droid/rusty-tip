**Description**: High-level orchestration integrating all system components.

**Implementation**: 
```rust
pub struct Controller {
    client: NanonisClient,
    classifier: Box<dyn StateClassifier>,
    policy: Box<dyn PolicyEngine>,
    
    shared_state: Option<Arc<Mutex<MachineState>>>,
    
    approach_count: u32,
    position_history: VecDeque<Position>,
    action_history: VecDeque<String>,
}
```

**Notes**: 
- Orchestrates fresh sampling → [[StateClassifier]] → [[PolicyEngine]] → actions
- Integrates [[NanonisClient]], [[BoundaryClassifier]], and [[RuleBasedPolicy]]
- Handles error recovery and maintains system statistics