use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::Capability;
use crate::spm_controller::AcquisitionMode;

/// Serializable acquisition mode for the oscilloscope.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionModeParam {
    Current,
    #[default]
    NextTrigger,
    WaitTwoTriggers,
}

impl From<AcquisitionModeParam> for AcquisitionMode {
    fn from(p: AcquisitionModeParam) -> AcquisitionMode {
        match p {
            AcquisitionModeParam::Current => AcquisitionMode::Current,
            AcquisitionModeParam::NextTrigger => AcquisitionMode::NextTrigger,
            AcquisitionModeParam::WaitTwoTriggers => AcquisitionMode::WaitTwoTriggers,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsciRead {
    pub channel: i32,
    #[serde(default)]
    pub mode: AcquisitionModeParam,
    // Trigger configuration is omitted for now -- the TriggerSetup type
    // aliases nanonis-rs TriggerConfig which doesn't derive Serialize.
    // Actions that need custom triggers can be added once we have our
    // own serializable trigger type.
}

impl Default for OsciRead {
    fn default() -> Self {
        Self {
            channel: 0,
            mode: AcquisitionModeParam::NextTrigger,
        }
    }
}

impl Action for OsciRead {
    fn name(&self) -> &str {
        "osci_read"
    }
    fn description(&self) -> &str {
        "Read oscilloscope data from a signal channel"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Oscilloscope]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let data = ctx.controller.osci_read(
            self.channel,
            None, // no trigger override
            self.mode.clone().into(),
        )?;
        Ok(ActionOutput::Data(serde_json::json!({
            "t0": data.t0,
            "dt": data.dt,
            "size": data.size,
            "data": data.data,
        })))
    }
}
