use serde::{Deserialize, Serialize};

use nanonis_rs::scan::{ScanAction, ScanDirection};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::Capability;

/// Serializable scan action that maps to nanonis-rs ScanAction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanActionParam {
    Start,
    Stop,
    Pause,
    Resume,
}

impl From<ScanActionParam> for ScanAction {
    fn from(p: ScanActionParam) -> Self {
        match p {
            ScanActionParam::Start => ScanAction::Start,
            ScanActionParam::Stop => ScanAction::Stop,
            ScanActionParam::Pause => ScanAction::Pause,
            ScanActionParam::Resume => ScanAction::Resume,
        }
    }
}

/// Serializable scan direction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanDirectionParam {
    #[default]
    Up,
    Down,
}

impl From<ScanDirectionParam> for ScanDirection {
    fn from(p: ScanDirectionParam) -> Self {
        match p {
            ScanDirectionParam::Up => ScanDirection::Up,
            ScanDirectionParam::Down => ScanDirection::Down,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanControl {
    pub action: ScanActionParam,
    #[serde(default)]
    pub direction: ScanDirectionParam,
}

impl Default for ScanControl {
    fn default() -> Self {
        Self {
            action: ScanActionParam::Start,
            direction: ScanDirectionParam::Up,
        }
    }
}

impl Action for ScanControl {
    fn name(&self) -> &str {
        "scan_control"
    }
    fn description(&self) -> &str {
        "Control scanning: start, stop, pause, or resume"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Scanning]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller
            .scan_action(self.action.clone().into(), self.direction.clone().into())?;
        Ok(ActionOutput::Unit)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadScanStatus;

impl Action for ReadScanStatus {
    fn name(&self) -> &str {
        "read_scan_status"
    }
    fn description(&self) -> &str {
        "Check if the scanner is currently running"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Scanning]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let running = ctx.controller.scan_status()?;
        Ok(ActionOutput::Data(serde_json::json!({ "running": running })))
    }
}
