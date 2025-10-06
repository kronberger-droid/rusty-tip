**Description**: Central orchestration system for SPM operations combining direct Nanonis control with synchronized data collection.

**Implementation**: 
```rust
pub struct ActionDriver {
    client: NanonisClient,
    stored_values: HashMap<String, ActionResult>,
    tcp_logger_config: Option<TCPLoggerConfig>,
    tcp_receiver: Option<mpsc::Receiver<TCPLoggerData>>,
}
```

**Notes**: 
- Direct 1:1 translation layer between Actions and NanonisClient calls
- Manages TCP logger integration for synchronized data collection during actions
- Storage system for Store/Retrieve actions across experiments
- Supports builder pattern for flexible configuration
- **Architecture**: ActionDriver replaces the previous Controller-based approach, providing a unified interface for both control commands and data collection

**Key Methods**:
- `execute()` - Direct action execution
- `execute_with_data_collection()` - Action execution with time-windowed data capture
- `pulse_with_data_collection()` - Convenience method for bias pulse experiments
- `start_tcp_buffering()` / `stop_tcp_buffering()` - Background data collection control

**Relationships**:
- Contains [[NanonisClient]] for hardware communication
- Uses [[TCPLoggerConfig]] for data collection setup
- Produces [[ActionResult]] from [[Action]] execution
- Integrates with [[BufferedTCPReader]] for continuous data collection