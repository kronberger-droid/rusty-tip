use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::Capability;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigureDataStream {
    pub channels: Vec<i32>,
    #[serde(default = "default_oversampling")]
    pub oversampling: i32,
}

fn default_oversampling() -> i32 {
    10
}

impl Default for ConfigureDataStream {
    fn default() -> Self {
        Self {
            channels: vec![],
            oversampling: 10,
        }
    }
}

impl Action for ConfigureDataStream {
    fn name(&self) -> &str {
        "configure_data_stream"
    }
    fn description(&self) -> &str {
        "Configure the data stream channels and oversampling rate"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::DataStream]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller
            .data_stream_configure(&self.channels, self.oversampling)?;
        Ok(ActionOutput::Unit)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StartDataStream;

impl Action for StartDataStream {
    fn name(&self) -> &str {
        "start_data_stream"
    }
    fn description(&self) -> &str {
        "Start the high-throughput data stream"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::DataStream]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller.data_stream_start()?;
        Ok(ActionOutput::Unit)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StopDataStream;

impl Action for StopDataStream {
    fn name(&self) -> &str {
        "stop_data_stream"
    }
    fn description(&self) -> &str {
        "Stop the high-throughput data stream"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::DataStream]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller.data_stream_stop()?;
        Ok(ActionOutput::Unit)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadDataStreamStatus;

impl Action for ReadDataStreamStatus {
    fn name(&self) -> &str {
        "read_data_stream_status"
    }
    fn description(&self) -> &str {
        "Get the current data stream status"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::DataStream]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let status = ctx.controller.data_stream_status()?;
        let json = serde_json::to_value(status).map_err(|e| {
            crate::spm_error::SpmError::Protocol(format!(
                "Failed to serialize data stream status: {}",
                e
            ))
        })?;
        Ok(ActionOutput::Data(json))
    }
}
