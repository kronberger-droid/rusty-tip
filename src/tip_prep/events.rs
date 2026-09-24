//! The events tip prep writes to its experiment log, and their declaration.
//!
//! Each struct is one event kind. The JSON written is the struct serialized;
//! the schema in the run header is generated from the same struct, so the
//! two cannot disagree. Add a field here and the schema snapshot test fails
//! until the snapshot is updated, which is the review point for a format
//! change.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::experiment_log::{LogEvent, ToolSchema};

/// One pulse cycle: what was fired, what was measured afterwards.
///
/// The GUI's status panel and pulse history are drawn from this.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct CycleEvent {
    /// 1-based cycle number.
    pub cycle: usize,
    /// Seconds since the budget clock started.
    pub elapsed_secs: f64,
    /// Frequency shift measured after the reposition, in Hz.
    pub freq_shift: f64,
    /// The pulse as fired, sign included, in volts.
    pub pulse_voltage: f64,
    /// Whether `freq_shift` fell inside the sharp window.
    pub is_sharp: bool,
}

impl LogEvent for CycleEvent {
    const KIND: &'static str = "tip_prep/cycle";
}

/// A change of phase within the sharpness confirmation and stability check.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum PhaseEvent {
    /// Three reposition-and-read confirmations are starting.
    Confirming,
    /// Confirmed sharp; the bias sweeps are starting.
    StabilityCheck {
        /// The confirmed frequency shift the final read is compared to, Hz.
        baseline_freq_shift: f64,
    },
    /// The final read stayed within the allowed change: the tip is done.
    Stable { final_freq_shift: f64 },
    /// The final read moved past the allowed change.
    Unstable {
        final_freq_shift: f64,
        /// `|final - baseline|`, Hz.
        change: f64,
        /// The configured allowed change, Hz.
        threshold: f64,
    },
}

impl LogEvent for PhaseEvent {
    const KIND: &'static str = "tip_prep/phase";
}

/// The maximum-voltage pulse fired after a failed stability check, with the
/// tip still engaged from the final read.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct MaxPulseEvent {
    /// The pulse as fired, sign included, in volts.
    pub pulse_voltage: f64,
}

impl LogEvent for MaxPulseEvent {
    const KIND: &'static str = "tip_prep/max_pulse";
}

/// Everything a tip-prep log can contain beyond the built-in events,
/// including what the routine harness emits on its behalf.
pub fn log_schema() -> ToolSchema {
    ToolSchema::new("tip_prep")
        .with::<CycleEvent>()
        .with::<PhaseEvent>()
        .with::<MaxPulseEvent>()
        .including(crate::routine::log_schema())
}
