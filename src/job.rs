use crate::error::NanonisError;
use std::time::Duration;

/// A trait for long-running processes that can succeed, fail, or timeout
///
/// Jobs represent autonomous processes like tip controllers, scanning operations,
/// monitoring loops, or any other task that runs for a duration and produces a result.
pub trait Job {
    /// The type returned on successful completion
    type Output;

    /// Run the job with a timeout
    ///
    /// Returns:
    /// - `Ok(output)` if the job completes successfully
    /// - `Err(NanonisError)` if the job fails or times out
    fn run(&mut self, timeout: Duration) -> Result<Self::Output, NanonisError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestJob {
        should_succeed: bool,
    }

    impl Job for TestJob {
        type Output = String;

        fn run(&mut self, _timeout: Duration) -> Result<Self::Output, NanonisError> {
            if self.should_succeed {
                Ok("success".to_string())
            } else {
                Err(NanonisError::Timeout)
            }
        }
    }

    #[test]
    fn test_job_success() {
        let mut job = TestJob {
            should_succeed: true,
        };
        let result = job.run(Duration::from_secs(1)).unwrap();
        assert_eq!(result, "success");
    }

    #[test]
    fn test_job_failure() {
        let mut job = TestJob {
            should_succeed: false,
        };
        let result = job.run(Duration::from_secs(1));
        assert!(result.is_err());
    }
}
