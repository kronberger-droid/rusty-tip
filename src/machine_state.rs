//! Machine state tracking for safe action execution.
//!
//! Every action declares what machine state it expects and what state it
//! produces.  The framework tracks state transitions, refuses to execute
//! actions whose preconditions are not met (in Strict mode), and
//! automatically runs side-effect-free Query actions to resolve Unknown
//! fields.

use std::fmt;

use crate::spm_controller::ZControllerStatus;

// ============================================================================
// Tracked<T> — a value with explicit knowledge status
// ============================================================================

/// A value whose tracking status is explicit.
///
/// - `Known(T)` — set by a successful action or verified by a query.
/// - `Unknown` — was known, but a failed action made it uncertain.
/// - `Uninitialized` — never set; procedure has not addressed this field yet.
#[derive(Debug, Clone, Default)]
pub enum Tracked<T> {
    Known(T),
    Unknown,
    #[default]
    Uninitialized,
}

impl<T: PartialEq> Tracked<T> {
    pub fn is_known(&self) -> bool {
        matches!(self, Tracked::Known(_))
    }

    /// Returns true if the value needs resolution (Unknown or Uninitialized).
    pub fn needs_resolution(&self) -> bool {
        !self.is_known()
    }

    pub fn as_known(&self) -> Option<&T> {
        match self {
            Tracked::Known(v) => Some(v),
            _ => None,
        }
    }

    /// Set to Known.
    pub fn set(&mut self, value: T) {
        *self = Tracked::Known(value);
    }

    /// Degrade to Unknown. Only affects Known values —
    /// Uninitialized stays Uninitialized (it was never set,
    /// so a failure didn't change that fact).
    pub fn degrade(&mut self) {
        if self.is_known() {
            *self = Tracked::Unknown;
        }
    }
}

impl<T: fmt::Display> Tracked<T> {
    pub fn describe(&self, field_name: &str) -> String {
        match self {
            Tracked::Known(v) => format!("{}", v),
            Tracked::Unknown => {
                format!("{}: UNKNOWN (query needed)", field_name)
            }
            Tracked::Uninitialized => {
                format!("{}: uninitialized", field_name)
            }
        }
    }
}

// ============================================================================
// Domain types for tracked fields
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipEngagement {
    Approached,
    Withdrawn,
}

impl fmt::Display for TipEngagement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TipEngagement::Approached => write!(f, "approached"),
            TipEngagement::Withdrawn => write!(f, "withdrawn"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanActivity {
    Running,
    Stopped,
}

impl fmt::Display for ScanActivity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanActivity::Running => write!(f, "running"),
            ScanActivity::Stopped => write!(f, "stopped"),
        }
    }
}

// ============================================================================
// MachineState
// ============================================================================

/// Which field of MachineState an action reads or writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateField {
    Tip,
    BiasV,
    ZSetpointA,
    ZController,
    Scan,
    SafeTipEnabled,
}

/// Software model of the physical machine state.
///
/// Updated by actions on success, degraded to Unknown on error.
/// This is NOT a live mirror of the hardware — it reflects what
/// the software believes to be true based on the actions it has
/// executed.
#[derive(Debug, Clone)]
pub struct MachineState {
    pub tip: Tracked<TipEngagement>,
    pub bias_v: Tracked<f64>,
    pub z_setpoint_a: Tracked<f64>,
    pub z_controller: Tracked<ZControllerStatus>,
    pub scan: Tracked<ScanActivity>,
    pub safe_tip_enabled: Tracked<bool>,
}

impl MachineState {
    /// All fields start as Uninitialized.
    pub fn uninitialized() -> Self {
        Self {
            tip: Tracked::Uninitialized,
            bias_v: Tracked::Uninitialized,
            z_setpoint_a: Tracked::Uninitialized,
            z_controller: Tracked::Uninitialized,
            scan: Tracked::Uninitialized,
            safe_tip_enabled: Tracked::Uninitialized,
        }
    }

    /// Degrade a specific field to Unknown (after a failed action).
    pub fn degrade_field(&mut self, field: StateField) {
        match field {
            StateField::Tip => self.tip.degrade(),
            StateField::BiasV => self.bias_v.degrade(),
            StateField::ZSetpointA => self.z_setpoint_a.degrade(),
            StateField::ZController => self.z_controller.degrade(),
            StateField::Scan => self.scan.degrade(),
            StateField::SafeTipEnabled => self.safe_tip_enabled.degrade(),
        }
    }

    /// Check whether a field needs resolution (Unknown or Uninitialized).
    pub fn needs_resolution(&self, field: StateField) -> bool {
        match field {
            StateField::Tip => self.tip.needs_resolution(),
            StateField::BiasV => self.bias_v.needs_resolution(),
            StateField::ZSetpointA => self.z_setpoint_a.needs_resolution(),
            StateField::ZController => self.z_controller.needs_resolution(),
            StateField::Scan => self.scan.needs_resolution(),
            StateField::SafeTipEnabled => {
                self.safe_tip_enabled.needs_resolution()
            }
        }
    }

    /// Human-readable summary, suitable for logging or LLM context.
    pub fn describe(&self) -> String {
        format!(
            "Tip: {}. Bias: {}. Z-setpoint: {}. Z-ctrl: {}. Scan: {}. Safe-tip: {}.",
            self.tip.describe("tip"),
            self.bias_v.describe("bias"),
            self.z_setpoint_a.describe("setpoint"),
            describe_z_controller(&self.z_controller),
            self.scan.describe("scan"),
            self.safe_tip_enabled.describe("safe-tip"),
        )
    }
}

/// ZControllerStatus doesn't implement Display, so we handle it separately.
fn describe_z_controller(tracked: &Tracked<ZControllerStatus>) -> String {
    match tracked {
        Tracked::Known(s) => format!("{:?}", s),
        Tracked::Unknown => "z-ctrl: UNKNOWN (query needed)".to_string(),
        Tracked::Uninitialized => "z-ctrl: uninitialized".to_string(),
    }
}

impl Default for MachineState {
    fn default() -> Self {
        Self::uninitialized()
    }
}

// ============================================================================
// StateRequirements — what an action expects before execution
// ============================================================================

/// A single requirement on one field of MachineState.
#[derive(Debug, Clone)]
pub enum FieldRequirement {
    /// Field must be Known (any value). Used when the action needs
    /// to read or depend on the field but doesn't care about the value.
    Known(StateField),
    /// Tip must be in a specific engagement state.
    TipIs(TipEngagement),
    /// Scan must be in a specific activity state.
    ScanIs(ScanActivity),
}

/// Collection of preconditions that must be satisfied before an action runs.
#[derive(Debug, Clone, Default)]
pub struct StateRequirements {
    requirements: Vec<FieldRequirement>,
}

impl StateRequirements {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn tip(mut self, engagement: TipEngagement) -> Self {
        self.requirements.push(FieldRequirement::TipIs(engagement));
        self
    }

    pub fn scan(mut self, activity: ScanActivity) -> Self {
        self.requirements.push(FieldRequirement::ScanIs(activity));
        self
    }

    pub fn known(mut self, field: StateField) -> Self {
        self.requirements.push(FieldRequirement::Known(field));
        self
    }

    pub fn requirements(&self) -> &[FieldRequirement] {
        &self.requirements
    }

    pub fn is_empty(&self) -> bool {
        self.requirements.is_empty()
    }

    /// Check all requirements against the current state.
    /// Returns a list of violations (empty = all satisfied).
    pub fn check(&self, state: &MachineState) -> Vec<Violation> {
        let mut violations = Vec::new();
        for req in &self.requirements {
            if let Some(v) = check_requirement(req, state) {
                violations.push(v);
            }
        }
        violations
    }

    /// Which fields these requirements depend on (for auto-recovery).
    pub fn required_fields(&self) -> Vec<StateField> {
        self.requirements
            .iter()
            .map(|r| match r {
                FieldRequirement::Known(f) => *f,
                FieldRequirement::TipIs(_) => StateField::Tip,
                FieldRequirement::ScanIs(_) => StateField::Scan,
            })
            .collect()
    }
}

fn check_requirement(
    req: &FieldRequirement,
    state: &MachineState,
) -> Option<Violation> {
    match req {
        FieldRequirement::Known(field) => {
            if state.needs_resolution(*field) {
                Some(Violation {
                    field: *field,
                    expected: "any known value".to_string(),
                    actual: "unknown/uninitialized".to_string(),
                })
            } else {
                None
            }
        }
        FieldRequirement::TipIs(expected) => match &state.tip {
            Tracked::Known(actual) if actual == expected => None,
            Tracked::Known(actual) => Some(Violation {
                field: StateField::Tip,
                expected: format!("{}", expected),
                actual: format!("{}", actual),
            }),
            _ => Some(Violation {
                field: StateField::Tip,
                expected: format!("{}", expected),
                actual: state.tip.describe("tip"),
            }),
        },
        FieldRequirement::ScanIs(expected) => match &state.scan {
            Tracked::Known(actual) if actual == expected => None,
            Tracked::Known(actual) => Some(Violation {
                field: StateField::Scan,
                expected: format!("{}", expected),
                actual: format!("{}", actual),
            }),
            _ => Some(Violation {
                field: StateField::Scan,
                expected: format!("{}", expected),
                actual: state.scan.describe("scan"),
            }),
        },
    }
}

// ============================================================================
// Violation — a precondition that was not met
// ============================================================================

#[derive(Debug, Clone)]
pub struct Violation {
    pub field: StateField,
    pub expected: String,
    pub actual: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?}: expected {}, got {}",
            self.field, self.expected, self.actual
        )
    }
}

// ============================================================================
// StateEffects — how an action changes state on success
// ============================================================================

/// Describes how an action modifies MachineState on successful execution.
///
/// Used for two purposes:
/// 1. **Chain validation** — simulate state changes without executing.
/// 2. **Error degradation** — on failure, fields listed here become Unknown.
#[derive(Debug, Clone, Default)]
pub struct StateEffects {
    effects: Vec<StateEffect>,
}

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
enum StateEffect {
    SetTip(TipEngagement),
    SetBias(f64),
    SetZSetpoint(f64),
    SetZController(ZControllerStatus),
    SetScan(ScanActivity),
    SetSafeTip(bool),
}

impl StateEffects {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    pub fn set_tip(mut self, engagement: TipEngagement) -> Self {
        self.effects.push(StateEffect::SetTip(engagement));
        self
    }

    pub fn set_bias(mut self, voltage: f64) -> Self {
        self.effects.push(StateEffect::SetBias(voltage));
        self
    }

    pub fn set_z_setpoint(mut self, setpoint: f64) -> Self {
        self.effects.push(StateEffect::SetZSetpoint(setpoint));
        self
    }

    pub fn set_z_controller(mut self, status: ZControllerStatus) -> Self {
        self.effects.push(StateEffect::SetZController(status));
        self
    }

    pub fn set_scan(mut self, activity: ScanActivity) -> Self {
        self.effects.push(StateEffect::SetScan(activity));
        self
    }

    pub fn set_safe_tip(mut self, enabled: bool) -> Self {
        self.effects.push(StateEffect::SetSafeTip(enabled));
        self
    }

    /// Apply effects to machine state (on successful execution).
    pub fn apply(&self, state: &mut MachineState) {
        for effect in &self.effects {
            match effect {
                StateEffect::SetTip(v) => state.tip.set(*v),
                StateEffect::SetBias(v) => state.bias_v.set(*v),
                StateEffect::SetZSetpoint(v) => state.z_setpoint_a.set(*v),
                StateEffect::SetZController(v) => state.z_controller.set(*v),
                StateEffect::SetScan(v) => state.scan.set(*v),
                StateEffect::SetSafeTip(v) => state.safe_tip_enabled.set(*v),
            }
        }
    }

    /// Degrade affected fields to Unknown (on failed execution).
    pub fn degrade(&self, state: &mut MachineState) {
        for effect in &self.effects {
            let field = match effect {
                StateEffect::SetTip(_) => StateField::Tip,
                StateEffect::SetBias(_) => StateField::BiasV,
                StateEffect::SetZSetpoint(_) => StateField::ZSetpointA,
                StateEffect::SetZController(_) => StateField::ZController,
                StateEffect::SetScan(_) => StateField::Scan,
                StateEffect::SetSafeTip(_) => StateField::SafeTipEnabled,
            };
            state.degrade_field(field);
        }
    }

    /// Which fields this effect set touches.
    pub fn affected_fields(&self) -> Vec<StateField> {
        self.effects
            .iter()
            .map(|e| match e {
                StateEffect::SetTip(_) => StateField::Tip,
                StateEffect::SetBias(_) => StateField::BiasV,
                StateEffect::SetZSetpoint(_) => StateField::ZSetpointA,
                StateEffect::SetZController(_) => StateField::ZController,
                StateEffect::SetScan(_) => StateField::Scan,
                StateEffect::SetSafeTip(_) => StateField::SafeTipEnabled,
            })
            .collect()
    }
}

// ============================================================================
// ActionKind + ValidationPolicy
// ============================================================================

/// How an action interacts with the physical hardware state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    /// Pure read — no physical side effects. Safe to auto-insert
    /// for state recovery when a field is Unknown or Uninitialized.
    Query,
    /// Modifies physical hardware state. Must be explicitly placed.
    /// Never auto-inserted by the framework.
    Mutate,
}

/// How strictly the executor enforces action preconditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValidationPolicy {
    /// Do not consult MachineState at all. Preserves pre-state-tracking
    /// behavior for workflows that predate the feature.
    #[default]
    Disabled,
    /// Log warnings but execute anyway. For simulation and testing.
    Advisory,
    /// Hard gate with automatic Query-based recovery for unknown fields.
    /// For real hardware.
    Strict,
}

// ============================================================================
// Chain validation (pre-execution, no hardware calls)
// ============================================================================

/// Error from pre-execution chain validation.
#[derive(Debug, Clone)]
pub struct ChainValidationError {
    pub step: usize,
    pub action_name: String,
    pub violations: Vec<Violation>,
}

impl fmt::Display for ChainValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Step {} ({}): {}",
            self.step,
            self.action_name,
            self.violations
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Validate an action chain against an initial state without executing.
///
/// Simulates state transitions using each action's `effects()` to check
/// whether all `expects()` preconditions would be met at each step.
/// Returns all violations found (not just the first).
pub fn validate_chain(
    chain: &[(&str, &StateRequirements, &StateEffects)],
    initial_state: &MachineState,
) -> Vec<ChainValidationError> {
    let mut simulated = initial_state.clone();
    let mut errors = Vec::new();

    for (i, (name, expects, effects)) in chain.iter().enumerate() {
        let violations = expects.check(&simulated);
        if !violations.is_empty() {
            errors.push(ChainValidationError {
                step: i,
                action_name: name.to_string(),
                violations,
            });
            // Don't apply effects of a step whose preconditions failed —
            // otherwise downstream violations reflect a simulated state that
            // would never actually have been reached, drowning the real root
            // cause in spurious cascades.
            continue;
        }
        effects.apply(&mut simulated);
    }

    errors
}

impl MachineState {
    /// Mark every Known field as Unknown.
    ///
    /// Used after a connection reset or other wide-scope uncertainty event:
    /// the software state model no longer reflects hardware, so the next
    /// action that needs a field will force a fresh query.
    pub fn degrade_all(&mut self) {
        self.tip.degrade();
        self.bias_v.degrade();
        self.z_setpoint_a.degrade();
        self.z_controller.degrade();
        self.scan.degrade();
        self.safe_tip_enabled.degrade();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninitialized_state_needs_resolution() {
        let state = MachineState::uninitialized();
        assert!(state.needs_resolution(StateField::Tip));
        assert!(state.needs_resolution(StateField::BiasV));
        assert!(state.needs_resolution(StateField::Scan));
    }

    #[test]
    fn known_state_does_not_need_resolution() {
        let mut state = MachineState::uninitialized();
        state.bias_v.set(0.5);
        assert!(!state.needs_resolution(StateField::BiasV));
        assert!(state.needs_resolution(StateField::Tip));
    }

    #[test]
    fn degrade_known_to_unknown() {
        let mut state = MachineState::uninitialized();
        state.bias_v.set(0.5);
        assert!(state.bias_v.is_known());

        state.degrade_field(StateField::BiasV);
        assert!(!state.bias_v.is_known());
        assert!(matches!(state.bias_v, Tracked::Unknown));
    }

    #[test]
    fn degrade_uninitialized_stays_uninitialized() {
        let mut state = MachineState::uninitialized();
        state.degrade_field(StateField::BiasV);
        assert!(matches!(state.bias_v, Tracked::Uninitialized));
    }

    #[test]
    fn requirements_empty_always_passes() {
        let state = MachineState::uninitialized();
        let reqs = StateRequirements::none();
        assert!(reqs.check(&state).is_empty());
    }

    #[test]
    fn tip_requirement_passes_when_matched() {
        let mut state = MachineState::uninitialized();
        state.tip.set(TipEngagement::Approached);

        let reqs = StateRequirements::none().tip(TipEngagement::Approached);
        assert!(reqs.check(&state).is_empty());
    }

    #[test]
    fn tip_requirement_fails_when_wrong_value() {
        let mut state = MachineState::uninitialized();
        state.tip.set(TipEngagement::Withdrawn);

        let reqs = StateRequirements::none().tip(TipEngagement::Approached);
        let violations = reqs.check(&state);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, StateField::Tip);
    }

    #[test]
    fn tip_requirement_fails_when_uninitialized() {
        let state = MachineState::uninitialized();
        let reqs = StateRequirements::none().tip(TipEngagement::Approached);
        let violations = reqs.check(&state);
        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn effects_apply_sets_fields() {
        let mut state = MachineState::uninitialized();
        let effects = StateEffects::none()
            .set_tip(TipEngagement::Approached)
            .set_bias(0.5)
            .set_scan(ScanActivity::Stopped);

        effects.apply(&mut state);

        assert_eq!(state.tip.as_known(), Some(&TipEngagement::Approached));
        assert_eq!(state.bias_v.as_known(), Some(&0.5));
        assert_eq!(state.scan.as_known(), Some(&ScanActivity::Stopped));
        // Unaffected fields stay uninitialized
        assert!(state.z_setpoint_a.needs_resolution());
    }

    #[test]
    fn effects_degrade_on_error() {
        let mut state = MachineState::uninitialized();
        state.tip.set(TipEngagement::Approached);
        state.bias_v.set(0.5);

        let effects = StateEffects::none().set_bias(1.0);
        effects.degrade(&mut state);

        // bias was affected by the effect, so it's now Unknown
        assert!(matches!(state.bias_v, Tracked::Unknown));
        // tip was not affected, so it stays Known
        assert_eq!(state.tip.as_known(), Some(&TipEngagement::Approached));
    }

    #[test]
    fn chain_validation_catches_missing_approach() {
        let state = MachineState::uninitialized();

        // Chain: BiasPulse (needs approached) without prior approach
        let pulse_expects =
            StateRequirements::none().tip(TipEngagement::Approached);
        let pulse_effects = StateEffects::none();

        let chain: Vec<(&str, &StateRequirements, &StateEffects)> =
            vec![("bias_pulse", &pulse_expects, &pulse_effects)];

        let errors = validate_chain(&chain, &state);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].step, 0);
        assert_eq!(errors[0].action_name, "bias_pulse");
    }

    #[test]
    fn chain_validation_passes_with_approach_before_pulse() {
        let state = MachineState::uninitialized();

        let approach_expects = StateRequirements::none();
        let approach_effects =
            StateEffects::none().set_tip(TipEngagement::Approached);

        let pulse_expects =
            StateRequirements::none().tip(TipEngagement::Approached);
        let pulse_effects = StateEffects::none();

        let chain: Vec<(&str, &StateRequirements, &StateEffects)> = vec![
            ("calibrated_approach", &approach_expects, &approach_effects),
            ("bias_pulse", &pulse_expects, &pulse_effects),
        ];

        let errors = validate_chain(&chain, &state);
        assert!(errors.is_empty());
    }

    #[test]
    fn chain_validation_catches_scan_start_without_approach() {
        let state = MachineState::uninitialized();

        let scan_expects =
            StateRequirements::none().tip(TipEngagement::Approached);
        let scan_effects = StateEffects::none().set_scan(ScanActivity::Running);

        let chain: Vec<(&str, &StateRequirements, &StateEffects)> =
            vec![("scan_start", &scan_expects, &scan_effects)];

        let errors = validate_chain(&chain, &state);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn describe_formats_mixed_state() {
        let mut state = MachineState::uninitialized();
        state.tip.set(TipEngagement::Approached);
        state.bias_v.set(-0.5);

        let desc = state.describe();
        assert!(desc.contains("approached"));
        assert!(desc.contains("-0.5"));
        assert!(desc.contains("uninitialized"));
    }
}
