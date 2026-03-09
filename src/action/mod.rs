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

use crate::spm_controller::Capability;
use crate::spm_error::SpmError;

type Result<T> = std::result::Result<T, SpmError>;

/// Every SPM operation implements this trait.
///
/// Actions are self-contained, serializable units of work that execute
/// against an `SpmController` via the `ActionContext`. Each action struct
/// holds its own parameters and knows how to execute itself.
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
}

/// Create an ActionRegistry pre-loaded with all built-in actions.
pub fn builtin_registry() -> ActionRegistry {
    let mut r = ActionRegistry::new();

    // Bias
    r.register::<bias::ReadBias>();
    r.register::<bias::SetBias>();
    r.register::<bias::BiasPulse>();

    // Signals
    r.register::<signals::ReadSignal>();
    r.register::<signals::ReadSignals>();
    r.register::<signals::ReadSignalNames>();

    // Z-Controller
    r.register::<z_controller::Withdraw>();
    r.register::<z_controller::AutoApproach>();
    r.register::<z_controller::SetZSetpoint>();

    // Piezo Position
    r.register::<position::ReadPosition>();
    r.register::<position::SetPosition>();

    // Motor
    r.register::<motor::MoveMotor>();
    r.register::<motor::MoveMotor3D>();
    r.register::<motor::MoveMotorClosedLoop>();
    r.register::<motor::StopMotor>();

    // Scanning
    r.register::<scan::ScanControl>();
    r.register::<scan::ReadScanStatus>();

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
