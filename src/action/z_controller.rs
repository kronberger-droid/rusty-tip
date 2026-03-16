use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::Capability;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Withdraw {
    #[serde(default = "super::default_true")]
    pub wait: bool,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    10_000
}

impl Default for Withdraw {
    fn default() -> Self {
        Self {
            wait: true,
            timeout_ms: 10_000,
        }
    }
}

impl Action for Withdraw {
    fn name(&self) -> &str {
        "withdraw"
    }
    fn description(&self) -> &str {
        "Withdraw the tip from the surface"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller
            .withdraw(self.wait, Duration::from_millis(self.timeout_ms))?;
        Ok(ActionOutput::Unit)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoApproach {
    #[serde(default = "super::default_true")]
    pub wait: bool,
    #[serde(default = "default_approach_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_approach_timeout_ms() -> u64 {
    300_000 // 5 minutes
}

impl Default for AutoApproach {
    fn default() -> Self {
        Self {
            wait: true,
            timeout_ms: 300_000,
        }
    }
}

impl Action for AutoApproach {
    fn name(&self) -> &str {
        "auto_approach"
    }
    fn description(&self) -> &str {
        "Auto-approach the tip to the surface. Blocks until contact or timeout."
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller
            .auto_approach(self.wait, Duration::from_millis(self.timeout_ms))?;
        Ok(ActionOutput::Unit)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetZSetpoint {
    pub setpoint: f64,
}

impl Default for SetZSetpoint {
    fn default() -> Self {
        Self { setpoint: 0.0 }
    }
}

impl Action for SetZSetpoint {
    fn name(&self) -> &str {
        "set_z_setpoint"
    }
    fn description(&self) -> &str {
        "Set the Z-controller setpoint value"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller.set_z_setpoint(self.setpoint)?;
        Ok(ActionOutput::Unit)
    }
}
