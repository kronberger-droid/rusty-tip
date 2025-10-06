**Description**: Background TCP stream reader that continuously buffers timestamped data frames from the Nanonis TCP logger.

**Implementation**: 
```rust
pub struct BufferedTCPReader {
    buffer: Arc<Mutex<VecDeque<TimestampedFrame>>>,
    reader_thread: Option<JoinHandle<Result<(), NanonisError>>>,
    max_buffer_size: usize,
    start_time: Instant,
    shutdown_signal: Arc<AtomicBool>,
}
```

**Notes**: 
- **Lightweight time-series database** - Provides simple time-range queries on buffered data
- **Continuous reading** - Background thread prevents data loss during action execution
- **Circular buffer** - Automatic memory management with configurable size limits
- **Thread-safe access** - Multiple queries can run while background thread continues reading
- **No data loss** - Unlike polling approaches, continuous reading captures all TCP frames

**Key Methods**:
- `new()` - Creates reader with background thread for given TCP stream
- `get_data_between()` - Query data for specific time window
- `get_recent_data()` - Get last N seconds of data
- `buffer_stats()` - Monitor buffer health (count, capacity, time span)
- `stop()` - Graceful shutdown with error propagation

**Architecture Benefits**:
- **O(1) writes** - VecDeque provides efficient push/pop operations
- **O(n) queries** - Filtering only happens on query, not during collection
- **Minimal lock contention** - Background thread only writes, queries only read

**Relationships**:
- Contains [[TimestampedFrame]] objects in circular buffer
- Used by [[ActionDriver]] for synchronized data collection
- Reads from TCPLoggerStream in background thread
- Enables [[ExperimentData]] generation with precise time windows