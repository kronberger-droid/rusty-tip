use std::time::Duration;

use serde::Serialize;

use crate::action::{Action, ActionContext};
use crate::spm_controller::{Capability, ZControllerStatus};

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReadBias;

impl Action for ReadBias {
    type Output = f64;

    fn name(&self) -> &str {
        "read_bias"
    }
    fn description(&self) -> &str {
        "Read the current bias voltage in volts"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Bias]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        let voltage = ctx.controller.get_bias()?;
        Ok(voltage)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SetBias {
    pub voltage: f64,
}

impl Default for SetBias {
    fn default() -> Self {
        Self { voltage: 1.0 }
    }
}

impl Action for SetBias {
    type Output = ();

    fn name(&self) -> &str {
        "set_bias"
    }
    fn description(&self) -> &str {
        "Set the bias voltage in volts"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Bias]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.set_bias(self.voltage)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BiasPulse {
    pub voltage: f64,
    pub duration_ms: u64,
    pub z_hold: bool,
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
    type Output = ();

    fn name(&self) -> &str {
        "bias_pulse"
    }
    fn description(&self) -> &str {
        "Apply a voltage pulse to the bias. Used for tip conditioning."
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Bias]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.bias_pulse(
            self.voltage,
            Duration::from_millis(self.duration_ms),
            self.z_hold,
            self.absolute,
        )?;
        Ok(())
    }

    // No effects: bias returns to previous value after pulse
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
#[derive(Debug, Clone, Serialize)]
pub struct SafeSetBias {
    pub voltage: f64,
}

impl Default for SafeSetBias {
    fn default() -> Self {
        Self { voltage: 0.0 }
    }
}

impl Action for SafeSetBias {
    type Output = ();

    fn name(&self) -> &str {
        "safe_set_bias"
    }
    fn description(&self) -> &str {
        "Set bias voltage, withdrawing first if polarity crosses zero while approached"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Bias, Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        let current_bias = ctx.controller.get_bias()?;
        let crosses_zero = current_bias.signum() != self.voltage.signum()
            && !(current_bias == 0.0 && self.voltage == 0.0);

        if crosses_zero {
            // On query failure, default to "approached" so the polarity flip
            // is preceded by a defensive withdraw rather than a bias swing
            // against an engaged tip.
            let is_approached = match ctx.controller.z_controller_status() {
                Ok(s) => matches!(s, ZControllerStatus::On),
                Err(e) => {
                    log::warn!(
                        "SafeSetBias: z_controller_status failed ({e}); defaulting to approached"
                    );
                    true
                }
            };

            if is_approached {
                log::info!(
                    "Bias change {:.3}V -> {:.3}V crosses zero while approached — withdrawing first",
                    current_bias,
                    self.voltage
                );
                ctx.controller.withdraw(true, Duration::from_secs(5))?;
            }
        }

        ctx.controller.set_bias(self.voltage)?;
        Ok(())
    }
}
