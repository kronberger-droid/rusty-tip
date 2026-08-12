use std::time::Duration;

use serde::Serialize;

use crate::action::pll::CenterFreqShift;
use crate::action::util::Wait;
use crate::action::{Action, ActionContext};
use crate::spm_controller::{Capability, ZControllerStatus};
use crate::spm_error::SpmError;

#[derive(Debug, Clone, Serialize)]
pub struct Withdraw {
    pub wait: bool,
    pub timeout_ms: u64,
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
    type Output = ();

    fn name(&self) -> &str {
        "withdraw"
    }
    fn description(&self) -> &str {
        "Withdraw the tip from the surface"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller
            .withdraw(self.wait, Duration::from_millis(self.timeout_ms))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AutoApproach {
    pub wait: bool,
    pub timeout_ms: u64,
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
    type Output = ();

    fn name(&self) -> &str {
        "auto_approach"
    }
    fn description(&self) -> &str {
        "Auto-approach the tip to the surface. Blocks until contact or timeout."
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller
            .auto_approach(self.wait, Duration::from_millis(self.timeout_ms))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SetZSetpoint {
    pub setpoint: f64,
}

impl Default for SetZSetpoint {
    fn default() -> Self {
        Self { setpoint: 0.0 }
    }
}

impl Action for SetZSetpoint {
    type Output = ();

    fn name(&self) -> &str {
        "set_z_setpoint"
    }
    fn description(&self) -> &str {
        "Set the Z-controller setpoint value"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.set_z_setpoint(self.setpoint)?;
        Ok(())
    }
}

/// Move the tip to the configured Z-home position (small withdraw from surface).
#[derive(Debug, Clone, Default, Serialize)]
pub struct ZHome;

impl Action for ZHome {
    type Output = ();

    fn name(&self) -> &str {
        "z_home"
    }
    fn description(&self) -> &str {
        "Move tip to configured Z-home position (small withdraw from surface)"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.go_z_home()?;
        Ok(())
    }
}

/// Query the current Z-controller status.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReadZControllerStatus;

impl Action for ReadZControllerStatus {
    type Output = ZControllerStatus;

    fn name(&self) -> &str {
        "read_z_controller_status"
    }
    fn description(&self) -> &str {
        "Read the current Z-controller status (on/off)"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.z_controller_status()
    }
}

/// Query whether safe-tip crash protection is currently enabled.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReadSafeTipStatus;

impl Action for ReadSafeTipStatus {
    type Output = bool;

    fn name(&self) -> &str {
        "read_safe_tip_status"
    }
    fn description(&self) -> &str {
        "Read whether the safe-tip crash protection is currently enabled"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::SafeTip]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.safe_tip_enabled()
    }
}

/// Enable or disable safe-tip crash protection.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SafeTipSet {
    pub enabled: bool,
}

impl Action for SafeTipSet {
    type Output = ();

    fn name(&self) -> &str {
        "safe_tip_set"
    }
    fn description(&self) -> &str {
        "Enable or disable the safe-tip crash protection"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::SafeTip]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.safe_tip_set_enabled(self.enabled)?;
        Ok(())
    }
}

/// Composite action: approach and calibrate frequency shift for a valid reading.
///
/// Sequence:
/// 1. Auto-approach to surface
/// 2. Wait 200ms
/// 3. Enable safe-tip protection
/// 4. Z-home (small *relative* withdraw, ~50nm off the surface)
/// 5. Wait 500ms
/// 6. Center frequency shift (while slightly withdrawn)
/// 7. Auto-approach again (final approach with calibrated freq shift)
/// 8. Restore safe-tip to previous state
#[derive(Debug, Clone, Serialize)]
pub struct CalibratedApproach {
    pub wait: bool,
    pub timeout_ms: u64,
    /// Abort if safe-tip protection trips during the sequence. Off unless the
    /// routine asks for it.
    ///
    /// This is the one place the check earns its keep, and it is why safe-tip
    /// is switched on here at all. Steps 4 to 7 run *out of feedback*: the tip
    /// is parked a little way off the surface so that centring the frequency
    /// shift is not corrupted by the sensor drifting with every movement.
    /// Nothing is holding the tip off the surface during that window, so it
    /// can drift into it. Safe-tip catches exactly that, and these checks turn
    /// a catch into an abort instead of an approach that carries on over a
    /// crashed tip. Back in feedback the protection is not needed, which is
    /// why step 8 restores it.
    pub check_safe_tip: bool,
}

impl Default for CalibratedApproach {
    fn default() -> Self {
        Self {
            wait: true,
            timeout_ms: 300_000,
            check_safe_tip: false,
        }
    }
}

/// Abort if the controller reports safe-tip protection has tripped.
///
/// A controller without the capability passes, and a failed status read is
/// logged rather than treated as a trip, so a flaky read cannot abort an
/// otherwise healthy approach.
fn abort_if_safe_tip(ctx: &mut ActionContext, stage: &str) -> super::Result<()> {
    if !ctx.controller.capabilities().contains(&Capability::SafeTip) {
        return Ok(());
    }
    match ctx.controller.z_controller_status() {
        Ok(ZControllerStatus::SafeTip) => Err(SpmError::Routine(format!(
            "safe-tip protection triggered ({stage}), aborting approach"
        ))),
        Ok(_) => Ok(()),
        Err(e) => {
            log::warn!("Could not read z-controller status ({stage}): {e}");
            Ok(())
        }
    }
}

impl Action for CalibratedApproach {
    type Output = ();

    fn name(&self) -> &str {
        "calibrated_approach"
    }
    fn description(&self) -> &str {
        "Approach, small withdraw, center freq shift, re-approach for a valid reading"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController, Capability::Pll]
    }

    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
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

        // Steps 4-7 wrapped so safe-tip is always restored on exit
        let armed = self.check_safe_tip;
        let result = (|| -> super::Result<()> {
            let guard = |ctx: &mut ActionContext, stage: &str| -> super::Result<()> {
                if armed {
                    abort_if_safe_tip(ctx, stage)
                } else {
                    Ok(())
                }
            };

            guard(ctx, "after enabling safe tip")?;

            // 4. Small withdraw to z-home (~50nm above surface)
            ctx.controller.go_z_home()?;
            guard(ctx, "after z home")?;

            // 5. Settle
            Wait { duration_ms: 500 }.execute(ctx)?;
            guard(ctx, "after settle")?;

            // 6. Center freq shift (non-fatal if it fails)
            if let Err(e) = CenterFreqShift.execute(ctx) {
                log::warn!("Failed to center frequency shift: {} (continuing)", e);
            }
            guard(ctx, "after centering freq shift")?;

            // 7. Final approach with centered freq shift
            ctx.controller.auto_approach(self.wait, timeout)?;
            guard(ctx, "after final approach")?;

            Ok(())
        })();

        // 8. Always restore safe-tip state before propagating errors
        if !was_enabled && let Err(e) = ctx.controller.safe_tip_set_enabled(false) {
            log::error!("Failed to restore safe-tip state: {}", e);
        }

        result?;
        Ok(())
    }
}
