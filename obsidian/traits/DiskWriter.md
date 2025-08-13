**Description**: Interface for persisting signal data to disk in various formats.

**Implementation**: 
```rust
pub trait DiskWriter: Send + Sync {
    fn write_state(
        &mut self, 
        state: &MachineState, 
        metadata: &SessionMetadata
    ) -> Result<(), Box<dyn std::error::Error>>;
    
    fn flush(&mut self) 
        -> Result<(), Box<dyn std::error::Error>>;
    
    fn close(&mut self) 
        -> Result<(), Box<dyn std::error::Error>>;
    
    fn get_file_path(&self) -> &std::path::Path;
}
```

**Notes**: 
- Abstracts data persistence for different file formats (JSON, CSV, etc.)
- Implemented by JsonDiskWriter for JSON output
- Used by signal monitoring system for data collection