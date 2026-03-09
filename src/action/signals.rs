use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::Capability;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadSignal {
    pub index: u32,
    #[serde(default = "default_true")]
    pub wait_for_newest: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ReadSignal {
    fn default() -> Self {
        Self {
            index: 0,
            wait_for_newest: true,
        }
    }
}

impl Action for ReadSignal {
    fn name(&self) -> &str {
        "read_signal"
    }
    fn description(&self) -> &str {
        "Read a single signal value by index"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let val = ctx.controller.read_signal(self.index, self.wait_for_newest)?;
        Ok(ActionOutput::Value(val))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadSignals {
    pub indices: Vec<u32>,
    #[serde(default = "default_true")]
    pub wait_for_newest: bool,
}

impl Default for ReadSignals {
    fn default() -> Self {
        Self {
            indices: vec![],
            wait_for_newest: true,
        }
    }
}

impl Action for ReadSignals {
    fn name(&self) -> &str {
        "read_signals"
    }
    fn description(&self) -> &str {
        "Read multiple signal values by index"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let vals = ctx.controller.read_signals(&self.indices, self.wait_for_newest)?;
        let labeled: Vec<(String, f64)> = self
            .indices
            .iter()
            .zip(vals)
            .map(|(idx, val)| (format!("signal_{}", idx), val))
            .collect();
        Ok(ActionOutput::Values(labeled))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadSignalNames;

impl Action for ReadSignalNames {
    fn name(&self) -> &str {
        "read_signal_names"
    }
    fn description(&self) -> &str {
        "Read all available signal names from the controller"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let names = ctx.controller.signal_names()?;
        Ok(ActionOutput::Data(serde_json::to_value(names).unwrap()))
    }
}
