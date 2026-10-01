//! The envelope acting requests have to stay inside.
//!
//! A TOML file the operator writes and the agent cannot change: the server
//! loads it at start and reports it in `describe` and `status`, so an agent
//! plans inside it instead of finding it by being refused. Acting requests,
//! as they arrive, check the limits that concern them before anything
//! reaches the controller; nothing acts yet. A field left out sets no
//! limit.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bounds on what acting requests may do. See the [module docs](self).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Largest bias magnitude a request may set, in volts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bias_v: Option<f64>,
    /// Largest bias pulse magnitude, in volts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pulse_v: Option<f64>,
    /// Most coarse-motor steps one request may take on any axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_coarse_steps: Option<u32>,
    /// The range a Z-controller setpoint may be set within, `[low, high]`,
    /// in the unit of the loop's input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_setpoint: Option<[f64; 2]>,
}

impl Limits {
    /// Read and check a limits file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let limits: Limits =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        limits
            .validate()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(limits)
    }

    /// Every bound positive and finite, and a range low to high.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("max_bias_v", self.max_bias_v),
            ("max_pulse_v", self.max_pulse_v),
        ] {
            if let Some(v) = value
                && (v.is_nan() || v <= 0.0 || v.is_infinite())
            {
                return Err(format!("{name} has to be positive and finite, not {v}"));
            }
        }
        if let Some([low, high]) = self.z_setpoint
            && (low.is_nan() || high.is_nan() || low > high)
        {
            return Err(format!(
                "z_setpoint has to run low to high, not [{low}, {high}]"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limits_file_parses_and_a_typo_is_an_error() {
        let limits: Limits =
            toml::from_str("max_bias_v = 2.0\nmax_coarse_steps = 5\nz_setpoint = [10e-12, 1e-9]\n")
                .unwrap();
        assert_eq!(limits.max_bias_v, Some(2.0));
        assert!(limits.validate().is_ok());
        assert!(
            toml::from_str::<Limits>("max_bais_v = 2.0").is_err(),
            "a misspelt limit must not silently set none"
        );
    }

    #[test]
    fn nonsense_bounds_are_refused() {
        let bad = |l: Limits| assert!(l.validate().is_err(), "{l:?}");
        bad(Limits {
            max_bias_v: Some(-1.0),
            ..Limits::default()
        });
        bad(Limits {
            z_setpoint: Some([1e-9, 1e-12]),
            ..Limits::default()
        });
    }
}
