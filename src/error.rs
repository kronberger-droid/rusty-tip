use thiserror::Error;

#[derive(Error, Debug)]
pub enum NanonisError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Connection timeout")]
    Timeout,
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("Type error: {0}")]
    Type(String),
    #[error("Command mismatch: expected {expected}, got {actual}")]
    CommandMismatch { expected: String, actual: String },
    #[error("Invalid command: {0}")]
    InvalidCommand(String),
    #[error("Server error: {0}")]
    ServerError(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
}
