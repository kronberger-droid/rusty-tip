use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Thread-safe flag for graceful workflow cancellation.
///
/// Share an `Arc<AtomicBool>` between the executor and the signal handler
/// (or GUI stop button). The executor checks this between steps.
#[derive(Debug, Clone)]
pub struct ShutdownFlag {
    flag: Arc<AtomicBool>,
}

impl ShutdownFlag {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Wrap an existing flag (e.g. from a signal handler).
    pub fn from_arc(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }

    /// Request shutdown.
    pub fn request(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Check if shutdown has been requested.
    pub fn is_requested(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Reset the flag (e.g. for reuse across multiple workflow runs).
    pub fn reset(&self) {
        self.flag.store(false, Ordering::SeqCst);
    }

    /// Get a clone of the underlying Arc for sharing with signal handlers.
    pub fn arc(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }
}

impl Default for ShutdownFlag {
    fn default() -> Self {
        Self::new()
    }
}
