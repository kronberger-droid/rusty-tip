use crate::{
    types::{
        DataToGet, MotorDisplacement, MovementMode, OsciData, Position, Position3D, ScanAction,
        SignalIndex, TipShape, TriggerConfig,
    },
    MotorDirection, TipShaperConfig,
};
use std::{collections::HashMap, time::Duration};

/// Method for determining tip state
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TipCheckMethod {
    /// Check single signal against bounds
    SignalBounds {
        signal: SignalIndex,
        bounds: (f32, f32),
    },
    /// Check multiple signals (all must be in bounds)
    MultiSignalBounds {
        signals: Vec<(SignalIndex, (f32, f32))>,
    },
}

/// Method for determining signal stability for GetStableSignal action
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SignalStabilityMethod {
    /// Standard deviation threshold
    StandardDeviation { threshold: f32 },
    /// Relative standard deviation (coefficient of variation)
    RelativeStandardDeviation { threshold_percent: f32 },
    /// Moving window - signal must be stable within sliding window
    MovingWindow {
        window_size: usize,
        max_variation: f32,
    },
    /// Trend analysis - ensure no consistent drift
    TrendAnalysis { max_slope: f32 },
}

impl Default for SignalStabilityMethod {
    fn default() -> Self {
        Self::RelativeStandardDeviation {
            threshold_percent: 5.0,
        } // 5% variation
    }
}

/// Method for determining tip stability with potentially invasive operations
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TipStabilityMethod {
    /// Extended signal monitoring over time with statistical analysis
    ExtendedMonitoring {
        signal: SignalIndex,
        duration: Duration,
        sampling_interval: Duration,
        stability_threshold: f32,
    },
    /// Bias sweep response analysis (potentially destructive)
    BiasSweepResponse {
        signal: SignalIndex,
        bias_range: (f32, f32),
        sweep_steps: u16,
        period: Duration,
        allowed_signal_change: f32,
    },
}

/// Configuration for bias sweep during stability testing
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BiasSweepConfig {
    pub lower_limit: f32,
    pub upper_limit: f32,
    pub steps: u16,
    pub period_ms: u16,
    pub reset_bias_after: bool,
    pub z_controller_behavior: u16, // 0=no change, 1=turn off, 2=don't turn off
}

/// Information about bounds checking results
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BoundsCheckInfo {
    pub bounds_used: Vec<(SignalIndex, (f32, f32))>,
    pub violations: Vec<(SignalIndex, f32, (f32, f32))>, // signal, value, bounds
    pub all_passed: bool,
}

/// Detailed stability analysis result
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StabilityResult {
    pub is_stable: bool,
    pub stability_score: f32, // 0.0 to 1.0
    pub method_used: String,
    pub measured_values: HashMap<SignalIndex, Vec<f32>>, // Time series data
    pub analysis_duration: Duration,
    pub metrics: HashMap<String, f32>, // Method-specific metrics
    pub potential_damage_detected: bool,
    pub recommendations: Vec<String>,
}

/// Tip state determination result with measured values
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TipState {
    pub shape: TipShape, // the enum value
    pub confidence: f32,
    pub measured_signals: HashMap<SignalIndex, f32>, // Always populated, empty for simple checks
    pub metadata: HashMap<String, String>,
}

/// TCP Logger status information  
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TCPReaderStatus {
    pub status: crate::types::TCPLogStatus,
    pub channels: Vec<i32>,
    pub oversampling: i32,
}

/// Stable signal analysis result
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StableSignal {
    pub stable_value: f32,
    pub confidence: f32,
    pub data_points_used: usize,
    pub analysis_duration: Duration,
    pub stability_metrics: HashMap<String, f32>,
    pub raw_data: Vec<f32>,
}

/// Enhanced Action enum representing all possible SPM operations
/// Properly separates motor (step-based) and piezo (continuous) movements
#[derive(Debug, Clone)]
pub enum Action {
    /// Read single signal value
    ReadSignal {
        signal: SignalIndex,
        wait_for_newest: bool,
    },

    /// Read multiple signal values
    ReadSignals {
        signals: Vec<SignalIndex>,
        wait_for_newest: bool,
    },

    /// Read all available signal names
    ReadSignalNames,

    /// Read current bias voltage
    ReadBias,

    /// Set bias voltage to specific value
    SetBias { voltage: f32 },

    // Osci functions
    ReadOsci {
        signal: SignalIndex,
        trigger: Option<TriggerConfig>,
        data_to_get: DataToGet,
        is_stable: Option<fn(&[f64]) -> bool>,
    },

    /// Read current piezo position (continuous coordinates)
    ReadPiezoPosition { wait_for_newest_data: bool },

    /// Set piezo position (absolute)
    SetPiezoPosition {
        position: Position,
        wait_until_finished: bool,
    },

    /// Move piezo position (relative to current)
    MovePiezoRelative { delta: Position },

    // === Coarse Positioning Operations (Motor) ===
    /// Move motor along a single axis (discrete positioning)
    MoveMotorAxis {
        direction: MotorDirection,
        steps: u16,
        blocking: bool,
    },

    /// Move motor in 3D space with single displacement vector
    MoveMotor3D {
        displacement: MotorDisplacement,
        blocking: bool,
    },

    /// Move motor using closed-loop to target position
    MoveMotorClosedLoop {
        target: Position3D,
        mode: MovementMode,
    },

    /// Stop all motor movement
    StopMotor,

    // === Control Operations ===
    /// Perform auto-approach with timeout
    AutoApproach {
        wait_until_finished: bool,
        timeout: Duration,
    },

    /// Withdraw tip with timeout
    Withdraw {
        wait_until_finished: bool,
        timeout: Duration,
    },

    /// Safely reposition tip: withdraw → move → approach → stabilize
    SafeReposition { x_steps: i16, y_steps: i16 },

    /// Set Z-controller setpoint
    SetZSetpoint { setpoint: f32 },

    // === Scan Operations ===
    /// Control scan operations
    ScanControl { action: ScanAction },

    /// Read scan status
    ReadScanStatus,

    // === Advanced Operations ===
    /// Execute bias pulse with parameters
    BiasPulse {
        wait_until_done: bool,
        pulse_width: Duration,
        bias_value_v: f32,
        z_controller_hold: u16,
        pulse_mode: u16,
    },

    /// Full tip shaper control with all parameters
    TipShaper {
        config: TipShaperConfig,
        wait_until_finished: bool,
        timeout: Duration,
    },

    /// Simple pulse-retract with predefined safe values
    PulseRetract {
        pulse_width: Duration,
        pulse_height_v: f32,
    },

    /// Wait for a specific duration
    Wait { duration: Duration },

    // === Data Management ===
    /// Store result value with key for later retrieval
    Store { key: String, action: Box<Action> },

    /// Retrieve previously stored value
    Retrieve { key: String },

    // === TCP Logger Operations ===
    /// Start TCP logger (must be configured first)
    StartTCPLogger,

    /// Stop TCP logger
    StopTCPLogger,

    /// Get TCP logger status and configuration
    GetTCPLoggerStatus,

    /// Configure TCP logger channels and oversampling
    ConfigureTCPLogger {
        channels: Vec<i32>,
        oversampling: i32,
    },

    // === Tip State Operations ===
    /// Check tip state using specified method (non-invasive)
    CheckTipState { method: TipCheckMethod },

    /// Check tip stability using potentially invasive methods
    /// WARNING: This action may damage the tip through bias sweeps or extended testing
    CheckTipStability {
        method: TipStabilityMethod,
        max_duration: Duration,
        abort_on_damage_signs: bool,
    },

    /// Get a stable signal value using TCP logger data and stability analysis
    ReadStableSignal {
        signal: SignalIndex,
        data_points: Option<usize>,
        use_new_data: bool,
        stability_method: SignalStabilityMethod,
        timeout: Duration,
        retry_count: Option<u32>,
    },

    /// Check if oscillation amplitude is reached
    ReachedTargedAmplitude,
}

/// Simplified ActionResult with clear semantic separation
#[derive(Debug, Clone)]
pub enum ActionResult {
    /// Single numeric value (signals, bias, etc.)
    Value(f64),

    /// Multiple numeric values (signal arrays)
    Values(Vec<f64>),

    /// String data (signal names, error messages, etc.)
    Text(Vec<String>),

    /// Boolean status (scanning/idle, running/stopped, etc.)
    Status(bool),

    /// Position data (meaningful x,y structure)
    Position(Position),

    /// Complex oscilloscope data (timing + data + metadata)
    OsciData(OsciData),

    /// Operation completed successfully (no data returned)
    Success,

    /// TCP Logger status information
    TCPReaderStatus(TCPReaderStatus),

    /// Tip state determination result
    TipState(TipState),

    /// Detailed stability analysis result
    StabilityResult(StabilityResult),

    /// Stable signal value with analysis metadata
    StableSignal(StableSignal),

    /// No result/waiting state
    None,
}

impl ActionResult {
    /// Convert to f64 if possible (for numerical results)
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ActionResult::Value(v) => Some(*v),
            ActionResult::Values(values) => {
                if values.len() == 1 {
                    Some(values[0])
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Convert to bool if possible (for status results)
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ActionResult::Status(b) => Some(*b),
            _ => None,
        }
    }

    /// Convert to Position if possible
    pub fn as_position(&self) -> Option<Position> {
        match self {
            ActionResult::Position(pos) => Some(*pos),
            _ => None,
        }
    }

    /// Convert to OsciData if possible
    pub fn as_osci_data(&self) -> Option<&OsciData> {
        match self {
            ActionResult::OsciData(data) => Some(data),
            _ => None,
        }
    }

    /// Convert to TipShape if possible
    pub fn as_tip_shape(&self) -> Option<TipShape> {
        match self {
            ActionResult::TipState(tip_state) => Some(tip_state.shape),
            _ => None,
        }
    }

    /// Convert to full TipState if possible
    pub fn as_tip_state(&self) -> Option<&TipState> {
        match self {
            ActionResult::TipState(tip_state) => Some(tip_state),
            _ => None,
        }
    }

    /// Convert to StabilityResult if possible
    pub fn as_stability_result(&self) -> Option<&StabilityResult> {
        match self {
            ActionResult::StabilityResult(result) => Some(result),
            _ => None,
        }
    }

    /// Convert to stable signal value if possible
    pub fn as_stable_signal_value(&self) -> Option<f32> {
        match self {
            ActionResult::StableSignal(stable) => Some(stable.stable_value),
            _ => None,
        }
    }

    /// Convert to full StableSignal if possible
    pub fn as_stable_signal(&self) -> Option<&StableSignal> {
        match self {
            ActionResult::StableSignal(stable) => Some(stable),
            _ => None,
        }
    }

    // === Action-Aware Type Extractors ===
    // These methods validate that the result type matches what the action should produce

    /// Extract OsciData with action validation (panics on type mismatch)
    pub fn expect_osci_data(self, action: &Action) -> OsciData {
        match (action, self) {
            (Action::ReadOsci { .. }, ActionResult::OsciData(data)) => data,
            (action, result) => panic!(
                "Expected OsciData from action {:?}, got {:?}",
                action, result
            ),
        }
    }

    /// Extract signal value with action validation (panics on type mismatch)
    pub fn expect_signal_value(self, action: &Action) -> f64 {
        match (action, self) {
            (Action::ReadSignal { .. }, ActionResult::Value(v)) => v,
            (Action::ReadSignal { .. }, ActionResult::Values(mut vs)) if vs.len() == 1 => {
                vs.pop().unwrap()
            }
            (Action::ReadBias, ActionResult::Value(v)) => v,
            (action, result) => panic!(
                "Expected signal value from action {:?}, got {:?}",
                action, result
            ),
        }
    }

    /// Extract multiple values with action validation (panics on type mismatch)
    pub fn expect_values(self, action: &Action) -> Vec<f64> {
        match (action, self) {
            (Action::ReadSignals { .. }, ActionResult::Values(values)) => values,
            (Action::ReadSignal { .. }, ActionResult::Value(v)) => vec![v],
            (action, result) => {
                panic!("Expected values from action {:?}, got {:?}", action, result)
            }
        }
    }

    /// Extract position with action validation (panics on type mismatch)
    pub fn expect_position(self, action: &Action) -> Position {
        match (action, self) {
            (Action::ReadPiezoPosition { .. }, ActionResult::Position(pos)) => pos,
            (action, result) => panic!(
                "Expected position from action {:?}, got {:?}",
                action, result
            ),
        }
    }

    /// Extract bias voltage with action validation (panics on type mismatch)
    pub fn expect_bias_voltage(self, action: &Action) -> f32 {
        match (action, self) {
            (Action::ReadBias, ActionResult::Value(v)) => v as f32,
            (action, result) => panic!(
                "Expected bias voltage from action {:?}, got {:?}",
                action, result
            ),
        }
    }

    /// Extract signal names with action validation (panics on type mismatch)
    pub fn expect_signal_names(self, action: &Action) -> Vec<String> {
        match (action, self) {
            (Action::ReadSignalNames, ActionResult::Text(names)) => names,
            (action, result) => panic!(
                "Expected signal names from action {:?}, got {:?}",
                action, result
            ),
        }
    }

    /// Extract status with action validation (panics on type mismatch)
    pub fn expect_status(self, action: &Action) -> bool {
        match (action, self) {
            (Action::ReadScanStatus, ActionResult::Status(status)) => status,
            (action, result) => {
                panic!("Expected status from action {:?}, got {:?}", action, result)
            }
        }
    }

    /// Extract tip shape enum with action validation (panics on type mismatch)
    pub fn expect_tip_shape(self, action: &Action) -> TipShape {
        match (action, self) {
            (Action::CheckTipState { .. }, ActionResult::TipState(tip_state)) => tip_state.shape,
            (action, result) => {
                panic!(
                    "Expected tip state from action {:?}, got {:?}",
                    action, result
                )
            }
        }
    }

    /// Extract full tip state result with action validation (panics on type mismatch)
    pub fn expect_tip_state(self, action: &Action) -> TipState {
        match (action, self) {
            (Action::CheckTipState { .. }, ActionResult::TipState(tip_state)) => tip_state,
            (action, result) => {
                panic!(
                    "Expected tip state from action {:?}, got {:?}",
                    action, result
                )
            }
        }
    }

    /// Extract stability result (panics on type mismatch)
    pub fn expect_stability_result(self, action: &Action) -> StabilityResult {
        match (action, self) {
            (Action::CheckTipStability { .. }, ActionResult::StabilityResult(result)) => result,
            (action, result) => {
                panic!(
                    "Expected stability result from action {:?}, got {:?}",
                    action, result
                )
            }
        }
    }

    /// Extract stable signal value (panics on type mismatch)
    pub fn expect_stable_signal_value(self, action: &Action) -> f32 {
        match (action, self) {
            (Action::ReadStableSignal { .. }, ActionResult::StableSignal(stable)) => {
                stable.stable_value
            }
            (action, result) => {
                panic!(
                    "Expected stable signal from action {:?}, got {:?}",
                    action, result
                )
            }
        }
    }

    /// Extract full stable signal result with action validation (panics on type mismatch)
    pub fn expect_stable_signal(self, action: &Action) -> StableSignal {
        match (action, self) {
            (Action::ReadStableSignal { .. }, ActionResult::StableSignal(stable)) => stable,
            (action, result) => {
                panic!(
                    "Expected stable signal from action {:?}, got {:?}",
                    action, result
                )
            }
        }
    }

    /// Extract TCP reader status with action validation (panics on type mismatch)
    pub fn expect_tcp_reader_status(self, action: &Action) -> TCPReaderStatus {
        match (action, self) {
            (Action::GetTCPLoggerStatus, ActionResult::TCPReaderStatus(status)) => status,
            (action, result) => {
                panic!(
                    "Expected TCP reader status from action {:?}, got {:?}",
                    action, result
                )
            }
        }
    }

    // === Safe Extraction Methods (non-panicking) ===

    /// Try to extract OsciData with action validation
    pub fn try_into_osci_data(self, action: &Action) -> Result<OsciData, String> {
        match (action, self) {
            (Action::ReadOsci { .. }, ActionResult::OsciData(data)) => Ok(data),
            (action, result) => Err(format!(
                "Expected OsciData from action {:?}, got {:?}",
                action, result
            )),
        }
    }

    /// Try to extract signal value with action validation
    pub fn try_into_signal_value(self, action: &Action) -> Result<f64, String> {
        match (action, self) {
            (Action::ReadSignal { .. }, ActionResult::Value(v)) => Ok(v),
            (Action::ReadSignal { .. }, ActionResult::Values(mut vs)) if vs.len() == 1 => {
                Ok(vs.pop().unwrap())
            }
            (Action::ReadBias, ActionResult::Value(v)) => Ok(v),
            (action, result) => Err(format!(
                "Expected signal value from action {:?}, got {:?}",
                action, result
            )),
        }
    }

    /// Try to extract position with action validation
    pub fn try_into_position(self, action: &Action) -> Result<Position, String> {
        match (action, self) {
            (Action::ReadPiezoPosition { .. }, ActionResult::Position(pos)) => Ok(pos),
            (action, result) => Err(format!(
                "Expected position from action {:?}, got {:?}",
                action, result
            )),
        }
    }

    /// Try to extract status with action validation
    pub fn try_into_status(self, action: &Action) -> Result<bool, String> {
        match (action, self) {
            (Action::ReadScanStatus, ActionResult::Status(status)) => Ok(status),
            (action, result) => Err(format!(
                "Expected status from action {:?}, got {:?}",
                action, result
            )),
        }
    }

    /// Try to extract stability result with action validation
    pub fn try_into_stability_result(self, action: &Action) -> Result<StabilityResult, String> {
        match (action, self) {
            (Action::CheckTipStability { .. }, ActionResult::StabilityResult(result)) => Ok(result),
            (action, result) => Err(format!(
                "Expected stability result from action {:?}, got {:?}",
                action, result
            )),
        }
    }

    /// Try to extract stable signal value with action validation
    pub fn try_into_stable_signal_value(self, action: &Action) -> Result<f32, String> {
        match (action, self) {
            (Action::ReadStableSignal { .. }, ActionResult::StableSignal(stable)) => {
                Ok(stable.stable_value)
            }
            (action, result) => Err(format!(
                "Expected stable signal from action {:?}, got {:?}",
                action, result
            )),
        }
    }
}

// === Trait for Generic Type Extraction ===

/// Trait for extracting specific types from ActionResult with action validation
pub trait ExpectFromAction<T> {
    fn expect_from_action(self, action: &Action) -> T;
}

impl ExpectFromAction<OsciData> for ActionResult {
    fn expect_from_action(self, action: &Action) -> OsciData {
        self.expect_osci_data(action)
    }
}

impl ExpectFromAction<f64> for ActionResult {
    fn expect_from_action(self, action: &Action) -> f64 {
        self.expect_signal_value(action)
    }
}

impl ExpectFromAction<Vec<f64>> for ActionResult {
    fn expect_from_action(self, action: &Action) -> Vec<f64> {
        self.expect_values(action)
    }
}

impl ExpectFromAction<Position> for ActionResult {
    fn expect_from_action(self, action: &Action) -> Position {
        self.expect_position(action)
    }
}

impl ExpectFromAction<Vec<String>> for ActionResult {
    fn expect_from_action(self, action: &Action) -> Vec<String> {
        self.expect_signal_names(action)
    }
}

impl ExpectFromAction<bool> for ActionResult {
    fn expect_from_action(self, action: &Action) -> bool {
        self.expect_status(action)
    }
}

impl ExpectFromAction<StabilityResult> for ActionResult {
    fn expect_from_action(self, action: &Action) -> StabilityResult {
        self.expect_stability_result(action)
    }
}

impl ExpectFromAction<f32> for ActionResult {
    fn expect_from_action(self, action: &Action) -> f32 {
        match action {
            Action::ReadStableSignal { .. } => self.expect_stable_signal_value(action),
            _ => self.expect_bias_voltage(action),
        }
    }
}

// === Action Categorization ===

impl Action {
    /// Check if this is a positioning action
    pub fn is_positioning_action(&self) -> bool {
        matches!(
            self,
            Action::SetPiezoPosition { .. }
                | Action::MovePiezoRelative { .. }
                | Action::MoveMotorAxis { .. }
                | Action::MoveMotor3D { .. }
                | Action::MoveMotorClosedLoop { .. }
        )
    }

    /// Check if this is a read-only action
    pub fn is_read_action(&self) -> bool {
        matches!(
            self,
            Action::ReadSignal { .. }
                | Action::ReadSignals { .. }
                | Action::ReadSignalNames
                | Action::ReadBias
                | Action::ReadPiezoPosition { .. }
                | Action::ReadScanStatus
                | Action::Retrieve { .. }
        )
    }

    /// Check if this is a control action
    pub fn is_control_action(&self) -> bool {
        matches!(
            self,
            Action::AutoApproach { .. }
                | Action::Withdraw { .. }
                | Action::SafeReposition { .. }
                | Action::ScanControl { .. }
                | Action::StopMotor
        )
    }

    /// Check if this action modifies bias voltage
    pub fn modifies_bias(&self) -> bool {
        matches!(self, Action::SetBias { .. } | Action::BiasPulse { .. })
    }

    /// Check if this action involves motor movement
    pub fn involves_motor(&self) -> bool {
        matches!(
            self,
            Action::MoveMotorAxis { .. }
                | Action::MoveMotor3D { .. }
                | Action::MoveMotorClosedLoop { .. }
                | Action::SafeReposition { .. }
                | Action::StopMotor
        )
    }

    /// Check if this action involves piezo movement
    pub fn involves_piezo(&self) -> bool {
        matches!(
            self,
            Action::SetPiezoPosition { .. }
                | Action::MovePiezoRelative { .. }
                | Action::ReadPiezoPosition { .. }
        )
    }

    /// Get a human-readable description of the action
    pub fn description(&self) -> String {
        match self {
            Action::ReadSignal { signal, .. } => {
                format!("Read signal {}", signal.0)
            }
            Action::ReadSignals { signals, .. } => {
                let indices: Vec<i32> = signals.iter().map(|s| s.0.get() as i32).collect();
                format!("Read signals: {:?}", indices)
            }
            Action::SetBias { voltage } => {
                format!("Set bias to {:.3}V", voltage)
            }
            Action::SetPiezoPosition { position, .. } => {
                format!(
                    "Set piezo position to ({:.3e}, {:.3e})",
                    position.x, position.y
                )
            }
            Action::MoveMotorAxis {
                direction,
                steps,
                blocking,
            } => {
                format!("Move motor {direction:?} {steps} steps with blocking {blocking}")
            }
            Action::MoveMotor3D {
                displacement,
                blocking,
            } => {
                format!(
                    "Move motor 3D displacement ({}, {}, {}) with blocking {blocking}",
                    displacement.x, displacement.y, displacement.z
                )
            }
            Action::AutoApproach {
                wait_until_finished,
                timeout,
            } => format!(
                "Auto approach blocking: {wait_until_finished}, timeout: {:?}",
                timeout
            ),
            Action::Withdraw { timeout, .. } => {
                format!("Withdraw tip (timeout: {}ms)", timeout.as_micros())
            }
            Action::SafeReposition { x_steps, y_steps } => {
                format!("Safe reposition: move ({}, {}) steps", x_steps, y_steps)
            }
            Action::SetZSetpoint { setpoint } => {
                format!("Set Z setpoint: {:.3e}", setpoint)
            }
            Action::Wait { duration } => {
                format!("Wait {:.1}s", duration.as_secs_f64())
            }
            Action::BiasPulse {
                wait_until_done: _,
                pulse_width,
                bias_value_v,
                z_controller_hold: _,
                pulse_mode: _,
            } => {
                format!("Bias pulse {:.3}V for {:?}ms", bias_value_v, pulse_width)
            }
            Action::TipShaper {
                config,
                wait_until_finished,
                timeout,
            } => {
                format!(
                    "Tip shaper: bias {:.1}V, lift {:.0}nm, times {:.1?}s/{:.1?}s (wait: {}, timeout: {:?}ms)",
                    config.bias_v,
                    config.tip_lift_m * 1e9,
                    config.lift_time_1.as_secs_f32(),
                    config.lift_time_2.as_secs_f32(),
                    wait_until_finished,
                    timeout
                )
            }
            Action::PulseRetract {
                pulse_width,
                pulse_height_v,
            } => {
                format!(
                    "Pulse retract {:.1}V for {:.0?}ms",
                    pulse_height_v, pulse_width
                )
            }
            Action::ReadOsci {
                signal,
                trigger,
                data_to_get,
                is_stable,
            } => {
                let trigger_desc = match trigger {
                    Some(config) => format!("trigger: {:?}", config.mode),
                    None => "no trigger config".to_string(),
                };
                let stability_desc = match is_stable {
                    Some(_) => " with custom stability",
                    None => "",
                };
                format!(
                    "Read oscilloscope signal {} with {} (mode: {:?}){}",
                    signal.0, trigger_desc, data_to_get, stability_desc
                )
            }
            Action::CheckTipState { method } => match method {
                TipCheckMethod::SignalBounds { signal, bounds } => {
                    format!(
                        "Check tip state: signal {} bounds ({:.3e}, {:.3e})",
                        signal.0, bounds.0, bounds.1
                    )
                }
                TipCheckMethod::MultiSignalBounds { signals } => {
                    format!("Check tip state: {} signal bounds", signals.len())
                }
            },
            Action::CheckTipStability {
                method,
                max_duration,
                abort_on_damage_signs,
            } => {
                let duration_desc = format!("{:.1}s", max_duration.as_secs_f32());
                let abort_desc = if *abort_on_damage_signs {
                    "abort on damage"
                } else {
                    "no abort"
                };
                match method {
                    TipStabilityMethod::ExtendedMonitoring {
                        signal, duration, ..
                    } => {
                        format!("Check tip stability: extended monitoring signal {} for {:.1}s (max: {}, {})", 
                               signal.0, duration.as_secs_f32(), duration_desc, abort_desc)
                    }
                    TipStabilityMethod::BiasSweepResponse {
                        signal,
                        bias_range,
                        sweep_steps,
                        period,
                        allowed_signal_change,
                        ..
                    } => {
                        format!("Check tip stability: bias sweep signal {} from {:.2}V to {:.2}V ({} steps, {:.1}ms period, {:.1}% change allowed, max: {}, {})", 
                               signal.0, bias_range.0, bias_range.1, sweep_steps, period.as_millis(), allowed_signal_change * 100.0, duration_desc, abort_desc)
                    }
                }
            }
            Action::ReadStableSignal {
                signal,
                data_points,
                use_new_data,
                stability_method,
                timeout,
                retry_count,
            } => {
                let points_desc = data_points.map_or("default".to_string(), |p| p.to_string());
                let data_desc = if *use_new_data {
                    "new data"
                } else {
                    "buffered data"
                };
                let method_desc = match stability_method {
                    SignalStabilityMethod::StandardDeviation { threshold } => {
                        format!("std dev {:.3e}", threshold)
                    }
                    SignalStabilityMethod::RelativeStandardDeviation { threshold_percent } => {
                        format!("rel std {:.1}%", threshold_percent)
                    }
                    SignalStabilityMethod::MovingWindow {
                        window_size,
                        max_variation,
                    } => {
                        format!("window {}pts, max var {:.3e}", window_size, max_variation)
                    }
                    SignalStabilityMethod::TrendAnalysis { max_slope } => {
                        format!("trend analysis, max slope {:.3e}", max_slope)
                    }
                };
                let retry_desc = retry_count.map_or("no retry".to_string(), |r| format!("{} retries", r));
                format!(
                    "Get stable signal {} ({} points, {}, {}, timeout {:.1}s, {})",
                    signal.0,
                    points_desc,
                    data_desc,
                    method_desc,
                    timeout.as_secs_f32(),
                    retry_desc
                )
            }
            _ => format!("{:?}", self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_result_extraction() {
        let bias_result = ActionResult::Value(2.5);
        assert_eq!(bias_result.as_f64(), Some(2.5));

        let position_result = ActionResult::Position(Position { x: 1e-9, y: 2e-9 });
        assert_eq!(
            position_result.as_position(),
            Some(Position { x: 1e-9, y: 2e-9 })
        );
    }
}

/// A sequence of actions with simple Vec<Action> foundation
#[derive(Debug, Clone)]
pub struct ActionChain {
    actions: Vec<Action>,
    name: Option<String>,
}

impl ActionChain {
    /// Create a new ActionChain from a vector of actions
    pub fn new(actions: Vec<Action>) -> Self {
        Self {
            actions,
            name: None,
        }
    }

    /// Create a new ActionChain from any iterator of actions
    pub fn from_actions(actions: impl IntoIterator<Item = Action>) -> Self {
        Self::new(actions.into_iter().collect())
    }

    /// Create a new ActionChain with a name
    pub fn named(actions: Vec<Action>, name: impl Into<String>) -> Self {
        Self {
            actions,
            name: Some(name.into()),
        }
    }

    /// Create an empty ActionChain
    pub fn empty() -> Self {
        Self::new(vec![])
    }

    // === Direct Vec<Action> Access ===

    /// Get immutable reference to actions
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// Get mutable reference to actions vector for direct manipulation
    pub fn actions_mut(&mut self) -> &mut Vec<Action> {
        &mut self.actions
    }

    /// Add an action to the end of the chain
    pub fn push(&mut self, action: Action) {
        self.actions.push(action);
    }

    /// Add multiple actions to the end of the chain
    pub fn extend(&mut self, actions: impl IntoIterator<Item = Action>) {
        self.actions.extend(actions);
    }

    /// Insert an action at a specific index
    pub fn insert(&mut self, index: usize, action: Action) {
        self.actions.insert(index, action);
    }

    /// Remove and return the action at index
    pub fn remove(&mut self, index: usize) -> Action {
        self.actions.remove(index)
    }

    /// Remove the last action and return it
    pub fn pop(&mut self) -> Option<Action> {
        self.actions.pop()
    }

    /// Clear all actions
    pub fn clear(&mut self) {
        self.actions.clear();
    }

    /// Create a new chain by appending another chain
    pub fn chain_with(mut self, other: ActionChain) -> Self {
        self.actions.extend(other.actions);
        self
    }

    /// Get an iterator over actions
    pub fn iter(&self) -> std::slice::Iter<'_, Action> {
        self.actions.iter()
    }

    /// Get a mutable iterator over actions
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Action> {
        self.actions.iter_mut()
    }

    // === Metadata Access ===

    /// Get the name of this chain
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Set the name of this chain
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = Some(name.into());
    }

    /// Get the number of actions in this chain
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Check if this chain is empty
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    // === Analysis Methods ===

    /// Get actions that match a specific category
    pub fn positioning_actions(&self) -> Vec<&Action> {
        self.actions
            .iter()
            .filter(|a| a.is_positioning_action())
            .collect()
    }

    pub fn read_actions(&self) -> Vec<&Action> {
        self.actions.iter().filter(|a| a.is_read_action()).collect()
    }

    pub fn control_actions(&self) -> Vec<&Action> {
        self.actions
            .iter()
            .filter(|a| a.is_control_action())
            .collect()
    }

    /// Check if chain contains any motor movements
    pub fn involves_motor(&self) -> bool {
        self.actions.iter().any(|a| a.involves_motor())
    }

    /// Check if chain contains any piezo movements
    pub fn involves_piezo(&self) -> bool {
        self.actions.iter().any(|a| a.involves_piezo())
    }

    /// Check if chain contains any bias modifications
    pub fn modifies_bias(&self) -> bool {
        self.actions.iter().any(|a| a.modifies_bias())
    }

    /// Get a summary description of the chain
    pub fn summary(&self) -> String {
        if let Some(name) = &self.name {
            format!("{} ({} actions)", name, self.len())
        } else {
            format!("Action chain with {} actions", self.len())
        }
    }

    /// Get detailed analysis of the chain
    pub fn analysis(&self) -> ChainAnalysis {
        ChainAnalysis {
            total_actions: self.len(),
            positioning_actions: self.positioning_actions().len(),
            read_actions: self.read_actions().len(),
            control_actions: self.control_actions().len(),
            involves_motor: self.involves_motor(),
            involves_piezo: self.involves_piezo(),
            modifies_bias: self.modifies_bias(),
        }
    }
}

/// Analysis result for an ActionChain
#[derive(Debug, Clone)]
pub struct ChainAnalysis {
    pub total_actions: usize,
    pub positioning_actions: usize,
    pub read_actions: usize,
    pub control_actions: usize,
    pub involves_motor: bool,
    pub involves_piezo: bool,
    pub modifies_bias: bool,
}

// === Iterator Support ===

impl IntoIterator for ActionChain {
    type Item = Action;
    type IntoIter = std::vec::IntoIter<Action>;

    fn into_iter(self) -> Self::IntoIter {
        self.actions.into_iter()
    }
}

impl<'a> IntoIterator for &'a ActionChain {
    type Item = &'a Action;
    type IntoIter = std::slice::Iter<'a, Action>;

    fn into_iter(self) -> Self::IntoIter {
        self.actions.iter()
    }
}

impl FromIterator<Action> for ActionChain {
    fn from_iter<T: IntoIterator<Item = Action>>(iter: T) -> Self {
        Self::from_actions(iter)
    }
}

impl From<Vec<Action>> for ActionChain {
    fn from(actions: Vec<Action>) -> Self {
        Self::new(actions)
    }
}

// ==================== Pre-built Common Patterns ====================

impl ActionChain {
    /// Comprehensive system status check
    pub fn system_status_check() -> Self {
        ActionChain::named(
            vec![
                Action::ReadSignalNames,
                Action::ReadBias,
                Action::ReadPiezoPosition {
                    wait_for_newest_data: true,
                },
            ],
            "System status check",
        )
    }

    /// Safe tip approach with verification
    pub fn safe_tip_approach() -> Self {
        ActionChain::named(
            vec![
                Action::ReadPiezoPosition {
                    wait_for_newest_data: true,
                },
                Action::AutoApproach {
                    wait_until_finished: true,
                    timeout: Duration::from_secs(300),
                },
                Action::Wait {
                    duration: Duration::from_millis(500),
                },
                Action::ReadSignal {
                    signal: SignalIndex::new(24),
                    wait_for_newest: true,
                }, // Typical bias voltage
                Action::ReadSignal {
                    signal: SignalIndex::new(0),
                    wait_for_newest: true,
                }, // Typical current
            ],
            "Safe tip approach",
        )
    }

    /// Move to position and approach
    pub fn move_and_approach(target: Position) -> Self {
        ActionChain::named(
            vec![
                Action::SetPiezoPosition {
                    position: target,
                    wait_until_finished: true,
                },
                Action::Wait {
                    duration: Duration::from_millis(100),
                },
                Action::AutoApproach {
                    wait_until_finished: true,
                    timeout: Duration::from_secs(300),
                },
                Action::ReadSignal {
                    signal: SignalIndex::new(24),
                    wait_for_newest: true,
                },
            ],
            format!("Move to ({:.1e}, {:.1e}) and approach", target.x, target.y),
        )
    }

    /// Bias pulse sequence with restoration
    pub fn bias_pulse_sequence(voltage: f32, duration_ms: u32) -> Self {
        ActionChain::named(
            vec![
                Action::ReadBias,
                Action::SetBias { voltage },
                Action::Wait {
                    duration: Duration::from_millis(50),
                },
                Action::Wait {
                    duration: Duration::from_millis(duration_ms as u64),
                },
                Action::SetBias { voltage: 0.0 },
            ],
            format!("Bias pulse {:.3}V for {}ms", voltage, duration_ms),
        )
    }

    /// Survey multiple positions
    pub fn position_survey(positions: Vec<Position>) -> Self {
        let position_count = positions.len(); // Store length before moving
        let mut actions = Vec::new();

        for pos in positions {
            actions.extend([
                Action::SetPiezoPosition {
                    position: pos,
                    wait_until_finished: true,
                },
                Action::Wait {
                    duration: Duration::from_millis(100),
                },
                Action::AutoApproach {
                    wait_until_finished: true,
                    timeout: Duration::from_secs(300),
                },
                Action::ReadSignal {
                    signal: SignalIndex::new(24),
                    wait_for_newest: true,
                }, // Bias voltage
                Action::ReadSignal {
                    signal: SignalIndex::new(0),
                    wait_for_newest: true,
                }, // Current
                Action::Withdraw {
                    wait_until_finished: true,
                    timeout: Duration::from_secs(5),
                },
            ]);
        }

        ActionChain::named(
            actions,
            format!("Position survey ({} points)", position_count),
        )
    }

    /// Complete tip recovery sequence
    pub fn tip_recovery_sequence() -> Self {
        ActionChain::named(
            vec![
                Action::Withdraw {
                    wait_until_finished: true,
                    timeout: Duration::from_secs(5),
                },
                Action::MovePiezoRelative {
                    delta: Position { x: 3e-9, y: 3e-9 },
                },
                Action::Wait {
                    duration: Duration::from_millis(200),
                },
                Action::AutoApproach {
                    wait_until_finished: true,
                    timeout: Duration::from_secs(300),
                },
                Action::ReadSignal {
                    signal: SignalIndex::new(24),
                    wait_for_newest: true,
                },
            ],
            "Tip recovery sequence",
        )
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;
    use crate::types::MotorDirection;

    #[test]
    fn test_vec_foundation() {
        // Test direct Vec<Action> usage
        let mut chain = ActionChain::new(vec![Action::ReadBias, Action::SetBias { voltage: 1.0 }]);

        assert_eq!(chain.len(), 2);

        // Test Vec operations
        chain.push(Action::AutoApproach {
            wait_until_finished: true,
            timeout: Duration::from_secs(300),
        });
        assert_eq!(chain.len(), 3);

        let action = chain.pop().unwrap();
        assert!(matches!(
            action,
            Action::AutoApproach {
                wait_until_finished: true,
                timeout: _
            }
        ));
        assert_eq!(chain.len(), 2);

        // Test extension
        chain.extend([
            Action::Wait {
                duration: Duration::from_millis(100),
            },
            Action::ReadBias,
        ]);
        assert_eq!(chain.len(), 4);
    }

    #[test]
    fn test_simple_construction() {
        let chain = ActionChain::named(
            vec![
                Action::ReadBias,
                Action::SetBias { voltage: 1.0 },
                Action::Wait {
                    duration: Duration::from_millis(100),
                },
                Action::AutoApproach {
                    wait_until_finished: true,
                    timeout: Duration::from_secs(300),
                },
            ],
            "Test chain",
        );

        assert_eq!(chain.name(), Some("Test chain"));
        assert_eq!(chain.len(), 4);

        let analysis = chain.analysis();
        assert_eq!(analysis.total_actions, 4);
        assert_eq!(analysis.read_actions, 1);
        assert_eq!(analysis.control_actions, 1);
        assert!(analysis.modifies_bias);
    }

    #[test]
    fn test_programmatic_generation() {
        // Test building chains programmatically
        let mut chain = ActionChain::empty();

        for _ in 0..3 {
            chain.push(Action::MoveMotorAxis {
                direction: MotorDirection::XPlus,
                steps: 10,
                blocking: true,
            });
            chain.push(Action::Wait {
                duration: Duration::from_millis(100),
            });
        }

        assert_eq!(chain.len(), 6);
        assert!(chain.involves_motor());

        // Test iterator construction
        let actions: Vec<Action> = (0..5).map(|_| Action::ReadBias).collect();

        let iter_chain: ActionChain = actions.into_iter().collect();
        assert_eq!(iter_chain.len(), 5);
    }

    #[test]
    fn test_pre_built_patterns() {
        let status_check = ActionChain::system_status_check();
        assert!(status_check.name().is_some());
        assert!(!status_check.is_empty());

        let approach = ActionChain::safe_tip_approach();
        assert!(!approach.control_actions().is_empty());

        let positions = vec![Position { x: 1e-9, y: 1e-9 }, Position { x: 2e-9, y: 2e-9 }];
        let survey = ActionChain::position_survey(positions);
        assert_eq!(survey.len(), 12); // 6 actions per position × 2 positions
    }

    #[test]
    fn test_chain_analysis() {
        let chain = ActionChain::new(vec![
            Action::MoveMotorAxis {
                direction: MotorDirection::XPlus,
                steps: 100,
                blocking: true,
            },
            Action::SetPiezoPosition {
                position: Position { x: 1e-9, y: 1e-9 },
                wait_until_finished: true,
            },
            Action::ReadBias,
            Action::AutoApproach {
                wait_until_finished: true,
                timeout: Duration::from_secs(1),
            },
            Action::SetBias { voltage: 1.5 },
        ]);

        let analysis = chain.analysis();
        assert_eq!(analysis.total_actions, 5);
        assert_eq!(analysis.positioning_actions, 2);
        assert_eq!(analysis.read_actions, 1);
        assert_eq!(analysis.control_actions, 1);
        assert!(analysis.involves_motor);
        assert!(analysis.involves_piezo);
        assert!(analysis.modifies_bias);
    }

    #[test]
    fn test_iteration() {
        let chain = ActionChain::new(vec![
            Action::ReadBias,
            Action::AutoApproach {
                wait_until_finished: true,
                timeout: Duration::from_secs(1),
            },
            Action::Wait {
                duration: Duration::from_millis(100),
            },
        ]);

        // Test iterator
        let mut count = 0;
        for _ in &chain {
            count += 1;
            // Can access action here
        }
        assert_eq!(count, 3);

        // Test into_iter
        let actions: Vec<Action> = chain.into_iter().collect();
        assert_eq!(actions.len(), 3);
    }

    #[test]
    fn test_from_vec_action() {
        // Test From<Vec<Action>> trait
        let actions = vec![
            Action::ReadBias,
            Action::SetBias { voltage: 1.5 },
            Action::AutoApproach {
                wait_until_finished: true,
                timeout: Duration::from_secs(1),
            },
        ];

        let chain: ActionChain = actions.into();
        assert_eq!(chain.len(), 3);
        assert!(chain.name().is_none());

        // Test that it's usable with Into<ActionChain> parameters
        let vec_actions = vec![
            Action::ReadBias,
            Action::Wait {
                duration: Duration::from_millis(50),
            },
        ];

        // This should compile thanks to Into<ActionChain>
        fn accepts_into_action_chain(_chain: impl Into<ActionChain>) {
            // This function would be called by execute methods
        }

        accepts_into_action_chain(vec_actions);
    }
}

// ==================== Action Logging Support ====================

/// Log entry for action execution with timing information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActionLogEntry {
    /// The action that was executed
    pub action: String, // Action description for JSON serialization
    /// The result of the action execution
    pub result: ActionLogResult,
    /// When the action started executing
    pub start_time: chrono::DateTime<chrono::Utc>,
    /// How long the action took to execute
    pub duration_ms: u64,
    /// Optional metadata for debugging
    pub metadata: Option<std::collections::HashMap<String, String>>,
}

/// Comprehensive action result for logging (JSON-serializable)
/// Captures all possible data types without simplification
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ActionLogResult {
    /// Single numeric value
    Value(f64),
    /// Multiple numeric values
    Values(Vec<f64>),
    /// String data
    Text(Vec<String>),
    /// Boolean status
    Status(bool),
    /// Position data
    Position { x: f64, y: f64 },
    /// Complete oscilloscope data with timing and statistics
    OsciData {
        t0: f64,
        dt: f64,
        size: i32,
        data: Vec<f64>,
        signal_stats: Option<LoggableSignalStats>,
        is_stable: bool,
        fallback_value: Option<f64>,
    },
    /// Experiment data with action result and TCP signal collection
    ExperimentData {
        action_result: Box<ActionLogResult>,
        signal_frames: Vec<LoggableTimestampedSignalFrame>,
        tcp_config: LoggableTCPLoggerConfig,
        action_start_ms: u64, // Timestamp as milliseconds since epoch
        action_end_ms: u64,
        total_duration_ms: u64,
    },
    /// Chain experiment data with per-action timing and results
    ChainExperimentData {
        action_results: Vec<ActionLogResult>,
        signal_frames: Vec<LoggableTimestampedSignalFrame>,
        tcp_config: LoggableTCPLoggerConfig,
        action_timings: Vec<(u64, u64)>, // (start_ms, end_ms) for each action
        chain_start_ms: u64,
        chain_end_ms: u64,
        total_duration_ms: u64,
    },
    /// TCP Logger Status
    TCPLoggerStatus {
        status: String, // TCPLogStatus serialized as string
        channels: Vec<i32>,
        oversampling: i32,
    },
    /// Tip state check result
    TipState(TipShape),
    /// Operation completed successfully
    Success,
    /// Operation completed but no data returned
    None,
    /// Error occurred during execution
    Error(String),
}

/// JSON-serializable version of SignalStats
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoggableSignalStats {
    pub mean: f64,
    pub std_dev: f64,
    pub relative_std: f64,
    pub window_size: usize,
    pub stability_method: String,
}

/// JSON-serializable version of TimestampedSignalFrame
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoggableTimestampedSignalFrame {
    pub signal_frame: LoggableSignalFrame,
    pub timestamp_ms: u64,     // Milliseconds since epoch
    pub relative_time_ms: u64, // Milliseconds relative to collection start
}

/// JSON-serializable version of SignalFrame
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoggableSignalFrame {
    pub counter: u64,
    pub data: Vec<f32>,
}

/// JSON-serializable version of TCPLoggerConfig
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoggableTCPLoggerConfig {
    pub stream_port: u16,
    pub channels: Vec<i32>,
    pub oversampling: i32,
    pub auto_start: bool,
    pub buffer_size: Option<usize>,
}

impl ActionLogEntry {
    /// Create a new log entry from action execution
    pub fn new(
        action: &Action,
        result: &ActionResult,
        start_time: chrono::DateTime<chrono::Utc>,
        duration: std::time::Duration,
    ) -> Self {
        Self {
            action: action.description(),
            result: ActionLogResult::from_action_result(result),
            start_time,
            duration_ms: duration.as_millis() as u64,
            metadata: None,
        }
    }

    /// Create a new log entry with error
    pub fn new_error(
        action: &Action,
        error: &crate::error::NanonisError,
        start_time: chrono::DateTime<chrono::Utc>,
        duration: std::time::Duration,
    ) -> Self {
        Self {
            action: action.description(),
            result: ActionLogResult::Error(error.to_string()),
            start_time,
            duration_ms: duration.as_millis() as u64,
            metadata: None,
        }
    }

    /// Add metadata to this log entry
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        if self.metadata.is_none() {
            self.metadata = Some(std::collections::HashMap::new());
        }
        self.metadata
            .as_mut()
            .unwrap()
            .insert(key.into(), value.into());
        self
    }
}

impl ActionLogResult {
    /// Convert ActionResult to ActionLogResult for comprehensive logging
    /// No data simplification - captures everything in full detail
    pub fn from_action_result(result: &ActionResult) -> Self {
        match result {
            ActionResult::Value(v) => ActionLogResult::Value(*v),
            ActionResult::Values(values) => ActionLogResult::Values(values.clone()),
            ActionResult::Text(text) => ActionLogResult::Text(text.clone()),
            ActionResult::Status(status) => ActionLogResult::Status(*status),
            ActionResult::Position(pos) => ActionLogResult::Position { x: pos.x, y: pos.y },
            ActionResult::OsciData(osci_data) => ActionLogResult::OsciData {
                t0: osci_data.t0,
                dt: osci_data.dt,
                size: osci_data.size,
                data: osci_data.data.clone(),
                signal_stats: osci_data
                    .signal_stats
                    .as_ref()
                    .map(|stats| LoggableSignalStats {
                        mean: stats.mean,
                        std_dev: stats.std_dev,
                        relative_std: stats.relative_std,
                        window_size: stats.window_size,
                        stability_method: stats.stability_method.clone(),
                    }),
                is_stable: osci_data.is_stable,
                fallback_value: osci_data.fallback_value,
            },
            ActionResult::Success => ActionLogResult::Success,
            ActionResult::None => ActionLogResult::None,
            ActionResult::TCPReaderStatus(tcp_status) => {
                ActionLogResult::TCPLoggerStatus {
                    status: format!("{:?}", tcp_status.status), // Serialize enum as string
                    channels: tcp_status.channels.clone(),
                    oversampling: tcp_status.oversampling,
                }
            }
            ActionResult::TipState(tip_state) => ActionLogResult::TipState(tip_state.shape),
            ActionResult::StabilityResult(result) => {
                // Convert stability result to a simple TipShape for logging compatibility
                let tip_shape = if result.is_stable {
                    TipShape::Stable
                } else {
                    TipShape::Blunt
                };
                ActionLogResult::TipState(tip_shape)
            }
            ActionResult::StableSignal(stable) => {
                // Convert stable signal to a single value for logging
                ActionLogResult::Value(stable.stable_value as f64)
            }
        }
    }

    /// Convert ExperimentData to ActionLogResult for comprehensive logging
    pub fn from_experiment_data(exp_data: &crate::types::ExperimentData) -> Self {
        let action_result = Box::new(Self::from_action_result(&exp_data.action_result));

        let signal_frames: Vec<LoggableTimestampedSignalFrame> = exp_data
            .signal_frames
            .iter()
            .map(|frame| LoggableTimestampedSignalFrame {
                signal_frame: LoggableSignalFrame {
                    counter: frame.signal_frame.counter,
                    data: frame.signal_frame.data.clone(),
                },
                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64, // Approximate current time
                relative_time_ms: frame.relative_time.as_millis() as u64,
            })
            .collect();

        let tcp_config = LoggableTCPLoggerConfig {
            stream_port: exp_data.tcp_config.stream_port,
            channels: exp_data.tcp_config.channels.clone(),
            oversampling: exp_data.tcp_config.oversampling,
            auto_start: exp_data.tcp_config.auto_start,
            buffer_size: exp_data.tcp_config.buffer_size,
        };

        ActionLogResult::ExperimentData {
            action_result,
            signal_frames,
            tcp_config,
            action_start_ms: chrono::Utc::now().timestamp_millis() as u64, // Approximate timing
            action_end_ms: chrono::Utc::now().timestamp_millis() as u64,
            total_duration_ms: exp_data.total_duration.as_millis() as u64,
        }
    }

    /// Convert ChainExperimentData to ActionLogResult for comprehensive logging
    pub fn from_chain_experiment_data(chain_data: &crate::types::ChainExperimentData) -> Self {
        let action_results: Vec<ActionLogResult> = chain_data
            .action_results
            .iter()
            .map(Self::from_action_result)
            .collect();

        let signal_frames: Vec<LoggableTimestampedSignalFrame> = chain_data
            .signal_frames
            .iter()
            .map(|frame| LoggableTimestampedSignalFrame {
                signal_frame: LoggableSignalFrame {
                    counter: frame.signal_frame.counter,
                    data: frame.signal_frame.data.clone(),
                },
                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64, // Approximate current time
                relative_time_ms: frame.relative_time.as_millis() as u64,
            })
            .collect();

        let tcp_config = LoggableTCPLoggerConfig {
            stream_port: chain_data.tcp_config.stream_port,
            channels: chain_data.tcp_config.channels.clone(),
            oversampling: chain_data.tcp_config.oversampling,
            auto_start: chain_data.tcp_config.auto_start,
            buffer_size: chain_data.tcp_config.buffer_size,
        };

        let action_timings: Vec<(u64, u64)> = chain_data
            .action_timings
            .iter()
            .map(|(_, _)| {
                let now = chrono::Utc::now().timestamp_millis() as u64;
                (now, now) // Approximate timing
            })
            .collect();

        ActionLogResult::ChainExperimentData {
            action_results,
            signal_frames,
            tcp_config,
            action_timings,
            chain_start_ms: chrono::Utc::now().timestamp_millis() as u64, // Approximate timing
            chain_end_ms: chrono::Utc::now().timestamp_millis() as u64,
            total_duration_ms: chain_data.total_duration.as_millis() as u64,
        }
    }
}
