**Description**: Individual data frame from TCP logger with high-resolution timestamp for precise time correlation.

**Implementation**: 
```rust
pub struct TimestampedFrame {
    pub data: TCPLoggerData,
    pub timestamp: Instant,
    pub relative_time: Duration,
}
```

**Notes**: 
- **Precise timing** - Uses `Instant::now()` for high-resolution timestamps
- **Relative time** - Duration since data collection start for easy analysis
- **Self-contained** - Contains both raw data and timing information
- **Time correlation** - Enables precise before/during/after action analysis

**Key Methods**:
- `new()` - Creates timestamped frame with current time and relative duration

**Data Flow**:
1. **TCP Logger** → Raw signal data frame
2. **Background Reader** → Adds timestamp + relative time
3. **Circular Buffer** → Stores timestamped frames
4. **Query Methods** → Filter by time ranges for analysis

**Usage in Time Windows**:
- **Pre-action**: Frames before action start timestamp
- **During-action**: Frames between action start/end timestamps  
- **Post-action**: Frames after action end timestamp

**Relationships**:
- Contains [[TCPLoggerData]] with raw signal values
- Stored in [[BufferedTCPReader]] circular buffer
- Used by [[ExperimentData]] for time-windowed analysis
- Enables precise synchronization between actions and signal measurements