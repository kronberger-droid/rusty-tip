pub mod bias;
mod context;
pub mod data_stream;
pub mod motor;
pub mod oscilloscope;
mod output;
pub mod pll;
pub mod position;
mod registry;
pub mod scan;
pub mod signals;
mod store;
pub mod tip_shaper;
pub mod util;
pub mod z_controller;

pub use context::ActionContext;
pub use output::ActionOutput;
pub use registry::{ActionFactory, ActionInfo, ActionRegistry};
pub use store::DataStore;

use crate::machine_state::{
    ActionKind, MachineState, StateEffects, StateRequirements, StateField,
};
use crate::spm_controller::Capability;
use crate::spm_error::SpmError;

/// Shared serde default for boolean fields that should default to `true`.
pub(crate) fn default_true() -> bool {
    true
}

type Result<T> = std::result::Result<T, SpmError>;

/// Every SPM operation implements this trait.
///
/// Actions are self-contained, serializable units of work that execute
/// against an `SpmController` via the `ActionContext`. Each action struct
/// holds its own parameters and knows how to execute itself.
///
/// ## State tracking (Phase 3)
///
/// Actions declare their relationship with machine state via four optional
/// methods: `kind()`, `expects()`, `effects()`, and `apply_to_state()`.
/// All have backwards-compatible defaults (Mutate, no requirements, no
/// effects, apply effects on success). Annotate actions incrementally,
/// starting with safety-critical ones.
pub trait Action: Send + Sync {
    /// Unique identifier, e.g. "read_signal", "bias_pulse"
    fn name(&self) -> &str;

    /// Human-readable description for documentation and LLM context
    fn description(&self) -> &str;

    /// Which hardware capabilities this action needs.
    /// The execution layer checks these against `SpmController::capabilities()`
    /// before running the action. Returns empty by default (no requirements).
    fn requires(&self) -> Vec<Capability> {
        vec![]
    }

    /// Execute this action against the provided context
    fn execute(&self, ctx: &mut ActionContext) -> Result<ActionOutput>;

    // -- State tracking --

    /// Whether this action is a pure read (Query) or changes hardware (Mutate).
    ///
    /// Query actions can be auto-inserted by the framework to resolve Unknown
    /// or Uninitialized fields. Mutate actions are never auto-inserted.
    /// Default: Mutate (conservative — forces explicit opt-in for Query).
    fn kind(&self) -> ActionKind {
        ActionKind::Mutate
    }

    /// What machine state this action needs to proceed.
    /// Default: no requirements (always valid).
    fn expects(&self) -> StateRequirements {
        StateRequirements::none()
    }

    /// How this action changes the machine state model on success.
    /// Used for chain validation and error degradation.
    /// Default: no state changes.
    fn effects(&self) -> StateEffects {
        StateEffects::none()
    }

    /// Which state fields this action resolves (Query actions only).
    ///
    /// When the framework encounters an Unknown or Uninitialized field
    /// required by a subsequent action, it searches for a Query action
    /// whose `resolves()` includes that field, executes it, and uses
    /// `apply_to_state()` to update the model.
    fn resolves(&self) -> Vec<StateField> {
        vec![]
    }

    /// Update machine state from the actual execution result.
    ///
    /// Called by the framework after a successful execute(). For Mutate
    /// actions, the default applies `effects()`. For Query actions, override
    /// this to write the queried value into MachineState.
    fn apply_to_state(
        &self,
        _output: &ActionOutput,
        state: &mut MachineState,
    ) {
        self.effects().apply(state);
    }

    /// Execute and store the result under the action's name.
    /// Overwrites any previous value stored under that key.
    fn execute_and_store_as(&self, ctx: &mut ActionContext) -> Result<ActionOutput> {
        let output = self.execute(ctx)?;
        ctx.store.set(self.name(), &output)?;
        Ok(output)
    }

    /// Execute and store the result under a custom key.
    fn execute_and_store(&self, ctx: &mut ActionContext, key: &str) -> Result<ActionOutput> {
        let output = self.execute(ctx)?;
        ctx.store.set(key, &output)?;
        Ok(output)
    }
}

/// Create an ActionRegistry pre-loaded with all built-in actions.
pub fn builtin_registry() -> ActionRegistry {
    let mut r = ActionRegistry::new();

    // Bias
    r.register::<bias::ReadBias>();
    r.register::<bias::SetBias>();
    r.register::<bias::SafeSetBias>();
    r.register::<bias::BiasPulse>();

    // Signals
    r.register::<signals::ReadSignal>();
    r.register::<signals::ReadSignals>();
    r.register::<signals::ReadSignalNames>();
    r.register::<signals::ReadStableSignal>();

    // Z-Controller
    r.register::<z_controller::Withdraw>();
    r.register::<z_controller::AutoApproach>();
    r.register::<z_controller::SetZSetpoint>();
    r.register::<z_controller::ZHome>();
    r.register::<z_controller::SafeTipSet>();
    r.register::<z_controller::CalibratedApproach>();

    // Piezo Position
    r.register::<position::ReadPosition>();
    r.register::<position::SetPosition>();

    // Motor
    r.register::<motor::MoveMotor>();
    r.register::<motor::MoveMotor3D>();
    r.register::<motor::MoveMotorClosedLoop>();
    r.register::<motor::StopMotor>();
    r.register::<motor::Reposition>();

    // Scanning
    r.register::<scan::ScanControl>();
    r.register::<scan::ReadScanStatus>();
    r.register::<scan::GrabScanFrame>();

    // Oscilloscope
    r.register::<oscilloscope::OsciRead>();

    // Tip Shaper
    r.register::<tip_shaper::TipShape>();

    // PLL
    r.register::<pll::CenterFreqShift>();

    // Data Stream
    r.register::<data_stream::ConfigureDataStream>();
    r.register::<data_stream::StartDataStream>();
    r.register::<data_stream::StopDataStream>();
    r.register::<data_stream::ReadDataStreamStatus>();

    // Utility
    r.register::<util::Wait>();

    r
}
