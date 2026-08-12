use serde::Serialize;

use nanonis_rs::scan::{ScanAction, ScanDirection};

use crate::action::{Action, ActionContext};
use crate::frame::Frame;
use crate::spm_controller::Capability;

/// Serializable scan action that maps to nanonis-rs ScanAction.
#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Default, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct ScanControl {
    pub action: ScanActionParam,
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
    type Output = ();

    fn name(&self) -> &str {
        "scan_control"
    }
    fn description(&self) -> &str {
        "Control scanning: start, stop, pause, or resume"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Scanning]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller
            .scan_action(self.action.clone().into(), self.direction.clone().into())?;
        Ok(())
    }
}

/// Grab 2D pixel data from a completed (or in-progress) scan frame,
/// together with the frame's physical geometry.
#[derive(Debug, Clone, Serialize)]
pub struct GrabScanFrame {
    /// Which scan buffer channel to read (0-based index).
    pub channel_index: u32,
    /// `true` for forward scan direction, `false` for backward.
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
    type Output = Frame;

    fn name(&self) -> &str {
        "grab_scan_frame"
    }
    fn description(&self) -> &str {
        "Grab 2D pixel data from the current scan frame buffer"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Scanning]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        let (channel_name, data, direction_up) = ctx
            .controller
            .scan_frame_data_grab(self.channel_index, self.forward)?;
        let geometry = ctx.controller.scan_frame_get()?;
        Ok(Frame::from_rows(channel_name, data)?
            .with_direction(direction_up)
            .with_geometry(geometry.into()))
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReadScanStatus;

impl Action for ReadScanStatus {
    type Output = bool;

    fn name(&self) -> &str {
        "read_scan_status"
    }
    fn description(&self) -> &str {
        "Check if the scanner is currently running"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Scanning]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.scan_status()
    }
}
