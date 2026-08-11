use std::time::Duration;

use serde::Serialize;

use crate::action::{Action, ActionContext};

#[derive(Debug, Clone, Serialize)]
pub struct Wait {
    pub duration_ms: u64,
}

impl Default for Wait {
    fn default() -> Self {
        Self { duration_ms: 1000 }
    }
}

impl Action for Wait {
    type Output = ();

    fn name(&self) -> &str {
        "wait"
    }
    fn description(&self) -> &str {
        "Wait for a specified duration in milliseconds"
    }
    fn execute(&self, _ctx: &mut ActionContext) -> super::Result<Self::Output> {
        std::thread::sleep(Duration::from_millis(self.duration_ms));
        Ok(())
    }
}
