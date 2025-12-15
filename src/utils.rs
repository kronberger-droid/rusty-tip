use std::time::{Duration, Instant};

/// Error type for polling operations
#[derive(Debug)]
pub enum PollError<E> {
    /// Operation timed out
    Timeout,
    /// Error occurred in the condition/operation function
    ConditionError(E),
}

impl<E> std::fmt::Display for PollError<E>
where
    E: std::fmt::Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PollError::Timeout => write!(f, "Operation timed out"),
            PollError::ConditionError(e) => write!(f, "Condition error: {}", e),
        }
    }
}

impl<E> std::error::Error for PollError<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PollError::Timeout => None,
            PollError::ConditionError(e) => Some(e),
        }
    }
}

/// Poll a condition with timeout
///
/// Repeatedly calls `condition` until it returns `Ok(true)` or timeout is reached.
///
/// # Arguments
/// * `condition` - Function that returns `Ok(true)` when complete, `Ok(false)` to continue polling
/// * `timeout` - Maximum duration to wait
/// * `poll_interval` - Duration to sleep between condition checks
///
/// # Returns
/// * `Ok(())` when condition returns `Ok(true)`
/// * `Err(PollError::Timeout)` when timeout is reached
/// * `Err(PollError::ConditionError(e))` when condition returns an error
///
/// # Example
/// ```no_run
/// use std::time::Duration;
/// use rusty_tip::utils::poll_until;
/// use rusty_tip::NanonisClient;
///
/// let mut client = NanonisClient::new("127.0.0.1", 6501)?;
/// // Poll until auto-approach completes
/// poll_until(
///     || client.auto_approach_on_off_get().map(|running| !running),
///     Duration::from_secs(300),
///     Duration::from_millis(100),
/// )?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn poll_until<F, E>(
    mut condition: F,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(), PollError<E>>
where
    F: FnMut() -> Result<bool, E>,
{
    let start = Instant::now();

    loop {
        if start.elapsed() >= timeout {
            return Err(PollError::Timeout);
        }

        match condition() {
            Ok(true) => return Ok(()),
            Ok(false) => {
                std::thread::sleep(poll_interval);
            }
            Err(e) => return Err(PollError::ConditionError(e)),
        }
    }
}

/// Poll with timeout for operations that may return a result
///
/// Repeatedly calls `operation` until it returns `Ok(Some(T))` or timeout is reached.
///
/// # Arguments
/// * `operation` - Function that returns `Ok(Some(T))` when complete, `Ok(None)` to continue polling
/// * `timeout` - Maximum duration to wait
/// * `poll_interval` - Duration to sleep between operation calls
///
/// # Returns
/// * `Ok(Some(T))` when operation returns `Ok(Some(T))`
/// * `Ok(None)` when timeout is reached
/// * `Err(PollError::ConditionError(e))` when operation returns an error
///
/// # Example
/// ```
/// use std::time::Duration;
/// use rusty_tip::utils::poll_with_timeout;
/// use rusty_tip::utils::PollError;
///
/// // Simple example that finds data after a few attempts
/// let result = poll_with_timeout(
///     || {
///         static mut ATTEMPTS: u32 = 0;
///         unsafe {
///             ATTEMPTS += 1;
///             if ATTEMPTS >= 3 {
///                 Ok::<Option<Vec<f64>>, &str>(Some(vec![1.0, 2.0, 3.0])) // Return data on 3rd attempt
///             } else {
///                 Ok(None) // Keep trying
///             }
///         }
///     },
///     Duration::from_secs(1),
///     Duration::from_millis(10),
/// );
///
/// assert!(result.is_ok());
/// ```
pub fn poll_with_timeout<F, T, E>(
    mut operation: F,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<Option<T>, PollError<E>>
where
    F: FnMut() -> Result<Option<T>, E>,
{
    let start = Instant::now();

    loop {
        if start.elapsed() >= timeout {
            return Ok(None);
        }

        match operation() {
            Ok(Some(result)) => return Ok(Some(result)),
            Ok(None) => {
                std::thread::sleep(poll_interval);
            }
            Err(e) => return Err(PollError::ConditionError(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_poll_until_success() {
        let counter = Arc::new(Mutex::new(0));
        let counter_clone = counter.clone();

        let result = poll_until(
            || {
                let mut count = counter_clone.lock().unwrap();
                *count += 1;
                Ok::<bool, &str>(*count >= 3)
            },
            Duration::from_millis(500),
            Duration::from_millis(10),
        );

        assert!(result.is_ok());
        assert!(*counter.lock().unwrap() >= 3);
    }

    #[test]
    fn test_poll_until_timeout() {
        let result = poll_until(
            || Ok::<bool, &str>(false), // Never returns true
            Duration::from_millis(50),
            Duration::from_millis(10),
        );

        assert!(matches!(result, Err(PollError::Timeout)));
    }

    #[test]
    fn test_poll_until_error() {
        let result = poll_until(
            || Err::<bool, &str>("test error"),
            Duration::from_millis(100),
            Duration::from_millis(10),
        );

        assert!(matches!(result, Err(PollError::ConditionError("test error"))));
    }

    #[test]
    fn test_poll_with_timeout_success() {
        let counter = Arc::new(Mutex::new(0));
        let counter_clone = counter.clone();

        let result = poll_with_timeout(
            || {
                let mut count = counter_clone.lock().unwrap();
                *count += 1;

                if *count >= 3 {
                    Ok::<Option<i32>, &str>(Some(*count))
                } else {
                    Ok(None)
                }
            },
            Duration::from_millis(500),
            Duration::from_millis(10),
        );

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(3));
    }

    #[test]
    fn test_poll_with_timeout_timeout() {
        let result = poll_with_timeout(
            || Ok::<Option<()>, &str>(None), // Never returns Some
            Duration::from_millis(50),
            Duration::from_millis(10),
        );

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn test_poll_with_timeout_error() {
        let result = poll_with_timeout(
            || Err::<Option<()>, &str>("test error"),
            Duration::from_millis(100),
            Duration::from_millis(10),
        );

        assert!(matches!(result, Err(PollError::ConditionError("test error"))));
    }
}