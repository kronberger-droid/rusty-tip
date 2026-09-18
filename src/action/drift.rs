//! Piezo drift compensation actions.
//!
//! The real-time system does the continuous work: it adds a constant velocity
//! to each axis for as long as compensation is on. What it cannot do is work
//! out what that velocity should be, or notice when it has gone stale. That is
//! what these actions are for.
//!
//! # How a drift rate is measured
//!
//! One measurement is a *burst*: every sample the data stream delivers for
//! `window_ms`, thousands of them, fitted with a line. A controller that does
//! not stream the Z signal falls back to `samples` timed reads over the same
//! window, which is far noisier.
//!
//! The fit is not done on the raw samples. Z under feedback carries slow
//! noise, so neighbouring samples are correlated and a textbook standard error
//! computed from them would be optimistic by an order of magnitude. The burst
//! is averaged into [`BLOCKS`] blocks first and the line is fitted through the
//! block means; the scatter of those means about the line is an honest
//! measure of what the slope is worth, and it is what decides when
//! [`CompensateDrift`] stops.

use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::event::Event;
use crate::experiment_log::{LogEvent, ToolSchema};
use crate::signal_registry::SignalIndex;
use crate::spm_controller::{Capability, DriftComp, ZControllerStatus};
use crate::spm_error::SpmError;

/// Blocks a burst is averaged into before the line fit.
const BLOCKS: usize = 10;

/// Longest the stream is read in one call, so a stop request is heard inside
/// a burst rather than after it.
const CHUNK: Duration = Duration::from_millis(500);

fn default_samples() -> usize {
    16
}

fn default_window_ms() -> u64 {
    5_000
}

fn default_bursts() -> usize {
    5
}

fn default_trial_vz() -> f64 {
    20e-12
}

fn default_tolerance() -> f64 {
    0.05e-12
}

fn default_max_vz() -> f64 {
    2e-9
}

/// A drift rate and what it is worth.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DriftEstimate {
    /// Fitted drift rate, metres per second.
    pub rate_m_s: f64,
    /// Standard error of that rate, from the scatter of the block means
    /// about the fitted line.
    pub std_err_m_s: f64,
    /// Samples the fit was made from.
    pub samples: usize,
    /// Time the samples span, in seconds.
    pub window_s: f64,
}

impl DriftEstimate {
    /// Whether the rate is indistinguishable from zero: inside two standard
    /// errors, or below `tolerance_m_s` outright.
    pub fn is_negligible(&self, tolerance_m_s: f64) -> bool {
        self.rate_m_s.abs() <= (2.0 * self.std_err_m_s).max(tolerance_m_s)
    }
}

/// Both actions return their result struct as [`ActionOutput::Data`], so a
/// caller gets it back with `serde_json::from_value`.
fn as_data<T: Serialize>(action: &str, result: &T) -> Result<ActionOutput, SpmError> {
    serde_json::to_value(result)
        .map(ActionOutput::Data)
        .map_err(|e| SpmError::Workflow(format!("{action}: result does not serialize: {e}")))
}

/// What a burst inside [`CompensateDrift`] was for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BurstRole {
    /// The drift as found, before anything was changed.
    Baseline,
    /// Measured with the trial velocity applied, to learn the response.
    Trial,
    /// Measured after a correction, to see what is left.
    Correction,
}

/// One burst of [`CompensateDrift`]: the velocity in effect and the drift
/// measured under it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DriftBurstEvent {
    /// 1-based position in the run.
    pub burst: usize,
    pub role: BurstRole,
    /// Compensation velocity in effect during the burst, m/s.
    pub vz_m_s: f64,
    pub drift_m_s: f64,
    pub std_err_m_s: f64,
}

impl LogEvent for DriftBurstEvent {
    const KIND: &'static str = "drift/burst";
}

/// The kinds these actions write. A tool that runs them includes this in its
/// own schema with [`ToolSchema::including`].
pub fn log_schema() -> ToolSchema {
    ToolSchema::new("drift").with::<DriftBurstEvent>()
}

/// Measure how fast Z is drifting.
///
/// Reads a burst of `z` over `window_ms` and fits a line; see the module docs.
/// Returns the rate together with its standard error, and leaves the judgment
/// to the caller: a rate inside its own error bar is a finding, not a failure.
///
/// The Z controller has to be **on**. With the feedback open, Z is whatever it
/// was parked at and the drift being measured is invisible, so the fit would
/// return a confident zero. That is also why drift can only be measured
/// between passes, never during one.
///
/// Nothing here decides where the tip is. A drift estimate only means
/// something on flat, quiet ground, so position the tip first: measured over a
/// step edge or a molecule this returns a confident, wrong number. Lateral
/// drift over a tilted surface reads as Z drift too, which is what a scan at
/// that spot would see, and is why the number changes with position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasureZDrift {
    /// Signal to watch. The Z position, in metres.
    pub z: SignalIndex,
    #[serde(default = "default_window_ms")]
    pub window_ms: u64,
    /// Timed reads to take when the controller does not stream `z`. Ignored
    /// for a burst, which takes whatever the stream delivers.
    #[serde(default = "default_samples")]
    pub samples: usize,
}

impl MeasureZDrift {
    /// Run the measurement.
    ///
    /// Separate from `execute` so [`CompensateDrift`] can use the estimate
    /// rather than unwrapping it back out of an [`ActionOutput`].
    pub(crate) fn measure(&self, ctx: &mut ActionContext) -> Result<DriftEstimate, SpmError> {
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

        let (times, values) = match ctx.controller.stream_rate_hz() {
            Some(hz) if hz > 0.0 && ctx.controller.streams_signal(self.z) => self.burst(ctx, hz)?,
            _ => self.poll(ctx)?,
        };

        fit_drift(&times, &values).ok_or_else(|| {
            SpmError::Workflow(format!(
                "measure_z_drift: {} samples over {} ms are too few to fit a drift rate",
                values.len(),
                self.window_ms
            ))
        })
    }

    /// Everything the stream delivers for the window, in chunks short enough
    /// that a stop request lands inside the burst.
    ///
    /// The time base is the sample index over the stream rate. The few frames
    /// that pass between two chunk reads are lost, which shortens the real
    /// spacing by well under a percent; against the noise on the slope that
    /// is nothing, and it keeps link latency out of the time axis entirely.
    fn burst(&self, ctx: &mut ActionContext, hz: f64) -> Result<(Vec<f64>, Vec<f64>), SpmError> {
        let wanted = (hz * self.window_ms as f64 / 1000.0).round() as usize;
        let chunk = ((hz * CHUNK.as_secs_f64()).ceil() as usize).max(1);
        let mut values = Vec::with_capacity(wanted);
        while values.len() < wanted {
            ctx.check_shutdown()?;
            let n = chunk.min(wanted - values.len());
            values.extend(ctx.controller.read_signal_samples(self.z, n)?);
        }
        let times = (0..values.len()).map(|k| k as f64 / hz).collect();
        Ok((times, values))
    }

    /// `samples` reads spread over the window, each stamped with the time it
    /// came back, for a controller without a stream.
    fn poll(&self, ctx: &mut ActionContext) -> Result<(Vec<f64>, Vec<f64>), SpmError> {
        if self.samples < 3 {
            return Err(SpmError::Workflow(
                "measure_z_drift: need at least 3 samples to fit a line".into(),
            ));
        }
        let window = Duration::from_millis(self.window_ms);
        let interval = window / (self.samples as u32 - 1);
        let start = Instant::now();
        let mut times = Vec::with_capacity(self.samples);
        let mut values = Vec::with_capacity(self.samples);

        for i in 0..self.samples {
            values.push(ctx.controller.read_signal(self.z, true)?);
            times.push(start.elapsed().as_secs_f64());
            if i + 1 < self.samples {
                // Wait until the next sample is due rather than for a fixed
                // interval, so the round trip is absorbed by the window
                // instead of stretching it.
                let due = start + interval * (i as u32 + 1);
                let remaining = due.saturating_duration_since(Instant::now());
                ctx.settle(remaining.as_millis() as u64)?;
            }
        }
        Ok((times, values))
    }
}

impl Action for MeasureZDrift {
    fn name(&self) -> &str {
        "measure_z_drift"
    }
    fn description(&self) -> &str {
        "Measure the Z drift rate and its standard error, in metres per second"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals, Capability::ZController]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        as_data(self.name(), &self.measure(ctx)?)
    }
}

/// Fit a drift rate through block means of `(times, values)`.
///
/// `None` with fewer than three samples, or when the samples span no time.
fn fit_drift(times: &[f64], values: &[f64]) -> Option<DriftEstimate> {
    let n = values.len().min(times.len());
    if n < 3 {
        return None;
    }
    let blocks = BLOCKS.min(n);
    let mean = |xs: &[f64]| xs.iter().sum::<f64>() / xs.len() as f64;
    let (bt, bz): (Vec<f64>, Vec<f64>) = (0..blocks)
        .map(|b| {
            let (lo, hi) = (b * n / blocks, (b + 1) * n / blocks);
            (mean(&times[lo..hi]), mean(&values[lo..hi]))
        })
        .unzip();

    let (t_mean, z_mean) = (mean(&bt), mean(&bz));
    let sxx: f64 = bt.iter().map(|t| (t - t_mean).powi(2)).sum();
    if sxx <= 0.0 {
        return None;
    }
    let sxz: f64 = bt
        .iter()
        .zip(&bz)
        .map(|(t, z)| (t - t_mean) * (z - z_mean))
        .sum();
    let rate = sxz / sxx;

    let residual: f64 = bt
        .iter()
        .zip(&bz)
        .map(|(t, z)| (z - z_mean - rate * (t - t_mean)).powi(2))
        .sum();
    let std_err = (residual / (blocks as f64 - 2.0) / sxx).sqrt();

    Some(DriftEstimate {
        rate_m_s: rate,
        std_err_m_s: std_err,
        samples: n,
        window_s: times[n - 1] - times[0],
    })
}

/// Where [`CompensateDrift`] left things.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DriftCompensation {
    /// Drift measured under the velocity left on the controller.
    pub residual: DriftEstimate,
    /// Compensation velocity left on the controller, m/s.
    pub vz_m_s: f64,
    /// How the measured drift answers to the velocity, in m/s per m/s: close
    /// to +1 or -1, and the sign is the controller's convention. `None` when
    /// the drift was negligible from the start and no response was given, so
    /// nothing had to be learned.
    pub response: Option<f64>,
    /// Bursts taken, the baseline and any trial included.
    pub bursts: usize,
    /// Whether the last burst read inside its error bar. A check on the
    /// result, not the stop rule: a burst over a perfectly cancelled drift
    /// still fails it now and then, more often the slower the noise on Z.
    pub converged: bool,
}

/// Measure the Z drift and leave the controller compensating for it.
///
/// A burst measures the drift, the velocity is corrected, and the next burst
/// measures what is left, for `bursts` bursts. The reported residual always
/// belongs to the velocity left on the controller, since the loop ends on a
/// measurement, never on a correction.
///
/// Things this handles that a bare `drift_comp_set` does not:
///
/// - **A latched axis is re-armed first.** Compensation stops for good at the
///   saturation limit, so measuring against a stopped axis would fit a drift
///   nothing is correcting and then write into a channel that ignores it.
/// - **The sign is measured, not assumed,** unless `response` says it is
///   known. Which way a positive `vz` moves the piezo is not documented, and
///   guessing wrong does not leave the drift uncorrected, it doubles it. The
///   trial is deliberately large, `trial_vz`, many times the measurement
///   noise: with the loop closed the feedback holds the gap, so the only cost
///   is a Z output that ramps by `trial_vz * window` for one burst, and the
///   response comes out clean. A trial the size of the drift itself, as this
///   used to do, divides noise by noise.
/// - **Corrections average, they do not chase.** The k-th one moves the
///   velocity by a k-th of what its burst says is missing, which leaves the
///   velocity at the mean of every estimate so far, and its error falling
///   with the square root of the bursts spent. Every burst corrects, however
///   small its reading: correcting only the readings that clear their error
///   bar picks out the bursts whose noise ran the same way as the residual,
///   and those overshoot. For the same reason the budget is spent in full
///   rather than ending on the first reading that looks like zero. Only a
///   baseline inside its error bar ends the run early, with nothing changed.
/// - **Positioning is the caller's problem**, as with [`MeasureZDrift`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompensateDrift {
    pub z: SignalIndex,
    #[serde(default = "default_window_ms")]
    pub window_ms: u64,
    /// Polling fallback only; see [`MeasureZDrift::samples`].
    #[serde(default = "default_samples")]
    pub samples: usize,
    /// Bursts to spend, the baseline and the trial included. At least 2, and
    /// 3 when the response has to be learned. A longer `window_ms` buys more
    /// than more bursts: the error of one burst falls with the window to the
    /// power 1.5, the error of the mean only with the root of the count.
    #[serde(default = "default_bursts")]
    pub bursts: usize,
    /// Velocity step used to learn the response, m/s.
    #[serde(default = "default_trial_vz")]
    pub trial_vz: f64,
    /// The response, when it is already known for this controller: +1 if a
    /// positive `vz` adds to the measured drift, -1 if it subtracts. Skips
    /// the trial burst.
    #[serde(default)]
    pub response: Option<f64>,
    /// A residual below this counts as zero even if the burst could resolve
    /// it, m/s.
    #[serde(default = "default_tolerance")]
    pub tolerance_m_s: f64,
    /// Refuse to set a velocity beyond this, m/s. Real drift is orders of
    /// magnitude below it, so getting here means the measurement is broken.
    #[serde(default = "default_max_vz")]
    pub max_vz: f64,
}

impl CompensateDrift {
    /// The defaults, for `z`.
    pub fn new(z: SignalIndex) -> Self {
        Self {
            z,
            window_ms: default_window_ms(),
            samples: default_samples(),
            bursts: default_bursts(),
            trial_vz: default_trial_vz(),
            response: None,
            tolerance_m_s: default_tolerance(),
            max_vz: default_max_vz(),
        }
    }

    /// Run the loop. Separate from `execute` for the same reason as
    /// [`MeasureZDrift::measure`].
    pub(crate) fn compensate(
        &self,
        ctx: &mut ActionContext,
    ) -> Result<DriftCompensation, SpmError> {
        // One burst fewer would end the run on the trial, with the trial
        // velocity left on the controller.
        let needed = match self.response {
            Some(_) => 2,
            None => 3,
        };
        if self.bursts < needed {
            return Err(SpmError::Workflow(format!(
                "compensate_drift: {} bursts are too few, it takes {needed}: one to measure{} \
                 and one to check",
                self.bursts,
                match self.response {
                    Some(_) => "",
                    None => ", one to learn the response",
                }
            )));
        }
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
        // the baseline sees zero, not `comp.vz`.
        let mut vz = match comp.enabled {
            true => comp.vz,
            false => 0.0,
        };
        let set = |ctx: &mut ActionContext, vz: f64| -> Result<(), SpmError> {
            if vz.abs() > self.max_vz {
                return Err(SpmError::Workflow(format!(
                    "compensate_drift: a compensation of {vz:.3e} m/s is beyond the \
                     {:.3e} m/s limit, so the measurement behind it cannot be trusted. \
                     Check the Z signal and the spot under the tip.",
                    self.max_vz
                )));
            }
            ctx.controller.drift_comp_set(&DriftComp {
                enabled: true,
                vz,
                ..comp
            })
        };
        let mut bursts = 0;
        let mut burst = |ctx: &mut ActionContext, role: BurstRole, vz: f64| {
            let estimate = measure.measure(ctx)?;
            bursts += 1;
            ctx.events.emit(Event::typed(&DriftBurstEvent {
                burst: bursts,
                role,
                vz_m_s: vz,
                drift_m_s: estimate.rate_m_s,
                std_err_m_s: estimate.std_err_m_s,
            }));
            Ok::<_, SpmError>((estimate, bursts))
        };

        let (mut estimate, mut taken) = burst(ctx, BurstRole::Baseline, vz)?;
        if estimate.is_negligible(self.tolerance_m_s) {
            return Ok(DriftCompensation {
                converged: true,
                residual: estimate,
                vz_m_s: vz,
                response: self.response,
                bursts: taken,
            });
        }

        let response = match self.response {
            Some(r) => r,
            None => {
                let trial = vz + self.trial_vz;
                set(ctx, trial)?;
                let (at_trial, n) = burst(ctx, BurstRole::Trial, trial)?;
                let measured = (at_trial.rate_m_s - estimate.rate_m_s) / self.trial_vz;
                // The velocity is in the same units as the drift, so a working
                // channel answers close to plus or minus one.
                if !measured.is_finite() || !(0.5..=2.0).contains(&measured.abs()) {
                    set(ctx, vz)?;
                    return Err(SpmError::Workflow(format!(
                        "compensate_drift: stepping vz by {:.3e} m/s changed the measured drift \
                         by {:.3e} m/s, a response of {measured:.2} where a working channel \
                         gives about +1 or -1. The previous velocity is back in place. Check \
                         that the compensation is enabled and not saturated.",
                        self.trial_vz,
                        at_trial.rate_m_s - estimate.rate_m_s
                    )));
                }
                (vz, estimate, taken) = (trial, at_trial, n);
                measured
            }
        };

        // The k-th correction takes a k-th of its burst's reading, which
        // leaves `vz` at the mean of the k estimates made so far.
        let mut k = 0.0;
        while taken < self.bursts {
            k += 1.0;
            vz -= estimate.rate_m_s / (k * response);
            set(ctx, vz)?;
            (estimate, taken) = burst(ctx, BurstRole::Correction, vz)?;
        }

        Ok(DriftCompensation {
            converged: estimate.is_negligible(self.tolerance_m_s),
            residual: estimate,
            vz_m_s: vz,
            response: Some(response),
            bursts: taken,
        })
    }
}

impl Action for CompensateDrift {
    fn name(&self) -> &str {
        "compensate_drift"
    }
    fn description(&self) -> &str {
        "Measure the Z drift in bursts and converge on the velocity that cancels it"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![
            Capability::DriftCompensation,
            Capability::Signals,
            Capability::ZController,
        ]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        as_data(self.name(), &self.compensate(ctx)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line of `rate` m/s sampled at `hz`, with a deterministic wobble of
    /// amplitude `noise` standing in for measurement scatter.
    fn line(rate: f64, hz: f64, n: usize, noise: f64) -> (Vec<f64>, Vec<f64>) {
        let times: Vec<f64> = (0..n).map(|k| k as f64 / hz).collect();
        let values = times
            .iter()
            .enumerate()
            .map(|(k, t)| 1e-7 + rate * t + noise * ((k * 7919 % 13) as f64 / 6.0 - 1.0))
            .collect();
        (times, values)
    }

    #[test]
    fn a_clean_line_fits_exactly_with_no_error_bar() {
        let (t, z) = line(3e-12, 1000.0, 5000, 0.0);
        let fit = fit_drift(&t, &z).unwrap();
        assert!((fit.rate_m_s - 3e-12).abs() < 1e-18, "{fit:?}");
        assert!(fit.std_err_m_s < 1e-18, "{fit:?}");
        assert_eq!(fit.samples, 5000);
    }

    #[test]
    fn scatter_shows_up_in_the_error_bar_and_the_rate_stays_inside_it() {
        let (t, z) = line(1e-12, 1000.0, 5000, 5e-12);
        let fit = fit_drift(&t, &z).unwrap();
        assert!(fit.std_err_m_s > 0.0);
        assert!(
            (fit.rate_m_s - 1e-12).abs() < 3.0 * fit.std_err_m_s,
            "{fit:?}"
        );
    }

    #[test]
    fn a_handful_of_polled_samples_still_fits() {
        let (t, z) = line(2e-12, 3.0, 16, 0.0);
        let fit = fit_drift(&t, &z).unwrap();
        assert!((fit.rate_m_s - 2e-12).abs() < 1e-18, "{fit:?}");
    }

    #[test]
    fn too_few_samples_or_no_time_span_is_no_fit() {
        assert!(fit_drift(&[0.0, 1.0], &[0.0, 1.0]).is_none());
        assert!(fit_drift(&[1.0; 8], &[0.0; 8]).is_none());
    }

    #[test]
    fn negligible_means_inside_two_sigma_or_under_the_floor() {
        let e = |rate: f64, err: f64| DriftEstimate {
            rate_m_s: rate,
            std_err_m_s: err,
            samples: 100,
            window_s: 5.0,
        };
        assert!(e(1e-12, 1e-12).is_negligible(0.0));
        assert!(!e(3e-12, 1e-12).is_negligible(0.0));
        assert!(e(0.04e-12, 0.0).is_negligible(0.05e-12));
    }
}
