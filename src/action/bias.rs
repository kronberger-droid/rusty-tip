use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::{Capability, ZControllerStatus};

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
            absolute: true,
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

/// Set bias voltage, withdrawing first if the polarity would cross zero
/// while the tip is approached.
///
/// If the z-controller is active (tip on surface) and the new voltage has
/// a different sign than the current bias, the sequence becomes:
/// 1. Withdraw
/// 2. Set bias
///
/// Otherwise behaves identically to `SetBias`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeSetBias {
    pub voltage: f64,
}

impl Default for SafeSetBias {
    fn default() -> Self {
        Self { voltage: 0.0 }
    }
}

impl Action for SafeSetBias {
    fn name(&self) -> &str {
        "safe_set_bias"
    }
    fn description(&self) -> &str {
        "Set bias voltage, withdrawing first if polarity crosses zero while approached"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Bias, Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let current_bias = ctx.controller.get_bias()?;
        let crosses_zero = current_bias.signum() != self.voltage.signum()
            && !(current_bias == 0.0 && self.voltage == 0.0);

        if crosses_zero {
            let is_approached = ctx
                .controller
                .z_controller_status()
                .map(|s| matches!(s, ZControllerStatus::On))
                .unwrap_or(false);

            if is_approached {
                log::info!(
                    "Bias change {:.3}V -> {:.3}V crosses zero while approached — withdrawing first",
                    current_bias, self.voltage
                );
                ctx.controller
                    .withdraw(true, Duration::from_secs(5))?;
            }
        }

        ctx.controller.set_bias(self.voltage)?;
        Ok(ActionOutput::Unit)
    }
}
