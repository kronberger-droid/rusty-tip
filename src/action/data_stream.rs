use serde::Serialize;

use nanonis_rs::tcplog::TCPLogStatus;

use crate::action::{Action, ActionContext};
use crate::spm_controller::Capability;

#[derive(Debug, Clone, Serialize)]
pub struct ConfigureDataStream {
    pub channels: Vec<i32>,
    pub oversampling: i32,
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
    type Output = ();

    fn name(&self) -> &str {
        "configure_data_stream"
    }
    fn description(&self) -> &str {
        "Configure the data stream channels and oversampling rate"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::DataStream]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller
            .data_stream_configure(&self.channels, self.oversampling)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StartDataStream;

impl Action for StartDataStream {
    type Output = ();

    fn name(&self) -> &str {
        "start_data_stream"
    }
    fn description(&self) -> &str {
        "Start the high-throughput data stream"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::DataStream]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.data_stream_start()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StopDataStream;

impl Action for StopDataStream {
    type Output = ();

    fn name(&self) -> &str {
        "stop_data_stream"
    }
    fn description(&self) -> &str {
        "Stop the high-throughput data stream"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::DataStream]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.data_stream_stop()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReadDataStreamStatus;

impl Action for ReadDataStreamStatus {
    type Output = TCPLogStatus;

    fn name(&self) -> &str {
        "read_data_stream_status"
    }
    fn description(&self) -> &str {
        "Get the current data stream status"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::DataStream]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.data_stream_status()
    }
}
