**Description**: High-level interface for communicating with Nanonis SPM systems.

**Implementation**: 
```rust
pub struct NanonisClient {
    stream: TcpStream,
    debug: bool,
    config: ConnectionConfig,
}
```

**Notes**: 
- Type-safe TCP interface to Nanonis with automatic reconnection
- Used by [[Controller]] for all hardware operations
- Supports signals, bias control, positioning, and automation commands