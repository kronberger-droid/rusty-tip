//! Piezo drift compensation actions.
//!
//! The real-time system does the continuous work: it adds a constant velocity
//! to each axis for as long as compensation is on. What it cannot do is work
//! out what that velocity should be, or notice when it has gone stale. That is
//! what these actions are for.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::action::signals::compute_stability_metrics;
use crate::action::{Action, ActionContext, ActionOutput};
use crate::signal_registry::SignalIndex;
use crate::spm_controller::{Capability, DriftComp, ZControllerStatus};
use crate::spm_error::SpmError;

fn default_samples() -> usize {
    16
}

fn default_window_ms() -> u64 {
    5_000
}

/// Measure how fast Z is drifting, in metres per second.
///
/// Takes `samples` readings of `z` spread over `window_ms` and fits a line.
///
/// The Z controller has to be **on**. With the feedback open, Z is whatever it
/// was parked at and the drift being measured is invisible, so the fit would
/// return a confident zero. That is also why drift can only be measured
/// between passes, never during one.
///
/// Nothing here decides where the tip is. A drift estimate only means
/// something on flat, quiet ground, so position the tip first: measured over a
/// step edge or a molecule this returns a confident, wrong number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasureZDrift {
    /// Signal to watch. The Z position, in metres.
    pub z: SignalIndex,
    #[serde(default = "default_window_ms")]
    pub window_ms: u64,
    #[serde(default = "default_samples")]
    pub samples: usize,
}

impl MeasureZDrift {
    /// Run the measurement, returning metres per second.
    ///
    /// Separate from `execute` so [`CompensateDrift`] can use the number
    /// rather than unwrapping it back out of an [`ActionOutput`].
    pub(crate) fn measure(&self, ctx: &mut ActionContext) -> Result<f64, SpmError> {
        if self.samples < 3 {
            return Err(SpmError::Workflow(
                "measure_z_drift: need at least 3 samples to fit a line".into(),
            ));
        }
        // Anything but `On` means Z is not tracking the surface: held,
        // withdrawing, or stopped by tip protection. The fit would read
        // whatever Z happened to be doing instead.
        let status = ctx.controller.z_controller_status()?;
        if status != ZControllerStatus::On {
            return Err(SpmError::Workflow(format!(
                "measure_z_drift: the Z controller is {status:?}, not On, so Z is not \
                 following the surface and there is no drift to measure"
            )));
        }

        // A scan moves Z over topography, and a line of topography fits as a
        // drift of nanometres per second. Only checked when the controller
        // scans at all, so this does not force the capability on one that
        // does not.
        if ctx
            .controller
            .capabilities()
            .contains(&Capability::Scanning)
            && ctx.controller.scan_status()?
        {
            return Err(SpmError::Workflow(
                "measure_z_drift: a scan is running, so Z is tracking topography rather \
                 than drift. Stop the scan first."
                    .into(),
            ));
        }

        let window = Duration::from_millis(self.window_ms);
        let interval = window / (self.samples as u32 - 1);
        let start = Instant::now();
        let mut values = Vec::with_capacity(self.samples);

        for i in 0..self.samples {
            values.push(ctx.controller.read_signal(self.z, true)?);
            if i + 1 < self.samples {
                // Wait until the next sample is due rather than for a fixed
                // interval, so the round trip is absorbed by the window
                // instead of stretching it. Each read blocks for the newest
                // sample, which on a slow link is milliseconds per point.
                let due = start + interval * (i as u32 + 1);
                let remaining = due.saturating_duration_since(Instant::now());
                ctx.settle(remaining.as_millis() as u64)?;
            }
        }

        let elapsed = start.elapsed().as_secs_f64();
        if elapsed <= 0.0 {
            return Err(SpmError::Workflow(
                "measure_z_drift: no time passed between the first and last sample".into(),
            ));
        }

        // `compute_stability_metrics` fits per sample, and the samples are
        // evenly spaced by construction, so the measured elapsed time converts
        // that to per second. Using the real elapsed time rather than the
        // nominal one keeps link latency out of the answer.
        let (_, std_dev, slope_per_sample) = compute_stability_metrics(&values);

        // How far the fitted line says Z moved across the whole window. If
        // that is smaller than the scatter between neighbouring samples, the
        // slope is reading noise, and a velocity derived from it is worse than
        // none: it would be applied with confidence and drive Z somewhere.
        // Coarse on purpose. The residual returned by `CompensateDrift` is the
        // real check, since it runs after the fact.
        let change = slope_per_sample * (self.samples as f64 - 1.0);
        if change.abs() <= std_dev {
            return Err(SpmError::Workflow(format!(
                "measure_z_drift: Z moved {change:.3e} m over the window, below the \
                 {std_dev:.3e} m sample noise, so no drift rate can be resolved. Use a \
                 longer window, or accept that the drift is negligible here."
            )));
        }

        Ok(change / elapsed)
    }
}

impl Action for MeasureZDrift {
    fn name(&self) -> &str {
        "measure_z_drift"
    }
    fn description(&self) -> &str {
        "Measure the Z drift rate in metres per second"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals, Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        Ok(ActionOutput::Value(self.measure(ctx)?))
    }
}

/// Measure the Z drift and leave the controller compensating for it.
///
/// Returns the residual drift, i.e. what is left afterwards, so a caller can
/// tell a good correction from a useless one without knowing how any of this
/// works. Costs three measurement windows.
///
/// Three things this handles that a bare `drift_comp_set` does not:
///
/// - **A latched axis is re-armed first.** Compensation stops for good at the
///   saturation limit, so measuring against a stopped axis would fit a drift
///   nothing is correcting and then write into a channel that ignores it.
/// - **The sign is measured, not assumed.** Which way a positive `vz` moves
///   the piezo is not documented anywhere we can check, and guessing wrong
///   does not leave the drift uncorrected, it doubles it. So this applies a
///   trial velocity, watches how the drift responds, and solves for the
///   velocity that cancels it. A channel that does not respond at all is an
///   error rather than a coin flip.
/// - **Positioning is the caller's problem**, as with [`MeasureZDrift`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompensateDrift {
    pub z: SignalIndex,
    #[serde(default = "default_window_ms")]
    pub window_ms: u64,
    #[serde(default = "default_samples")]
    pub samples: usize,
}

impl Action for CompensateDrift {
    fn name(&self) -> &str {
        "compensate_drift"
    }
    fn description(&self) -> &str {
        "Measure the Z drift and set the compensation velocity that cancels it"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![
            Capability::DriftCompensation,
            Capability::Signals,
            Capability::ZController,
        ]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let measure = MeasureZDrift {
            z: self.z,
            window_ms: self.window_ms,
            samples: self.samples,
        };

        let mut comp = ctx.controller.drift_comp_get()?;
        if comp.z_saturated {
            ctx.controller.drift_comp_set(&DriftComp {
                enabled: false,
                ..comp
            })?;
            ctx.controller.drift_comp_set(&DriftComp {
                enabled: true,
                ..comp
            })?;
            comp = ctx.controller.drift_comp_get()?;
        }

        // A velocity that is configured but switched off is not in effect, so
        // the baseline the first measurement sees is zero, not `comp.vz`.
        // Reading it as `comp.vz` would fit the response against a velocity
        // the machine never applied.
        let v0 = match comp.enabled {
            true => comp.vz,
            false => 0.0,
        };
        let s0 = measure.measure(ctx)?;

        // The trial velocity doubles as the first guess: if the sign
        // convention runs the way we would guess, this already cancels the
        // drift and the solve below simply confirms it.
        let v1 = v0 - s0;
        ctx.controller.drift_comp_set(&DriftComp {
            enabled: true,
            vz: v1,
            ..comp
        })?;
        let s1 = measure.measure(ctx)?;

        let vz = solve_compensation(v0, s0, v1, s1).ok_or_else(|| {
            SpmError::Workflow(format!(
                "compensate_drift: changing vz from {v0} to {v1} m/s did not change the \
                 measured drift ({s0} then {s1} m/s), so the Z compensation is not \
                 responding. Check that it is enabled and not saturated."
            ))
        })?;

        ctx.controller.drift_comp_set(&DriftComp {
            enabled: true,
            vz,
            ..comp
        })?;
        Ok(ActionOutput::Value(measure.measure(ctx)?))
    }
}

/// Solve for the compensation velocity that cancels the drift.
///
/// Given the drift `s0` measured at compensation velocity `v0`, and `s1`
/// measured at `v1`, the drift responds to the velocity as
/// `s(v) = s0 + response * (v - v0)`, and this returns the `v` where that
/// reaches zero. Deliberately not assuming what `response` is: its sign is the
/// undocumented convention we are trying to avoid guessing at, and its
/// magnitude covers a channel that is scaled rather than one-to-one.
///
/// `None` when the drift did not respond, which means the compensation is off,
/// saturated, or otherwise not connected to anything.
fn solve_compensation(v0: f64, s0: f64, v1: f64, s1: f64) -> Option<f64> {
    let response = (s1 - s0) / (v1 - v0);
    // A response far below 1 cannot be a working compensation channel: the
    // velocity is in the same units as the drift, so any sane channel is close
    // to plus or minus one.
    if !response.is_finite() || response.abs() < 0.1 {
        return None;
    }
    Some(v0 - s0 / response)
}

#[cfg(test)]
mod tests {
    use super::solve_compensation;

    /// 5 pm/s of drift, measured with no compensation applied yet.
    const DRIFT: f64 = 5e-12;

    /// The measured drift after applying `v`, for a channel where a positive
    /// velocity adds to the drift (`sign` = 1) or subtracts from it (-1).
    fn measured(v: f64, sign: f64) -> f64 {
        DRIFT + sign * v
    }

    #[test]
    fn solves_for_either_sign_convention() {
        // The whole point: neither branch knows which convention it is in, and
        // both land on the velocity that zeroes the drift.
        for sign in [1.0, -1.0] {
            let (v0, s0) = (0.0, measured(0.0, sign));
            let v1 = v0 - s0;
            let vz = solve_compensation(v0, s0, v1, measured(v1, sign)).unwrap();
            assert!(
                measured(vz, sign).abs() < 1e-24,
                "sign {sign}: vz {vz} leaves {} m/s",
                measured(vz, sign)
            );
        }
    }

    #[test]
    fn solves_when_compensation_is_already_partly_applied() {
        // Re-running against a machine that is already compensating must not
        // throw away the velocity that is already in effect.
        let sign = -1.0;
        let v0 = 3e-12;
        let s0 = measured(v0, sign);
        let v1 = v0 - s0;
        let vz = solve_compensation(v0, s0, v1, measured(v1, sign)).unwrap();
        assert!((vz - DRIFT).abs() < 1e-24, "expected {DRIFT}, got {vz}");
    }

    #[test]
    fn solves_a_channel_that_is_scaled_rather_than_one_to_one() {
        let scaled = |v: f64| DRIFT - 0.5 * v;
        let (v0, s0) = (0.0, scaled(0.0));
        let v1 = v0 - s0;
        let vz = solve_compensation(v0, s0, v1, scaled(v1)).unwrap();
        assert!(scaled(vz).abs() < 1e-24);
    }

    #[test]
    fn refuses_a_channel_that_does_not_respond() {
        // Saturated, disabled, or otherwise not wired to the piezo: the drift
        // is unchanged by the trial velocity. Dividing by that response would
        // produce a huge confident number.
        assert_eq!(solve_compensation(0.0, DRIFT, -DRIFT, DRIFT), None);
        // And the degenerate case where no trial was actually applied.
        assert_eq!(solve_compensation(0.0, DRIFT, 0.0, DRIFT), None);
    }
}
