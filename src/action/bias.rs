use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::Capability;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadBias;

impl Action for ReadBias {
    fn name(&self) -> &str {
        "read_bias"
    }
    fn description(&self) -> &str {
        "Read the current bias voltage in volts"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Bias]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let voltage = ctx.controller.get_bias()?;
        Ok(ActionOutput::Value(voltage))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetBias {
    pub voltage: f64,
}

impl Default for SetBias {
    fn default() -> Self {
        Self { voltage: 1.0 }
    }
}

impl Action for SetBias {
    fn name(&self) -> &str {
        "set_bias"
    }
    fn description(&self) -> &str {
        "Set the bias voltage in volts"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Bias]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller.set_bias(self.voltage)?;
        Ok(ActionOutput::Unit)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiasPulse {
    pub voltage: f64,
    pub duration_ms: u64,
    #[serde(default = "super::default_true")]
    pub z_hold: bool,
    #[serde(default)]
    pub absolute: bool,
}


impl Default for BiasPulse {
    fn default() -> Self {
        Self {
            voltage: 0.0,
            duration_ms: 100,
            z_hold: true,
            absolute: false,
        }
    }
}

impl Action for BiasPulse {
    fn name(&self) -> &str {
        "bias_pulse"
    }
    fn description(&self) -> &str {
        "Apply a voltage pulse to the bias. Used for tip conditioning."
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Bias]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller.bias_pulse(
            self.voltage,
            Duration::from_millis(self.duration_ms),
            self.z_hold,
            self.absolute,
        )?;
        Ok(ActionOutput::Unit)
    }
}
