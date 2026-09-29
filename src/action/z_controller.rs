use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::action::pll::CenterFreqShift;
use crate::action::signals::{compute_stability_metrics, emit_measurement};
use crate::action::util::Wait;
use crate::action::{Action, ActionContext, ActionOutput};
use crate::controllers::ControllerId;
use crate::signal_registry::SignalIndex;
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

/// Start the auto-approach and, with `wait`, poll it to completion through
/// the interruptible settle, so a stop request lands within a poll interval
/// rather than after the approach ends by itself.
///
/// On a stop request or a timeout the auto-approach is switched off before
/// the error is returned. Without that the controller keeps stepping toward
/// the surface while the cleanup that follows tries to withdraw.
fn approach(ctx: &mut ActionContext, wait: bool, timeout: Duration) -> super::Result<()> {
    ctx.controller.auto_approach(false, timeout)?;
    if !wait {
        return Ok(());
    }

    let start = Instant::now();
    loop {
        if let Err(stop) = ctx.settle(100) {
            log::info!("Stop requested during auto-approach; switching it off");
            if let Err(e) = ctx.controller.auto_approach_stop() {
                log::error!("Could not switch the auto-approach off: {e}");
            }
            return Err(stop);
        }
        if !ctx.controller.auto_approach_running()? {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            log::warn!("Auto-approach timed out after {timeout:?}");
            if let Err(e) = ctx.controller.auto_approach_stop() {
                log::error!("Could not switch the auto-approach off: {e}");
            }
            return Err(SpmError::Timeout("Auto-approach timed out".to_string()));
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

/// Budget for one auto-approach when the caller names none: five minutes.
pub const DEFAULT_APPROACH_TIMEOUT_MS: u64 = 300_000;

fn default_approach_timeout_ms() -> u64 {
    DEFAULT_APPROACH_TIMEOUT_MS
}

impl Default for AutoApproach {
    fn default() -> Self {
        Self {
            wait: true,
            timeout_ms: DEFAULT_APPROACH_TIMEOUT_MS,
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
        approach(ctx, self.wait, Duration::from_millis(self.timeout_ms))?;
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

/// Switch the Z-controller loop on or off, leaving Z where it is.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ZControllerSet {
    pub on: bool,
}

impl Action for ZControllerSet {
    fn name(&self) -> &str {
        "z_controller_set"
    }
    fn description(&self) -> &str {
        "Switch the Z-controller loop on or off"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller
            .set_controller_enabled(ControllerId::Z, self.on)?;
        Ok(ActionOutput::Unit)
    }
}

/// What "landed" means: the Z loop's input sitting near its setpoint and
/// holding still, judged on the stream.
///
/// The auto-approach flag drops the moment the setpoint is first crossed,
/// not when the loop has settled. After coarse steps the stage keeps
/// creeping for a while and the loop rides a current well above the
/// setpoint until it stops; after any landing the loop's own step response
/// has to die out. Both show as a batch that is off the setpoint, noisy or
/// drifting, so one gate covers them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandingGate {
    /// The loop's input signal, the current for a current loop.
    pub index: SignalIndex,
    /// The setpoint the loop was given, in the signal's unit.
    pub setpoint: f64,
    /// Fraction of the setpoint within which the batch mean has to sit, and
    /// the bound on its standard deviation and its drift per second. Half
    /// is loose on purpose: the loop being *near* is what matters, not on.
    pub tolerance: f64,
    /// Samples per batch.
    pub num_samples: usize,
    /// Stop waiting after this long and take the landing as done, with a
    /// warning. The batches are in the log either way.
    pub timeout_ms: u64,
    /// Rate the samples arrive at when the controller has not measured its
    /// own; turns the per-sample slope into a drift per second.
    pub sample_rate_hz: f64,
}

/// Wait until the Z loop's input reads stable near the setpoint, or the
/// budget runs out, after which the landing is taken as done with a
/// warning. The batch statistics are the same ones `read_stable_signal`
/// uses and every batch goes to the log as a `landing` measurement, so a
/// slow settle, or one that never came, can be read back afterwards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettleOnSetpoint {
    #[serde(flatten)]
    pub gate: LandingGate,
}

/// Between two batches of the settle.
const SETTLE_POLL_MS: u64 = 100;

impl Action for SettleOnSetpoint {
    fn name(&self) -> &str {
        "settle_on_setpoint"
    }
    fn description(&self) -> &str {
        "Wait until the Z loop's input reads stable near its setpoint"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }

    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let g = &self.gate;
        let band = g.tolerance * g.setpoint.abs();
        if band <= 0.0 || band.is_nan() || g.num_samples == 0 {
            return Err(SpmError::Workflow(format!(
                "settle_on_setpoint needs a non-zero setpoint, tolerance and batch; got \
                 setpoint {:.3e}, tolerance {}, {} samples",
                g.setpoint, g.tolerance, g.num_samples
            )));
        }
        let rate = ctx
            .controller
            .stream_rate_hz()
            .filter(|&hz| hz > 0.0)
            .unwrap_or(g.sample_rate_hz);
        let start = Instant::now();
        let timeout = Duration::from_millis(g.timeout_ms);
        loop {
            // Fresh samples only: what the loop is doing now, not what the
            // buffer holds from the landing.
            ctx.controller.clear_data_buffer();
            let samples = ctx.controller.read_signal_samples(g.index, g.num_samples)?;
            let (mean, std_dev, slope_per_sample) = compute_stability_metrics(&samples);
            let drift = slope_per_sample * rate;
            let settled =
                (mean - g.setpoint).abs() <= band && std_dev <= band && drift.abs() <= band;
            emit_measurement(
                ctx,
                "landing",
                g.index,
                samples.len(),
                mean,
                std_dev,
                drift,
                slope_per_sample,
                settled,
            );
            if settled {
                log::info!(
                    "Z loop settled: {mean:.3e} against {:.3e} ± {band:.1e} after {:.1} s",
                    g.setpoint,
                    start.elapsed().as_secs_f64()
                );
                return Ok(ActionOutput::Value(mean));
            }
            if start.elapsed() >= timeout {
                // Taken as landed anyway: a fixed wait, which is what this
                // replaced, would have carried on too, and the batches are
                // in the log for anyone asking why a cycle went wrong.
                log::warn!(
                    "Z loop not settled after {:.0} s, carrying on: mean {mean:.3e} against \
                     {:.3e} ± {band:.1e}, std_dev {std_dev:.3e}, drift {drift:.3e}/s",
                    timeout.as_secs_f64(),
                    g.setpoint
                );
                return Ok(ActionOutput::Value(mean));
            }
            log::debug!(
                "Z loop not settled yet: mean {mean:.3e} against {:.3e} ± {band:.1e}, \
                 std_dev {std_dev:.3e}, drift {drift:.3e}/s",
                g.setpoint
            );
            ctx.settle(SETTLE_POLL_MS)?;
        }
    }
}

/// Composite action: approach and calibrate frequency shift for a valid reading.
///
/// Sequence:
/// 1. Auto-approach to surface, safe-tip off. The landing after coarse
///    steps carries the stage's creep and the loop's step response; both
///    are harmless with nothing armed and nothing read.
/// 2. Wait until the loop sits near its setpoint ([`LandingGate`]), or a
///    fixed 200 ms when the caller gave no gate.
/// 3. Enable safe-tip protection
/// 4. Z-home (small withdraw ~50nm from surface)
/// 5. Wait 500ms
/// 6. Center frequency shift (while slightly withdrawn)
/// 7. Restore safe-tip to previous state
/// 8. Second landing. With a gate, switch the Z loop on from home and
///    wait for the gate again: the loop walks the 50 nm in on its own
///    integrator and lands softly, which the auto-approach's ramp does not.
///    Without one, auto-approach again and wait 200 ms.
///
/// Safe-tip is armed only while the tip is parked at home, steps 3 to 7,
/// since that is the one stretch where the loop is off and nothing else
/// would notice a contact. Between those steps the Z-controller status is
/// checked and the action aborts if safe-tip has fired: the hardware
/// retracts on its own, what must not happen is step 8 driving the tip
/// straight back at whatever caused it.
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
    /// What "landed" means. `None` falls back to fixed waits and a second
    /// auto-approach.
    #[serde(default)]
    pub landing: Option<LandingGate>,
}

impl Default for CalibratedApproach {
    fn default() -> Self {
        Self {
            wait: true,
            timeout_ms: DEFAULT_APPROACH_TIMEOUT_MS,
            landing: None,
        }
    }
}

impl CalibratedApproach {
    /// After a landing: the gate when there is one, a fixed settle otherwise.
    fn settle_after_landing(&self, ctx: &mut ActionContext) -> super::Result<()> {
        match &self.landing {
            Some(gate) => ctx.run(&SettleOnSetpoint { gate: gate.clone() })?,
            None => ctx.run(&Wait { duration_ms: 200 })?,
        };
        Ok(())
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
        // 1. Initial approach
        ctx.run(&AutoApproach {
            wait: self.wait,
            timeout_ms: self.timeout_ms,
        })?;

        // 2. Let the landing die out before anything is armed.
        self.settle_after_landing(ctx)?;

        // 3. Enable safe-tip
        let was_enabled = ctx.controller.safe_tip_enabled().unwrap_or(false);
        if !was_enabled {
            ctx.run(&SafeTipSet { enabled: true })?;
        }

        // Steps 4-6 wrapped so safe-tip is always restored on exit
        let result = (|| -> super::Result<()> {
            abort_if_safe_tip_tripped(ctx, "after enabling safe-tip")?;

            // 4. Small withdraw to z-home (~50nm above surface)
            ctx.run(&ZHome)?;
            abort_if_safe_tip_tripped(ctx, "after z-home")?;

            // 5. Settle
            ctx.run(&Wait { duration_ms: 500 })?;
            abort_if_safe_tip_tripped(ctx, "after the post-home settle")?;

            // 6. Center freq shift (non-fatal if it fails)
            if let Err(e) = ctx.run(&CenterFreqShift) {
                log::warn!("Failed to center frequency shift: {} (continuing)", e);
            }
            abort_if_safe_tip_tripped(ctx, "after centring the frequency shift")?;

            Ok(())
        })();

        // 7. Always restore safe-tip state before propagating errors
        if !was_enabled && let Err(e) = ctx.run(&SafeTipSet { enabled: false }) {
            log::error!("Failed to restore safe-tip state: {}", e);
        }
        result?;

        // 8. Second landing, with safe-tip as the caller had it.
        match &self.landing {
            Some(_) => ctx.run(&ZControllerSet { on: true })?,
            None => ctx.run(&AutoApproach {
                wait: self.wait,
                timeout_ms: self.timeout_ms,
            })?,
        };
        self.settle_after_landing(ctx)?;
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
            depth: 0,
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

    fn gate(setpoint: f64, timeout_ms: u64) -> LandingGate {
        LandingGate {
            index: SignalIndex(0),
            setpoint,
            tolerance: 0.5,
            num_samples: 8,
            timeout_ms,
            sample_rate_hz: 1000.0,
        }
    }

    /// With a gate the second landing is the loop's own, not the
    /// auto-approach's ramp, and safe-tip is off for both landings: it is
    /// armed after the first has settled and disarmed before the second.
    #[test]
    fn with_a_gate_the_second_landing_is_made_on_the_loop() {
        // Index 0 is the mock's current; the tip model must sit elsewhere.
        let mut controller = MockController::builder()
            .freq_shift_index(SignalIndex(2))
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
            depth: 0,
        };
        ctx.controller.set_z_setpoint(100e-12).unwrap();

        CalibratedApproach {
            landing: Some(gate(100e-12, 1_000)),
            ..Default::default()
        }
        .execute(&mut ctx)
        .expect("a loop sitting on its setpoint lands");

        let obs = obs.lock();
        assert_eq!(obs.approach_count, 1, "the auto-approach runs once");
        assert!(
            obs.called("set_controller_enabled"),
            "the second landing switches the loop on"
        );
        assert!(obs.z_controller_on);
        assert!(!obs.safe_tip_enabled, "safe-tip is back off at the end");
    }

    /// A loop that never reads near the setpoint holds the gate for its
    /// budget and no longer: after that the landing is taken as done, the
    /// way a fixed wait would have.
    #[test]
    fn a_gate_that_never_sees_the_setpoint_gives_up_after_its_budget() {
        let mut controller = MockController::builder()
            .freq_shift_index(SignalIndex(2))
            .build();
        let mut store = DataStore::new();
        let events = EventBus::new();
        let shutdown = ShutdownFlag::new();
        let mut ctx = ActionContext {
            controller: &mut controller,
            store: &mut store,
            events: &events,
            shutdown: &shutdown,
            depth: 0,
        };
        // The mock's current follows its own setpoint, 100 pA; the gate
        // expects 1 nA within half.
        ctx.controller.set_z_setpoint(100e-12).unwrap();

        let start = Instant::now();
        let out = SettleOnSetpoint {
            gate: gate(1e-9, 250),
        }
        .execute(&mut ctx)
        .expect("the budget running out is not an error");
        assert!(
            start.elapsed() >= Duration::from_millis(250),
            "the whole budget is spent before giving up"
        );
        assert!(
            matches!(out, ActionOutput::Value(v) if (v - 100e-12).abs() < 1e-15),
            "what the loop read is what comes back: {out:?}"
        );
    }

    /// A stop during an approach must land inside the approach, not after
    /// it, and must switch the approach off so the controller stops stepping
    /// toward the surface before the cleanup withdraws.
    #[test]
    fn a_stop_request_interrupts_the_approach_and_switches_it_off() {
        let mut controller = MockController::builder()
            .approach_takes_polls(1_000)
            .build();
        let obs = controller.observations();
        let mut store = DataStore::new();
        let events = EventBus::new();
        let shutdown = ShutdownFlag::new();

        let flag = shutdown.clone();
        let requester = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            flag.request();
        });

        let mut ctx = ActionContext {
            controller: &mut controller,
            store: &mut store,
            events: &events,
            shutdown: &shutdown,
            depth: 0,
        };
        let started = std::time::Instant::now();
        let err = AutoApproach {
            wait: true,
            timeout_ms: 60_000,
        }
        .execute(&mut ctx)
        .expect_err("the stop must interrupt the approach");
        requester.join().unwrap();

        assert!(matches!(err, SpmError::ShutdownRequested), "got {err}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the stop must not wait for the approach to finish on its own"
        );
        let obs = obs.lock();
        let start = obs
            .calls
            .iter()
            .position(|c| *c == "auto_approach")
            .expect("the approach was started");
        let stop = obs
            .calls
            .iter()
            .position(|c| *c == "auto_approach_stop")
            .expect("the approach must be switched off on a stop request");
        assert!(stop > start);
    }

    /// The budget is enforced by the action now, and an overrun switches the
    /// approach off the same way a stop does.
    #[test]
    fn an_overrun_approach_is_switched_off_and_reported_as_a_timeout() {
        let mut controller = MockController::builder()
            .approach_takes_polls(1_000)
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
            depth: 0,
        };
        let err = AutoApproach {
            wait: true,
            timeout_ms: 300,
        }
        .execute(&mut ctx)
        .expect_err("an overrun must be an error");

        assert!(matches!(err, SpmError::Timeout(_)), "got {err}");
        assert!(obs.lock().called("auto_approach_stop"));
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
            depth: 0,
        };
        CalibratedApproach::default()
            .execute(&mut ctx)
            .expect("a status read failure is logged, not fatal");
        assert_eq!(obs.lock().approach_count, 2);
    }
}
