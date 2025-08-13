**Description**: Core trait for classifying raw signal data into interpreted tip states.

**Implementation**: 
```rust
pub trait StateClassifier: Send + Sync {
    fn classify(
        &mut self, 
        machine_state: &mut MachineState
    );
    
    fn initialize_buffer(
        &mut self, 
        machine_state: &MachineState, 
        target_samples: usize
    );
    
    fn clear_buffer(&mut self);
    
    \\ ...
}
```

**Notes**: 
- Updates [[MachineState]] classification in-place
- Used by [[BoundaryClassifier]] for boundary detection
- Part of: Raw Signals → StateClassifier → [[PolicyEngine]] pipeline