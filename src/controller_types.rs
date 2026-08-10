use serde::{Deserialize, Serialize};

// ============================================================================
// BIAS SWEEP POLARITY
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BiasSweepPolarity {
    /// Sweep from upper_bound toward lower_bound (toward zero)
    Positive,
    /// Sweep from -upper_bound toward -lower_bound (toward zero)
    Negative,
    /// Two sweeps: positive first (toward zero), then negative (toward zero)
    #[default]
    Both,
}

// ============================================================================
// STABILITY CONFIG
// ============================================================================

#[derive(Debug, Deserialize, Serialize, Clone)]
// `default` at the container level so a config may set only the stability
// fields it cares about — `configs/tip_prep_no_stability.toml` sets just
// `check_stability = false` — and the rest fall back to `Default`. Without this,
// naming the `[tip_prep.stability]` table at all forced every field to be spelled
// out, which made that config fail to load with a bare "missing field" error.
#[serde(default)]
pub struct StabilityConfig {
    /// Whether to perform stability checking
    /// When true, performs a scan with bias sweep to verify tip stability
    /// When false, only checks if tip is sharp based on bounds
    pub check_stability: bool,
    /// Maximum allowed change in signal for tip to be considered stable (in Hz)
    /// During the bias sweep, if the signal changes more than this threshold,
    /// the tip is considered unstable
    pub stable_tip_allowed_change: f64,
    /// Bias voltage range for stability sweep (lower, upper) in V
    /// Must be positive magnitude-only; polarity_mode determines sign
    pub bias_range: (f64, f64),
    /// Number of steps in the bias sweep
    pub bias_steps: u16,
    /// Time to wait at each step in ms
    pub step_period_ms: u64,
    /// Maximum duration for stability check in seconds
    pub max_duration_secs: u64,
    /// Polarity mode for bias sweep
    #[serde(default)]
    pub polarity_mode: BiasSweepPolarity,
    /// Scan speed for stability check in m/s (None = use current scan speed)
    pub scan_speed_m_s: Option<f64>,
}

impl Default for StabilityConfig {
    fn default() -> Self {
        Self {
            check_stability: true,
            stable_tip_allowed_change: 0.2,
            bias_range: (0.01, 2.0), // Strictly positive range
            bias_steps: 1000,
            step_period_ms: 200,
            max_duration_secs: 100,
            polarity_mode: BiasSweepPolarity::Both,
            scan_speed_m_s: Some(5e-9), // 5 nm/s default
        }
    }
}

impl StabilityConfig {
    /// Validate configuration values
    pub fn validate(&self) -> Result<(), String> {
        if self.bias_range.0 <= 0.0 || self.bias_range.1 <= 0.0 {
            return Err(format!(
                "bias_range must be strictly positive (got [{}, {}]). Use polarity_mode to control sign.",
                self.bias_range.0, self.bias_range.1
            ));
        }
        if self.bias_range.0 >= self.bias_range.1 {
            return Err(format!(
                "bias_range: lower bound ({}) must be less than upper bound ({})",
                self.bias_range.0, self.bias_range.1
            ));
        }
        if self.stable_tip_allowed_change <= 0.0 {
            return Err(format!(
                "stable_tip_allowed_change must be positive, got: {}",
                self.stable_tip_allowed_change
            ));
        }
        if self.bias_steps == 0 {
            return Err("bias_steps must be greater than zero".to_string());
        }
        Ok(())
    }
}

// ============================================================================
// POLARITY
// ============================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolaritySign {
    #[default]
    Positive,
    Negative,
}

impl PolaritySign {
    pub fn opposite(&self) -> Self {
        match self {
            PolaritySign::Positive => PolaritySign::Negative,
            PolaritySign::Negative => PolaritySign::Positive,
        }
    }
}

// ============================================================================
// RANDOM POLARITY SWITCH
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RandomPolaritySwitch {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub switch_every_n_pulses: u32,
}

fn default_enabled() -> bool {
    true
}

// ============================================================================
// PULSE METHOD
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PulseMethod {
    Fixed {
        voltage: f64,
        #[serde(default)]
        polarity: PolaritySign,
        #[serde(default, alias = "random_switch")]
        random_polarity_switch: Option<RandomPolaritySwitch>,
    },
    Stepping {
        voltage_bounds: (f64, f64),
        voltage_steps: u16,
        cycles_before_step: u16,
        threshold_value: f64,
        #[serde(default)]
        polarity: PolaritySign,
        #[serde(default, alias = "random_switch")]
        random_polarity_switch: Option<RandomPolaritySwitch>,
    },
    /// Linear response based on frequency shift
    /// voltage_bounds: (min_voltage, max_voltage) - pulse voltage range in V
    /// linear_clamp: (min_freq, max_freq) - frequency shift range in Hz
    /// If freq_shift is outside linear_clamp range, pulse with max voltage
    /// If freq_shift is inside linear_clamp range, linearly interpolate voltage
    Linear {
        voltage_bounds: (f64, f64),
        linear_clamp: (f64, f64),
        #[serde(default)]
        polarity: PolaritySign,
        #[serde(default, alias = "random_switch")]
        random_polarity_switch: Option<RandomPolaritySwitch>,
    },
}

impl PulseMethod {
    #[allow(dead_code)]
    pub fn stepping_fixed_threshold(
        voltage_bounds: (f64, f64),
        voltage_steps: u16,
        cycles_before_step: u16,
        threshold_value: f64,
        polarity: PolaritySign,
        random_polarity_switch: Option<RandomPolaritySwitch>,
    ) -> PulseMethod {
        PulseMethod::Stepping {
            voltage_bounds,
            voltage_steps,
            cycles_before_step,
            threshold_value: threshold_value.abs(),
            polarity,
            random_polarity_switch,
        }
    }

    pub fn method_name(&self) -> &str {
        match self {
            PulseMethod::Fixed { .. } => "Fixed",
            PulseMethod::Stepping { .. } => "Stepping",
            PulseMethod::Linear { .. } => "Linear",
        }
    }

    /// Get the maximum voltage from this pulse method configuration
    pub fn max_voltage(&self) -> f64 {
        match self {
            PulseMethod::Fixed { voltage, .. } => *voltage,
            PulseMethod::Stepping { voltage_bounds, .. } => voltage_bounds.1,
            PulseMethod::Linear { voltage_bounds, .. } => voltage_bounds.1,
        }
    }

    /// Validate pulse method configuration
    pub fn validate(&self) -> Result<(), String> {
        match self {
            PulseMethod::Fixed { voltage, .. } => {
                if *voltage <= 0.0 {
                    return Err(format!(
                        "Fixed pulse voltage must be positive, got: {}. Use polarity to control sign.",
                        voltage
                    ));
                }
            }
            PulseMethod::Stepping {
                voltage_bounds,
                voltage_steps,
                ..
            } => {
                if voltage_bounds.0 <= 0.0 || voltage_bounds.1 <= 0.0 {
                    return Err(format!(
                        "Stepping voltage_bounds must be positive (got [{}, {}]). Use polarity to control sign.",
                        voltage_bounds.0, voltage_bounds.1
                    ));
                }
                if voltage_bounds.0 >= voltage_bounds.1 {
                    return Err(format!(
                        "Stepping voltage_bounds: min ({}) must be less than max ({})",
                        voltage_bounds.0, voltage_bounds.1
                    ));
                }
                if *voltage_steps == 0 {
                    return Err("voltage_steps must be greater than zero".to_string());
                }
            }
            PulseMethod::Linear {
                voltage_bounds,
                linear_clamp,
                ..
            } => {
                if voltage_bounds.0 <= 0.0 || voltage_bounds.1 <= 0.0 {
                    return Err(format!(
                        "Linear voltage_bounds must be positive (got [{}, {}]). Use polarity to control sign.",
                        voltage_bounds.0, voltage_bounds.1
                    ));
                }
                if voltage_bounds.0 >= voltage_bounds.1 {
                    return Err(format!(
                        "Linear voltage_bounds: min ({}) must be less than max ({})",
                        voltage_bounds.0, voltage_bounds.1
                    ));
                }
                if linear_clamp.0 >= linear_clamp.1 {
                    return Err(format!(
                        "Linear linear_clamp: min ({}) must be less than max ({})",
                        linear_clamp.0, linear_clamp.1
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Default for PulseMethod {
    fn default() -> Self {
        Self::Stepping {
            voltage_bounds: (2.0, 6.0),
            voltage_steps: 4,
            cycles_before_step: 2,
            threshold_value: 0.1,
            polarity: PolaritySign::Positive,
            random_polarity_switch: None,
        }
    }
}
