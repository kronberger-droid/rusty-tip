use serde::{Deserialize, Serialize};

use nanonis_rs::Position;

use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::Capability;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadPosition {
    #[serde(default = "default_true")]
    pub wait_for_newest: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ReadPosition {
    fn default() -> Self {
        Self {
            wait_for_newest: true,
        }
    }
}

impl Action for ReadPosition {
    fn name(&self) -> &str {
        "read_position"
    }
    fn description(&self) -> &str {
        "Read the current piezo position (x, y) in meters"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::PiezoPosition]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let pos = ctx.controller.get_position(self.wait_for_newest)?;
        Ok(ActionOutput::Values(vec![
            ("x".to_string(), pos.x),
            ("y".to_string(), pos.y),
        ]))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetPosition {
    pub x: f64,
    pub y: f64,
    #[serde(default = "default_true")]
    pub wait: bool,
}

impl Default for SetPosition {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            wait: true,
        }
    }
}

impl Action for SetPosition {
    fn name(&self) -> &str {
        "set_position"
    }
    fn description(&self) -> &str {
        "Set the piezo position (x, y) in meters"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::PiezoPosition]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let pos = Position::new(self.x, self.y);
        ctx.controller.set_position(pos, self.wait)?;
        Ok(ActionOutput::Unit)
    }
}
