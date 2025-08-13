# NanonisError

**Description**: Comprehensive error types for all system operations.

**Implementation**: 
```rust
#[derive(Error, Debug)]
pub enum NanonisError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Protocol error: {0}")]
    ProtocolError(String),
    #[error("Not connected to Nanonis")]
    NotConnected,
    #[error("Connection timeout")]
    Timeout,
    // ... more error types
}
```

**Notes**: 
- Used by [[NanonisClient]] and [[Controller]] for error handling
- Includes automatic conversion from std::io::Error
- Enables graceful error recovery and reconnection logic