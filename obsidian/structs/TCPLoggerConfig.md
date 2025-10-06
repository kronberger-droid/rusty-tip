**Description**: Configuration structure for TCP Logger integration with ActionDriver.

**Implementation**: 
```rust
pub struct TCPLoggerConfig {
    pub stream_port: u16,
    pub channels: Vec<i32>,
    pub oversampling: i32,
    pub auto_start: bool,
}
```

**Notes**: 
- **Stream configuration** - Defines which signals to collect and at what rate
- **Channel selection** - Specify signal indices (0-127) for data collection
- **Sampling control** - Oversampling rate for higher temporal resolution
- **Automatic startup** - Optional auto-start of logger during ActionDriver initialization

**Configuration Options**:
- **stream_port** - TCP port for data stream (typically 6590)
- **channels** - Signal indices to record (e.g., [0, 8] for bias + current)
- **oversampling** - Multiplier for base sampling rate (0-1000)
- **auto_start** - Whether to start logging immediately on connection

**Common Channel Configurations**:
- **Bias spectroscopy**: `[0, 8]` - Bias voltage + current
- **Approach curves**: `[8, 24]` - Current + Z-position  
- **Full monitoring**: `[0, 8, 16, 24]` - Bias, current, frequency, Z-pos
- **Custom signals**: Any combination of available signal indices

**Integration Flow**:
1. **ActionDriver creation** with TCPLoggerConfig
2. **TCP stream setup** on specified port
3. **Channel configuration** via NanonisClient
4. **Background reader** starts continuous data collection
5. **Automatic buffering** enables time-windowed queries

**Relationships**:
- Used by [[ActionDriver]] during initialization
- Configures [[BufferedTCPReader]] data collection parameters
- Works with [[NanonisClient]] for TCP logger control commands
- Enables [[ExperimentData]] generation with specified signals