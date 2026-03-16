use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::action::util::Wait;
use crate::action::pll::CenterFreqShift;
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

/// Move the tip to the configured Z-home position (small withdraw from surface).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ZHome;

impl Action for ZHome {
    fn name(&self) -> &str {
        "z_home"
    }
    fn description(&self) -> &str {
        "Move tip to configured Z-home position (small withdraw from surface)"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller.go_z_home()?;
        Ok(ActionOutput::Unit)
    }
}

/// Enable or disable safe-tip crash protection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeTipSet {
    pub enabled: bool,
}

impl Action for SafeTipSet {
    fn name(&self) -> &str {
        "safe_tip_set"
    }
    fn description(&self) -> &str {
        "Enable or disable the safe-tip crash protection"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::SafeTip]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller.safe_tip_set_enabled(self.enabled)?;
        Ok(ActionOutput::Unit)
    }
}

/// Composite action: approach and calibrate frequency shift for a valid reading.
///
/// Sequence:
/// 1. Auto-approach to surface
/// 2. Wait 200ms
/// 3. Enable safe-tip protection
/// 4. Z-home (small withdraw ~50nm from surface)
/// 5. Wait 500ms
/// 6. Center frequency shift (while slightly withdrawn)
/// 7. Auto-approach again (final approach with calibrated freq shift)
/// 8. Restore safe-tip to previous state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibratedApproach {
    #[serde(default = "super::default_true")]
    pub wait: bool,
    #[serde(default = "default_approach_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for CalibratedApproach {
    fn default() -> Self {
        Self {
            wait: true,
            timeout_ms: 300_000,
        }
    }
}

impl Action for CalibratedApproach {
    fn name(&self) -> &str {
        "calibrated_approach"
    }
    fn description(&self) -> &str {
        "Approach, small withdraw, center freq shift, re-approach for a valid reading"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController, Capability::Pll]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let timeout = Duration::from_millis(self.timeout_ms);

        // 1. Initial approach
        ctx.controller.auto_approach(self.wait, timeout)?;

        // 2. Settle
        Wait { duration_ms: 200 }.execute(ctx)?;

        // 3. Enable safe-tip
        let was_enabled = ctx.controller.safe_tip_enabled().unwrap_or(false);
        if !was_enabled {
            ctx.controller.safe_tip_set_enabled(true)?;
        }

        // 4. Small withdraw to z-home (~50nm above surface)
        ctx.controller.go_z_home()?;

        // 5. Settle
        Wait { duration_ms: 500 }.execute(ctx)?;

        // 6. Center freq shift (non-fatal if it fails)
        if let Err(e) = CenterFreqShift.execute(ctx) {
            log::warn!("Failed to center frequency shift: {} (continuing)", e);
        }

        // 7. Final approach with centered freq shift
        ctx.controller.auto_approach(self.wait, timeout)?;

        // 8. Restore safe-tip state
        if !was_enabled {
            ctx.controller.safe_tip_set_enabled(false)?;
        }

        Ok(ActionOutput::Unit)
    }
}
