use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_not_requested() {
        let flag = ShutdownFlag::new();
        assert!(!flag.is_requested());
    }

    #[test]
    fn request_sets_flag() {
        let flag = ShutdownFlag::new();
        flag.request();
        assert!(flag.is_requested());
    }

    #[test]
    fn reset_clears_flag() {
        let flag = ShutdownFlag::new();
        flag.request();
        flag.reset();
        assert!(!flag.is_requested());
    }

    #[test]
    fn clone_shares_state() {
        let flag = ShutdownFlag::new();
        let flag2 = flag.clone();
        flag.request();
        assert!(flag2.is_requested());
    }

    #[test]
    fn from_arc_shares_state() {
        let raw = Arc::new(AtomicBool::new(false));
        let flag = ShutdownFlag::from_arc(raw.clone());
        raw.store(true, Ordering::SeqCst);
        assert!(flag.is_requested());
    }

    #[test]
    fn arc_returns_shared_reference() {
        let flag = ShutdownFlag::new();
        let arc = flag.arc();
        flag.request();
        assert!(arc.load(Ordering::SeqCst));
    }

    #[test]
    fn thread_safety() {
        let flag = ShutdownFlag::new();
        let flag2 = flag.clone();
        let handle = std::thread::spawn(move || {
            flag2.request();
        });
        handle.join().unwrap();
        assert!(flag.is_requested());
    }
}
