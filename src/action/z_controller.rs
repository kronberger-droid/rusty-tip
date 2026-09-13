use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::action::pll::CenterFreqShift;
use crate::action::util::Wait;
use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::{Capability, ZControllerStatus};
use crate::spm_error::SpmError;

/// Fail if the Z controller reports that safe-tip protection has fired.
///
/// A status read that fails is logged and treated as "not tripped", as 0.2.3
/// did: the check is a guard on top of the hardware's own retract, not the
/// thing that keeps the tip safe.
fn abort_if_safe_tip_tripped(ctx: &mut ActionContext, when: &str) -> super::Result<()> {
    match ctx.controller.z_controller_status() {
        Ok(ZControllerStatus::SafeTip) => Err(SpmError::Workflow(format!(
            "safe-tip protection tripped {when}; aborting rather than approaching again"
        ))),
        Ok(_) => Ok(()),
        Err(e) => {
            log::warn!("Could not read the Z-controller status {when}: {e}");
            Ok(())
        }
    }
}

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

/// Query the current Z-controller status (on/off).
///
/// Resolves `StateField::ZController` so the framework can auto-insert this
/// action when a downstream step requires the field to be Known.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadZControllerStatus;

impl Action for ReadZControllerStatus {
    fn name(&self) -> &str {
        "read_z_controller_status"
    }
    fn description(&self) -> &str {
        "Read the current Z-controller status (on/off)"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let status = ctx.controller.z_controller_status()?;
        let on = matches!(status, ZControllerStatus::On);
        Ok(ActionOutput::Data(serde_json::json!({
            "on": on,
            "status": format!("{:?}", status),
        })))
    }
}

/// Query whether safe-tip crash protection is currently enabled.
///
/// Resolves `StateField::SafeTipEnabled`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadSafeTipStatus;

impl Action for ReadSafeTipStatus {
    fn name(&self) -> &str {
        "read_safe_tip_status"
    }
    fn description(&self) -> &str {
        "Read whether the safe-tip crash protection is currently enabled"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::SafeTip]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let enabled = ctx.controller.safe_tip_enabled()?;
        Ok(ActionOutput::Data(
            serde_json::json!({ "enabled": enabled }),
        ))
    }
}

/// Enable or disable safe-tip crash protection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
///
/// Between steps 3 and 7 the Z-controller status is checked, and the action
/// aborts if safe-tip protection has fired. Safe-tip retracts the tip on its
/// own, so the trip itself is handled; what must not happen is step 7 driving
/// the tip straight back at whatever caused it.
///
/// Step 4 relies on the controller's Z-home mode being *relative*: it has to
/// mean "back off from here", not "go to a coordinate". `NanonisSetupConfig`
/// defaults to relative for that reason.
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

        // Steps 4-7 wrapped so safe-tip is always restored on exit
        let result = (|| -> super::Result<()> {
            abort_if_safe_tip_tripped(ctx, "after enabling safe-tip")?;

            // 4. Small withdraw to z-home (~50nm above surface)
            ctx.controller.go_z_home()?;
            abort_if_safe_tip_tripped(ctx, "after z-home")?;

            // 5. Settle
            Wait { duration_ms: 500 }.execute(ctx)?;
            abort_if_safe_tip_tripped(ctx, "after the post-home settle")?;

            // 6. Center freq shift (non-fatal if it fails)
            if let Err(e) = CenterFreqShift.execute(ctx) {
                log::warn!("Failed to center frequency shift: {} (continuing)", e);
            }
            abort_if_safe_tip_tripped(ctx, "after centring the frequency shift")?;

            // 7. Final approach with centered freq shift
            ctx.controller.auto_approach(self.wait, timeout)?;
            abort_if_safe_tip_tripped(ctx, "after the final approach")?;

            Ok(())
        })();

        // 8. Always restore safe-tip state before propagating errors
        if !was_enabled && let Err(e) = ctx.controller.safe_tip_set_enabled(false) {
            log::error!("Failed to restore safe-tip state: {}", e);
        }

        result?;
        Ok(ActionOutput::Unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::DataStore;
    use crate::event::EventBus;
    use crate::mock_controller::{FaultKind, MockController};
    use crate::shutdown::ShutdownFlag;

    /// Once safe-tip has fired, the hardware has already retracted the tip.
    /// The one thing the sequence must not do is approach again on top of
    /// that, which is what the final step would do without the check.
    #[test]
    fn a_tripped_safe_tip_aborts_before_the_second_approach() {
        let mut controller = MockController::builder().build();
        let obs = controller.observations();
        let mut store = DataStore::new();
        let events = EventBus::new();
        let shutdown = ShutdownFlag::new();

        // Trip it before the sequence starts: the first status check, right
        // after safe-tip is enabled, is the earliest point the abort can fire.
        obs.lock().safe_tip_tripped = true;

        let mut ctx = ActionContext {
            controller: &mut controller,
            store: &mut store,
            events: &events,
            shutdown: &shutdown,
        };
        let err = CalibratedApproach::default()
            .execute(&mut ctx)
            .expect_err("a tripped safe-tip must abort the approach");
        assert!(
            err.to_string().contains("safe-tip"),
            "the error must say what tripped: {err}"
        );

        let obs = obs.lock();
        assert_eq!(
            obs.approach_count, 1,
            "only the first approach may run; the re-approach must be skipped"
        );
        assert!(
            !obs.called("go_z_home"),
            "the abort fires before the home step, not after it"
        );
        assert!(
            !obs.safe_tip_enabled,
            "safe-tip is restored to its previous state even on abort"
        );
    }

    /// The status check is a guard, not the safety mechanism: an unreadable
    /// status must not turn every calibrated approach into an error.
    #[test]
    fn an_unreadable_status_does_not_abort() {
        let mut controller = MockController::builder()
            .fail_every("z_controller_status", FaultKind::Protocol)
            .build();
        let obs = controller.observations();
        let mut store = DataStore::new();
        let events = EventBus::new();
        let shutdown = ShutdownFlag::new();

        let mut ctx = ActionContext {
            controller: &mut controller,
            store: &mut store,
            events: &events,
            shutdown: &shutdown,
        };
        CalibratedApproach::default()
            .execute(&mut ctx)
            .expect("a status read failure is logged, not fatal");
        assert_eq!(obs.lock().approach_count, 2);
    }
}
