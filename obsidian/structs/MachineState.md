**Description**: Central data structure representing the complete state of the SPM system.

**Implementation**: 
```rust
pub struct MachineState {
    pub all_signals: Option<Vec<f32>>,
    pub position: Option<(f64, f64)>
    pub z_position: Option<f64>,
    pub timestamp: f64,
    pub signal_history: VecDeque<f32>, // !?
    pub tip_state: TipState,
    pub last_action: Option<String>,
}
```

**Notes**: 
- Central hub updated by [[StateClassifier]] and read by [[PolicyEngine]]
- Contains signal data, spatial/temporal context, and classification results
- Designed for ML expansion with future embedding fields