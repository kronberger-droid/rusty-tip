use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

/// Thread-safe flag for graceful cancellation of a running routine.
///
/// Clone the flag into the signal handler (or GUI stop button) and call
/// [`request()`](Self::request) there; the running routine checks
/// [`is_requested()`](Self::is_requested) between steps and sleeps with
/// [`wait_timeout()`](Self::wait_timeout).
///
/// Backed by a condition variable rather than a bare `AtomicBool`, so a
/// shutdown request wakes sleeping waiters immediately instead of being
/// noticed at the next poll interval.
#[derive(Debug, Clone)]
pub struct ShutdownFlag {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    requested: Mutex<bool>,
    cvar: Condvar,
}

impl ShutdownFlag {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                requested: Mutex::new(false),
                cvar: Condvar::new(),
            }),
        }
    }

    /// Request shutdown and wake every thread blocked in `wait_timeout`.
    pub fn request(&self) {
        *self.inner.requested.lock() = true;
        self.inner.cvar.notify_all();
    }

    /// Check if shutdown has been requested.
    pub fn is_requested(&self) -> bool {
        *self.inner.requested.lock()
    }

    /// Reset the flag (e.g. for reuse across multiple runs).
    pub fn reset(&self) {
        *self.inner.requested.lock() = false;
    }

    /// Block for up to `timeout`, returning early if shutdown is requested.
    ///
    /// Returns `true` if shutdown was requested (immediately if it already
    /// was), `false` if the full timeout elapsed. This is the interruptible
    /// replacement for `std::thread::sleep` in routine code.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut requested = self.inner.requested.lock();
        while !*requested {
            if self
                .inner
                .cvar
                .wait_until(&mut requested, deadline)
                .timed_out()
            {
                return *requested;
            }
        }
        true
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
    fn wait_timeout_elapses_when_not_requested() {
        let flag = ShutdownFlag::new();
        let start = Instant::now();
        assert!(!flag.wait_timeout(Duration::from_millis(20)));
        assert!(start.elapsed() >= Duration::from_millis(20));
    }

    #[test]
    fn wait_timeout_returns_immediately_when_already_requested() {
        let flag = ShutdownFlag::new();
        flag.request();
        let start = Instant::now();
        assert!(flag.wait_timeout(Duration::from_secs(10)));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn request_wakes_a_sleeping_waiter() {
        let flag = ShutdownFlag::new();
        let waiter = flag.clone();
        let handle = std::thread::spawn(move || {
            let start = Instant::now();
            let requested = waiter.wait_timeout(Duration::from_secs(10));
            (requested, start.elapsed())
        });
        std::thread::sleep(Duration::from_millis(50));
        flag.request();
        let (requested, waited) = handle.join().unwrap();
        assert!(requested);
        assert!(
            waited < Duration::from_secs(5),
            "waiter should wake on request, not sleep out the timeout (waited {waited:?})"
        );
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
