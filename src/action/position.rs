use serde::Serialize;

use nanonis_rs::Position;

use crate::action::{Action, ActionContext};
use crate::spm_controller::Capability;

#[derive(Debug, Clone, Serialize)]
pub struct ReadPosition {
    pub wait_for_newest: bool,
}

impl Default for ReadPosition {
    fn default() -> Self {
        Self {
            wait_for_newest: true,
        }
    }
}

impl Action for ReadPosition {
    type Output = Vec<(String, f64)>;

    fn name(&self) -> &str {
        "read_position"
    }
    fn description(&self) -> &str {
        "Read the current piezo position (x, y) in meters"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::PiezoPosition]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        let pos = ctx.controller.get_position(self.wait_for_newest)?;
        Ok(vec![("x".to_string(), pos.x), ("y".to_string(), pos.y)])
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SetPosition {
    pub x: f64,
    pub y: f64,
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
    type Output = ();

    fn name(&self) -> &str {
        "set_position"
    }
    fn description(&self) -> &str {
        "Set the piezo position (x, y) in meters"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::PiezoPosition]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        let pos = Position::new(self.x, self.y);
        ctx.controller.set_position(pos, self.wait)?;
        Ok(())
    }
}
