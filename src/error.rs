use std::fmt;
use std::time::Duration;

use crate::NanonisError;

/// Outcome of a successful tip controller run
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    /// Tip preparation completed (tip reached stable state)
    Completed,
    /// User requested shutdown (Ctrl+C or GUI stop)
    StoppedByUser,
}

/// Error type for the tip controller
#[derive(Debug)]
pub enum Error {
    /// Underlying Nanonis communication error
    Nanonis(NanonisError),
    /// Graceful shutdown was requested (internal, caught by run())
    Shutdown,
    /// Maximum cycle count exceeded
    CycleLimit(u32),
    /// Maximum duration exceeded
    TimedOut(Duration),
    /// Configuration validation error
    Config(String),
}

impl From<NanonisError> for Error {
    fn from(e: NanonisError) -> Self {
        Error::Nanonis(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Nanonis(e) => write!(f, "Nanonis error: {}", e),
            Error::Shutdown => write!(f, "Shutdown requested"),
            Error::CycleLimit(n) => {
                write!(f, "Maximum cycle count ({}) exceeded", n)
            }
            Error::TimedOut(d) => {
                write!(f, "Maximum duration ({:.0}s) exceeded", d.as_secs_f64())
            }
            Error::Config(msg) => write!(f, "Configuration error: {}", msg),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Nanonis(e) => Some(e),
            _ => None,
        }
    }
}
