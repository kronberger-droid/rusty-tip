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

/// Grab 2D pixel data from a completed (or in-progress) scan frame.
///
/// Stores the result in the DataStore under `"scan_frame"` as:
/// ```json
/// { "channel_name": "...", "data": [[f32, ...], ...], "direction_up": bool }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrabScanFrame {
    /// Which scan buffer channel to read (0-based index).
    pub channel_index: u32,
    /// `true` for forward scan direction, `false` for backward.
    #[serde(default = "super::default_true")]
    pub forward: bool,
}


impl Default for GrabScanFrame {
    fn default() -> Self {
        Self {
            channel_index: 0,
            forward: true,
        }
    }
}

impl Action for GrabScanFrame {
    fn name(&self) -> &str {
        "grab_scan_frame"
    }
    fn description(&self) -> &str {
        "Grab 2D pixel data from the current scan frame buffer"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Scanning]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let (channel_name, data, direction_up) = ctx
            .controller
            .scan_frame_data_grab(self.channel_index, self.forward)?;
        let result = serde_json::json!({
            "channel_name": channel_name,
            "data": data,
            "direction_up": direction_up,
        });
        Ok(ActionOutput::Data(result))
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
