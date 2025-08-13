# BoundaryClassifier

**Description**: Boundary-based signal classifier with buffering and stability tracking.

**Implementation**: 
```rust
pub struct BoundaryClassifier {
    name: String,
    signal_index: i32,
    min_bound: f32,
    max_bound: f32,
    drop_front: usize,
    buffer_size: usize,
    
    consecutive_good_count: u32,
    stable_threshold: u32,
    last_classification: Option<TipState>,
    own_signal_history: VecDeque<f32>,
}
```

**Notes**: 
- Primary implementation of [[StateClassifier]] trait
- Uses boundary detection (signal in/out of bounds) with configurable buffering
- Tracks consecutive good readings for [[TipState]]::Stable classification