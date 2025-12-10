use thiserror::Error;

#[derive(Error, Debug)]
pub enum NanonisError {
    /// IO error with context
    #[error("IO error: {context}")]
    Io {
        #[source]
        source: std::io::Error,
        context: String,
    },

    #[error("Connection timeout")]
    Timeout,

    #[error("Operation timed out: {context}")]
    TimeoutWithContext { context: String },

    #[error("Shutdown requested")]
    Shutdown,

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Type error: {0}")]
    Type(String),

    #[error("Command mismatch: expected {expected}, got {actual}")]
    CommandMismatch { expected: String, actual: String },

    #[error("Invalid command: {0}")]
    InvalidCommand(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    /// Server explicitly returned an error
    #[error("Nanonis server error: {message} (code: {code})")]
    ServerError { code: i32, message: String },

    /// Command was rejected (convenience wrapper for common case)
    #[error("Command rejected: {0}")]
    CommandRejected(String),

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),
}

impl NanonisError {
    /// Check if this is a server-side rejection
    pub fn is_server_error(&self) -> bool {
        matches!(
            self,
            NanonisError::ServerError { .. } | NanonisError::CommandRejected(_)
        )
    }

    /// Get error code if this is a server error
    pub fn error_code(&self) -> Option<i32> {
        match self {
            NanonisError::ServerError { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// Check if this is a shutdown request
    pub fn is_shutdown(&self) -> bool {
        matches!(self, NanonisError::Shutdown)
    }
}

// Maintain backward compatibility with simple IO errors
impl From<std::io::Error> for NanonisError {
    fn from(error: std::io::Error) -> Self {
        NanonisError::Io {
            source: error,
            context: "IO operation failed".to_string(),
        }
    }
}

impl From<serde_json::Error> for NanonisError {
    fn from(error: serde_json::Error) -> Self {
        NanonisError::SerializationError(error.to_string())
    }
}
