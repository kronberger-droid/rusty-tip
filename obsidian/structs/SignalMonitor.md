**Description**: Real-time signal monitoring component with shared state integration (should be renamed to SignalReader).

**Implementation**: 
```rust
pub struct SyncSignalMonitor {
    nanonis_address: String,
    nanonis_port: u16,
    
    signal_indices: Vec<usize>,
    sample_rate: Duration,
    buffer_size: usize,
    
    metadata_primary_signal_index: Option<i32>,
    
    is_running: Arc<AtomicBool>,
    shared_state: Option<Arc<Mutex<MachineState>>>,
    ==
    signal_sender: Option<Sender<MachineState>>,
    disk_writer: Option<Box<dyn DiskWriter>>,
}
```

**Notes**: 
- Real-time signal monitoring with configurable sampling rates
- Integrates with [[DiskWriter]] for data persistence
- Shares state with [[Controller]] via Arc<Mutex<MachineState>>
- **Suggested rename**: Should be called `SignalReader` for clarity