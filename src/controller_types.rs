use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::Signal;

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
pub struct StabilityConfig {
    /// Whether to perform stability checking
    /// When true, performs a scan with bias sweep to verify tip stability
    /// When false, only checks if tip is sharp based on bounds
    pub check_stability: bool,
    /// Maximum allowed change in signal for tip to be considered stable (in Hz)
    /// During the bias sweep, if the signal changes more than this threshold,
    /// the tip is considered unstable
    pub stable_tip_allowed_change: f32,
    /// Bias voltage range for stability sweep (lower, upper) in V
    /// Must be positive magnitude-only; polarity_mode determines sign
    pub bias_range: (f32, f32),
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
    pub scan_speed_m_s: Option<f32>,
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
        voltage: f32,
        #[serde(default)]
        polarity: PolaritySign,
        #[serde(default, alias = "random_switch")]
        random_polarity_switch: Option<RandomPolaritySwitch>,
    },
    Stepping {
        voltage_bounds: (f32, f32),
        voltage_steps: u16,
        cycles_before_step: u16,
        threshold_value: f32,
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
        voltage_bounds: (f32, f32),
        linear_clamp: (f32, f32),
        #[serde(default)]
        polarity: PolaritySign,
        #[serde(default, alias = "random_switch")]
        random_polarity_switch: Option<RandomPolaritySwitch>,
    },
}

impl PulseMethod {
    #[allow(dead_code)]
    pub fn stepping_fixed_threshold(
        voltage_bounds: (f32, f32),
        voltage_steps: u16,
        cycles_before_step: u16,
        threshold_value: f32,
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
    pub fn max_voltage(&self) -> f32 {
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
                    return Err(
                        "voltage_steps must be greater than zero".to_string()
                    );
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

// ============================================================================
// CONTROLLER ACTION & STATE
// ============================================================================

/// Current action being performed by the controller
#[derive(Debug, Clone, Default, PartialEq)]
pub enum ControllerAction {
    #[default]
    Idle,
    Initializing,
    LoadingLayout,
    LoadingSettings,
    SettingBias,
    SettingSetpoint,
    Approaching,
    Withdrawing,
    CenteringFreqShift,
    MeasuringSignal,
    Pulsing,
    StabilityCheck,
    StabilitySweep {
        sweep: u32,
        total: u32,
    },
    Repositioning,
    Completed,
    Stopped,
    Error(String),
}

/// Snapshot of the controller's current state for GUI display
#[derive(Debug, Clone, Default)]
pub struct ControllerState {
    pub tip_shape: crate::TipShape,
    pub cycle_count: u32,
    pub pulse_voltage: f32,
    pub freq_shift: Option<f32>,
    pub elapsed_secs: f64,
    pub current_action: ControllerAction,
}

// ============================================================================
// TIP STATE CONFIG (for action_driver constants)
// ============================================================================

/// Configuration for tip state checking in ActionDriver
#[derive(Debug, Clone)]
pub struct TipStateConfig {
    /// Maximum standard deviation for stable signal (Hz)
    pub max_std_dev: f32,
    /// Maximum slope for stable signal (Hz per sample)
    pub max_slope: f32,
    /// Duration of data collection for tip state checking
    pub data_collection_duration: Duration,
    /// Timeout for stable signal reading during tip state check
    pub read_timeout: Duration,
    /// Number of retries for stable signal reading
    pub read_retry_count: u32,
}

impl Default for TipStateConfig {
    fn default() -> Self {
        Self {
            max_std_dev: 1.0,
            max_slope: 0.01,
            data_collection_duration: Duration::from_millis(500),
            read_timeout: Duration::from_secs(15),
            read_retry_count: 3,
        }
    }
}

// ============================================================================
// TIP CONTROLLER CONFIG
// ============================================================================

pub struct TipControllerConfig {
    pub freq_shift_signal: Signal,
    pub sharp_tip_bounds: (f32, f32),
    pub pulse_method: PulseMethod,
    pub allowed_change_for_stable: f32,
    pub max_cycles: Option<usize>,
    pub max_duration: Option<Duration>,
    pub check_stability: bool,
    pub stability_config: StabilityConfig,
    /// Optional path to a Nanonis layout file to load during initialization
    pub layout_file: Option<String>,
    /// Optional path to a Nanonis settings file to load during initialization
    pub settings_file: Option<String>,
    /// Initial bias voltage (V) set before the first approach
    pub initial_bias_v: f32,
    /// Initial Z-controller setpoint (A) set before the first approach
    pub initial_z_setpoint_a: f32,
    /// Safe tip threshold (A) for safe tip configuration
    pub safe_tip_threshold: f32,
    /// Pulse width for tip pulsing
    pub pulse_width: Duration,
    /// Wait time after approach for signal to stabilize before first measurement
    pub post_approach_settle: Duration,
    /// Wait time after reposition for signal to stabilize
    pub post_reposition_settle: Duration,
    /// Wait time after clearing TCP buffer to accumulate fresh data
    pub buffer_clear_wait: Duration,
    /// Wait time after a bias pulse for signal to settle
    pub post_pulse_settle: Duration,
    /// Number of motor steps for repositioning (x, y)
    pub reposition_steps: (i16, i16),
    /// Report progress every N cycles
    pub status_interval: usize,
}

impl Default for TipControllerConfig {
    fn default() -> Self {
        let freq_shift_signal =
            Signal::new_unchecked("freq_shift", 76, Some(18));

        Self {
            freq_shift_signal,
            sharp_tip_bounds: (-2.0, 0.0),
            pulse_method: PulseMethod::Fixed {
                voltage: 4.0,
                polarity: PolaritySign::Positive,
                random_polarity_switch: None,
            },
            allowed_change_for_stable: 0.2,
            max_cycles: Some(1000),
            max_duration: Some(Duration::from_secs(3600)), // 1 hour
            check_stability: true,
            stability_config: StabilityConfig::default(),
            layout_file: None,
            settings_file: None,
            initial_bias_v: -500e-3,
            initial_z_setpoint_a: 100e-12,
            safe_tip_threshold: 1e-9,
            pulse_width: Duration::from_millis(50),
            post_approach_settle: Duration::from_millis(2000),
            post_reposition_settle: Duration::from_millis(1000),
            buffer_clear_wait: Duration::from_millis(500),
            post_pulse_settle: Duration::from_secs(1),
            reposition_steps: (3i16, 3i16),
            status_interval: 10,
        }
    }
}
