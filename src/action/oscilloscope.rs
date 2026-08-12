use serde::Serialize;

use crate::action::{Action, ActionContext};
use crate::spm_controller::AcquisitionMode;
use crate::spm_controller::Capability;

/// Serializable acquisition mode for the oscilloscope.
#[derive(Debug, Clone, Default, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct OsciRead {
    pub channel: i32,
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

/// One oscilloscope acquisition: samples starting at `t0`, spaced `dt`
/// seconds apart. Serializable mirror of nanonis-rs `OsciData`, which
/// does not derive `Serialize`.
#[derive(Debug, Clone, Serialize)]
pub struct OsciTrace {
    pub t0: f64,
    pub dt: f64,
    pub data: Vec<f64>,
}

impl Action for OsciRead {
    type Output = OsciTrace;

    fn name(&self) -> &str {
        "osci_read"
    }
    fn description(&self) -> &str {
        "Read oscilloscope data from a signal channel"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Oscilloscope]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        let data = ctx.controller.osci_read(
            self.channel,
            None, // no trigger override
            self.mode.clone().into(),
        )?;
        Ok(OsciTrace {
            t0: data.t0,
            dt: data.dt,
            data: data.data,
        })
    }
}
