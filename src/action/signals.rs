use std::time::Duration;

use serde::Serialize;

use crate::action::{Action, ActionContext};
use crate::event::Event;
use crate::signal_registry::SignalIndex;
use crate::spm_controller::Capability;

#[derive(Debug, Clone, Serialize)]
pub struct ReadSignal {
    pub index: SignalIndex,
    pub wait_for_newest: bool,
}

impl Default for ReadSignal {
    fn default() -> Self {
        Self {
            index: SignalIndex(0),
            wait_for_newest: true,
        }
    }
}

impl Action for ReadSignal {
    type Output = f64;

    fn name(&self) -> &str {
        "read_signal"
    }
    fn description(&self) -> &str {
        "Read a single signal value by index"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        let val = ctx
            .controller
            .read_signal(self.index, self.wait_for_newest)?;
        Ok(val)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadSignals {
    pub indices: Vec<SignalIndex>,
    pub wait_for_newest: bool,
}

impl Default for ReadSignals {
    fn default() -> Self {
        Self {
            indices: vec![],
            wait_for_newest: true,
        }
    }
}

impl Action for ReadSignals {
    type Output = Vec<(String, f64)>;

    fn name(&self) -> &str {
        "read_signals"
    }
    fn description(&self) -> &str {
        "Read multiple signal values by index"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }

    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        let vals = ctx
            .controller
            .read_signals(&self.indices, self.wait_for_newest)?;
        if vals.len() != self.indices.len() {
            return Err(crate::spm_error::SpmError::Protocol(format!(
                "read_signals: requested {} indices but got {} values",
                self.indices.len(),
                vals.len(),
            )));
        }
        let labeled: Vec<(String, f64)> = self
            .indices
            .iter()
            .zip(vals)
            .map(|(idx, val)| (format!("signal_{}", idx), val))
            .collect();
        Ok(labeled)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReadSignalNames;

impl Action for ReadSignalNames {
    type Output = Vec<String>;

    fn name(&self) -> &str {
        "read_signal_names"
    }
    fn description(&self) -> &str {
        "Read all available signal names from the controller"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }

    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.signal_names()
    }
}

/// Read a stable signal by collecting samples and checking statistical stability.
///
/// Collects `num_samples` from the data stream, then checks that both the
/// standard deviation (Hz) and the drift rate (Hz/s) are within bounds.
/// Retries with exponential backoff (100ms, 200ms, 400ms, ...) if the
/// signal is not stable. Returns the mean of the stable batch.
#[derive(Debug, Clone, Serialize)]
pub struct ReadStableSignal {
    pub index: SignalIndex,
    pub num_samples: usize,
    /// Noise gate: maximum standard deviation of the batch, in Hz.
    pub max_std_dev: f64,
    /// Drift gate: maximum regression slope, in Hz/s.
    pub max_slope: f64,
    pub max_retries: usize,
    /// Rate the samples arrive at, used to turn the per-sample regression
    /// slope into Hz/s. Without it the effective drift tolerance would scale
    /// with `num_samples`, so a tip could pass or fail on batch size alone.
    pub sample_rate_hz: f64,
}

impl Default for ReadStableSignal {
    fn default() -> Self {
        Self {
            index: SignalIndex(0),
            num_samples: 100,
            max_std_dev: 1.5,
            max_slope: 0.5,
            max_retries: 3,
            sample_rate_hz: 2000.0,
        }
    }
}

impl Action for ReadStableSignal {
    type Output = f64;

    fn name(&self) -> &str {
        "read_stable_signal"
    }
    fn description(&self) -> &str {
        "Read a stable signal value with std_dev + slope checking and retries"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }

    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        for attempt in 0..=self.max_retries {
            let samples = ctx
                .controller
                .read_signal_samples(self.index, self.num_samples)?;

            // Treat severely incomplete data as instability — thresholds were
            // tuned for the configured sample count, so computing on too few
            // samples would produce unreliable statistics.
            if samples.len() < self.num_samples / 2 {
                log::warn!(
                    "ReadStableSignal: only got {}/{} samples, treating as unstable (attempt {})",
                    samples.len(),
                    self.num_samples,
                    attempt
                );
                if attempt < self.max_retries {
                    let backoff_ms = 100u64 * (1 << attempt);
                    std::thread::sleep(Duration::from_millis(backoff_ms));
                    continue;
                }
                // Fall through to compute on partial data as last resort
            }

            let (mean, std_dev, slope_per_sample) = compute_stability_metrics(&samples);
            // Compare drift in Hz/s, not Hz/sample, so the gate does not move
            // when `num_samples` or the stream's oversampling changes.
            let drift = slope_per_sample * self.sample_rate_hz;

            let noise_ok = std_dev <= self.max_std_dev;
            let drift_ok = drift.abs() <= self.max_slope;

            if noise_ok && drift_ok {
                log::debug!(
                    "ReadStableSignal: index={}, samples={}, mean={:.6}, std_dev={:.4} Hz, drift={:.4} Hz/s (stable, attempt {})",
                    self.index,
                    samples.len(),
                    mean,
                    std_dev,
                    drift,
                    attempt
                );
                emit_measurement(
                    ctx,
                    self.index,
                    samples.len(),
                    mean,
                    std_dev,
                    drift,
                    slope_per_sample,
                    true,
                );
                return Ok(mean);
            }

            if attempt < self.max_retries {
                let backoff_ms = 100u64 * (1 << attempt);
                log::debug!(
                    "ReadStableSignal: not stable (std_dev={:.4}/{:.4} Hz, drift={:.4}/{:.4} Hz/s, n={}), retry {} in {}ms",
                    std_dev,
                    self.max_std_dev,
                    drift.abs(),
                    self.max_slope,
                    samples.len(),
                    attempt + 1,
                    backoff_ms
                );
                std::thread::sleep(Duration::from_millis(backoff_ms));
            } else {
                log::warn!(
                    "ReadStableSignal: signal not stable after {} retries (std_dev={:.4}/{:.4} Hz, drift={:.4}/{:.4} Hz/s, n={}), using mean={:.6}",
                    self.max_retries,
                    std_dev,
                    self.max_std_dev,
                    drift.abs(),
                    self.max_slope,
                    samples.len(),
                    mean
                );
                emit_measurement(
                    ctx,
                    self.index,
                    samples.len(),
                    mean,
                    std_dev,
                    drift,
                    slope_per_sample,
                    false,
                );
                return Ok(mean);
            }
        }

        // Every loop iteration returns; keep an honest error rather than a
        // panic path in case the control flow above ever changes.
        Err(crate::spm_error::SpmError::Routine(
            "ReadStableSignal: retry loop exited without a result".into(),
        ))
    }
}

/// Publish the outcome of a [`ReadStableSignal`] as one measurement: the value
/// plus the batch statistics it was derived from.
///
/// One event per read, deliberately — a stable read *is* a single measurement of
/// the signal, and the raw sample batch is an implementation detail of how it was
/// averaged. Carrying `std_dev` and `n` keeps the uncertainty available (for an
/// error bar, say) without putting every sample in the log.
///
/// `ActionCompleted` also carries the mean, but only as a bare value; this event
/// is self-describing and stable to parse.
#[allow(clippy::too_many_arguments)]
fn emit_measurement(
    ctx: &ActionContext,
    index: SignalIndex,
    n: usize,
    mean: f64,
    std_dev: f64,
    drift: f64,
    slope_per_sample: f64,
    stable: bool,
) {
    ctx.events.emit(Event::data_collected(
        "stable_read",
        serde_json::json!({
            "index": index,
            "value": mean,
            "std_dev": std_dev,
            "slope": drift,
            "slope_per_sample": slope_per_sample,
            "n": n,
            "stable": stable,
        }),
    ));
}

/// Compute mean, standard deviation, and linear regression slope from sample data.
///
/// The slope is per sample, since this works on a bare slice with no notion of
/// how fast the samples arrived. Callers holding a sample rate should scale it
/// to Hz/s before comparing against a threshold.
pub(crate) fn compute_stability_metrics(data: &[f64]) -> (f64, f64, f64) {
    if data.is_empty() {
        return (f64::NAN, 0.0, 0.0);
    }

    let n = data.len() as f64;
    let mean = data.iter().sum::<f64>() / n;

    if data.len() < 2 {
        return (mean, 0.0, 0.0);
    }

    // Standard deviation (sample, N-1)
    let variance = data
        .iter()
        .map(|&v| {
            let d = v - mean;
            d * d
        })
        .sum::<f64>()
        / (n - 1.0);
    let std_dev = variance.sqrt();

    // Linear regression slope
    let x_mean = (n - 1.0) / 2.0;
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for (i, &v) in data.iter().enumerate() {
        let x_diff = i as f64 - x_mean;
        let y_diff = v - mean;
        numerator += x_diff * y_diff;
        denominator += x_diff * x_diff;
    }
    let slope = if denominator > 0.0 {
        numerator / denominator
    } else {
        0.0
    };

    (mean, std_dev, slope)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slope is per sample, and a caller's sample rate is what turns it
    /// into the Hz/s that thresholds are expressed in. Pinned because getting
    /// this backwards makes the drift gate scale with the batch size: the same
    /// tip then passes or fails on `num_samples` alone.
    #[test]
    fn slope_is_per_sample_and_scales_to_hz_per_second() {
        // 0.01 Hz per sample, dead straight.
        let ramp: Vec<f64> = (0..100).map(|i| i as f64 * 0.01).collect();

        let (_, std_dev, slope_per_sample) = compute_stability_metrics(&ramp);
        assert!((slope_per_sample - 0.01).abs() < 1e-9);

        // At 2 kHz that ramp is 20 Hz/s, far past a 0.5 Hz/s gate...
        assert!((slope_per_sample * 2000.0 - 20.0).abs() < 1e-6);
        // ...though the batch is not *noisy*, which is the other, separate gate.
        assert!(std_dev > 0.0);
    }

    #[test]
    fn a_flat_batch_has_no_drift_at_any_sample_rate() {
        let flat = vec![-1.5_f64; 64];
        let (mean, std_dev, slope_per_sample) = compute_stability_metrics(&flat);

        assert!((mean - -1.5).abs() < 1e-12);
        assert_eq!(std_dev, 0.0);
        assert_eq!(slope_per_sample, 0.0);
    }
}
