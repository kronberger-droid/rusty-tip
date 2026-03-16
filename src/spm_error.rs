use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpmError {
    /// Network/IO failure
    #[error("IO error: {context}")]
    Io {
        #[source]
        source: std::io::Error,
        context: String,
    },
    /// Operation timed out
    #[error("Timeout: {0}")]
    Timeout(String),
    /// Protocol-level errors (malformed responses, type mismatches)
    #[error("Protocol error: {0}")]
    Protocol(String),
    /// Hardware/server reported error
    #[error("Hardware error (code {code}: {message})")]
    Hardware { code: i32, message: String },
    /// Workflow or execution logic error
    #[error("{0}")]
    Workflow(String),
    /// Operation not supported by the current controller
    #[error("Unsupported: {0}")]
    Unsupported(String),
    /// Clean shutdown requested by user (e.g. Ctrl+C)
    #[error("Shutdown requested by user")]
    ShutdownRequested,
}

impl From<nanonis_rs::NanonisError> for SpmError {
    fn from(value: nanonis_rs::NanonisError) -> Self {
        match value {
            nanonis_rs::NanonisError::Io { source, context } => {
                Self::Io { source, context }
            }
            nanonis_rs::NanonisError::Timeout(s) => Self::Timeout(s),
            nanonis_rs::NanonisError::Protocol(s) => Self::Protocol(s),
            nanonis_rs::NanonisError::Server { code, message } => {
                Self::Hardware { code, message }
            }
        }
    }
}
