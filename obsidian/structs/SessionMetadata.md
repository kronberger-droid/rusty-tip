# SessionMetadata

**Description**: Session metadata containing static information for monitoring sessions.

**Implementation**: 
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub session_id: String,
    pub signal_names: Vec<String>,
    pub active_indices: Vec<usize>,
    pub primary_signal_index: usize,
    pub session_start: f64,
}
```

**Notes**: 
- Static information written once per monitoring session
- Used by [[DiskWriter]] for data persistence context
- Contains signal mapping and session identification