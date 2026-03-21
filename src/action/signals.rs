use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::Capability;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadSignal {
    pub index: u32,
    #[serde(default = "super::default_true")]
    pub wait_for_newest: bool,
}


impl Default for ReadSignal {
    fn default() -> Self {
        Self {
            index: 0,
            wait_for_newest: true,
        }
    }
}

impl Action for ReadSignal {
    fn name(&self) -> &str {
        "read_signal"
    }
    fn description(&self) -> &str {
        "Read a single signal value by index"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let val = ctx.controller.read_signal(self.index, self.wait_for_newest)?;
        Ok(ActionOutput::Value(val))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadSignals {
    pub indices: Vec<u32>,
    #[serde(default = "super::default_true")]
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
    fn name(&self) -> &str {
        "read_signals"
    }
    fn description(&self) -> &str {
        "Read multiple signal values by index"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let vals = ctx.controller.read_signals(&self.indices, self.wait_for_newest)?;
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
        Ok(ActionOutput::Values(labeled))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadSignalNames;

impl Action for ReadSignalNames {
    fn name(&self) -> &str {
        "read_signal_names"
    }
    fn description(&self) -> &str {
        "Read all available signal names from the controller"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let names = ctx.controller.signal_names()?;
        let json = serde_json::to_value(names).map_err(|e| {
            crate::spm_error::SpmError::Protocol(format!("Failed to serialize signal names: {}", e))
        })?;
        Ok(ActionOutput::Data(json))
    }
}

/// Read a stable signal by collecting samples and checking statistical stability.
///
/// Collects `num_samples` from the data stream, then checks that both
/// the standard deviation and linear regression slope are within bounds.
/// Retries with exponential backoff (100ms, 200ms, 400ms, ...) if the
/// signal is not stable. Returns the mean of the stable batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadStableSignal {
    pub index: u32,
    #[serde(default = "default_num_samples")]
    pub num_samples: usize,
    #[serde(default = "default_max_std_dev")]
    pub max_std_dev: f64,
    #[serde(default = "default_max_slope")]
    pub max_slope: f64,
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
}

fn default_num_samples() -> usize {
    100
}

fn default_max_std_dev() -> f64 {
    1.0
}

fn default_max_slope() -> f64 {
    0.01
}

fn default_max_retries() -> usize {
    3
}

impl Default for ReadStableSignal {
    fn default() -> Self {
        Self {
            index: 0,
            num_samples: default_num_samples(),
            max_std_dev: default_max_std_dev(),
            max_slope: default_max_slope(),
            max_retries: default_max_retries(),
        }
    }
}

impl Action for ReadStableSignal {
    fn name(&self) -> &str {
        "read_stable_signal"
    }
    fn description(&self) -> &str {
        "Read a stable signal value with std_dev + slope checking and retries"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Signals]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        for attempt in 0..=self.max_retries {
            let samples = ctx
                .controller
                .read_signal_samples(self.index, self.num_samples)?;

            let (mean, std_dev, slope) = compute_stability_metrics(&samples);

            let noise_ok = std_dev <= self.max_std_dev;
            let drift_ok = slope.abs() <= self.max_slope;

            if noise_ok && drift_ok {
                log::debug!(
                    "ReadStableSignal: index={}, samples={}, mean={:.6}, std_dev={:.4}, slope={:.6} (stable, attempt {})",
                    self.index, samples.len(), mean, std_dev, slope, attempt
                );
                return Ok(ActionOutput::Value(mean));
            }

            if attempt < self.max_retries {
                let backoff_ms = 100u64 * (1 << attempt);
                log::debug!(
                    "ReadStableSignal: not stable (std_dev={:.4}/{:.4}, slope={:.6}/{:.6}), retry {} in {}ms",
                    std_dev, self.max_std_dev, slope.abs(), self.max_slope, attempt + 1, backoff_ms
                );
                std::thread::sleep(Duration::from_millis(backoff_ms));
            } else {
                log::warn!(
                    "ReadStableSignal: signal not stable after {} retries (std_dev={:.4}, slope={:.6}), using mean={:.6}",
                    self.max_retries, std_dev, slope, mean
                );
                return Ok(ActionOutput::Value(mean));
            }
        }

        unreachable!()
    }
}

/// Compute mean, standard deviation, and linear regression slope from sample data.
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
