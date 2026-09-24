use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wait {
    pub duration_ms: u64,
}

impl Default for Wait {
    fn default() -> Self {
        Self { duration_ms: 1000 }
    }
}

impl Action for Wait {
    fn name(&self) -> &str {
        "wait"
    }
    fn description(&self) -> &str {
        "Wait for a specified duration in milliseconds"
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.settle(self.duration_ms)?;
        Ok(ActionOutput::Unit)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::action::DataStore;
    use crate::event::EventBus;
    use crate::mock_controller::MockController;
    use crate::shutdown::ShutdownFlag;
    use crate::spm_error::SpmError;

    /// A long wait must not outlive a stop request. Before `ActionContext`
    /// carried the flag this slept the full duration, so a Ctrl+C during a
    /// settle was ignored until the settle ended.
    #[test]
    fn a_stop_request_cuts_a_wait_short() {
        let mut controller = MockController::builder().build();
        let mut store = DataStore::new();
        let events = EventBus::new();
        let shutdown = ShutdownFlag::new();
        shutdown.request();

        let mut ctx = ActionContext {
            controller: &mut controller,
            store: &mut store,
            events: &events,
            shutdown: &shutdown,
            depth: 0,
        };

        let start = Instant::now();
        let result = Wait {
            duration_ms: 60_000,
        }
        .execute(&mut ctx);

        assert!(matches!(result, Err(SpmError::ShutdownRequested)));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the wait should end on the stop request, not run its full minute"
        );
    }
}
