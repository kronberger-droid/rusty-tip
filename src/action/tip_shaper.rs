use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::Capability;
use crate::spm_error::SpmError;

/// Serializable tip shaper configuration.
///
/// Maps to nanonis-rs TipShaperConfig but with JSON-friendly field types.
/// Durations are expressed in milliseconds, voltages in volts, distances in meters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipShaperParams {
    pub switch_off_delay_ms: u64,
    pub change_bias: bool,
    pub bias_v: f64,
    pub tip_lift_m: f64,
    pub lift_time_1_ms: u64,
    pub bias_lift_v: f64,
    pub bias_settling_time_ms: u64,
    pub lift_height_m: f64,
    pub lift_time_2_ms: u64,
    pub end_wait_time_ms: u64,
    pub restore_feedback: bool,
}

impl Default for TipShaperParams {
    fn default() -> Self {
        Self {
            switch_off_delay_ms: 0,
            change_bias: true,
            bias_v: 3.0,
            tip_lift_m: 2e-9,
            lift_time_1_ms: 200,
            bias_lift_v: 0.0,
            bias_settling_time_ms: 100,
            lift_height_m: 2e-9,
            lift_time_2_ms: 200,
            end_wait_time_ms: 100,
            restore_feedback: true,
        }
    }
}

impl TipShaperParams {
    fn to_nanonis_config(&self) -> Result<nanonis_rs::tip_recovery::TipShaperConfig, SpmError> {
        Ok(nanonis_rs::tip_recovery::TipShaperConfig {
            switch_off_delay: Duration::from_millis(self.switch_off_delay_ms),
            change_bias: self.change_bias,
            bias_v: self.bias_v as f32,
            tip_lift_m: self.tip_lift_m as f32,
            lift_time_1: Duration::from_millis(self.lift_time_1_ms),
            bias_lift_v: self.bias_lift_v as f32,
            bias_settling_time: Duration::from_millis(self.bias_settling_time_ms),
            lift_height_m: self.lift_height_m as f32,
            lift_time_2: Duration::from_millis(self.lift_time_2_ms),
            end_wait_time: Duration::from_millis(self.end_wait_time_ms),
            restore_feedback: self.restore_feedback,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipShape {
    pub config: TipShaperParams,
    #[serde(default = "super::default_true")]
    pub wait: bool,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    10_000
}

impl Default for TipShape {
    fn default() -> Self {
        Self {
            config: TipShaperParams::default(),
            wait: true,
            timeout_ms: 10_000,
        }
    }
}

impl Action for TipShape {
    fn name(&self) -> &str {
        "tip_shape"
    }
    fn description(&self) -> &str {
        "Execute the tip shaper: lift tip, apply bias, and restore. Used for tip conditioning."
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::TipShaper]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let config = self.config.to_nanonis_config()?;
        ctx.controller
            .tip_shaper(&config, self.wait, Duration::from_millis(self.timeout_ms))?;
        Ok(ActionOutput::Unit)
    }
}
