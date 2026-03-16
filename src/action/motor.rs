use serde::{Deserialize, Serialize};

use nanonis_rs::motor::{MotorDirection, MotorDisplacement, MovementMode, Position3D};

use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_controller::Capability;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveMotor {
    pub direction: MotorDirectionParam,
    pub steps: u16,
    #[serde(default = "super::default_true")]
    pub wait: bool,
}

/// Serializable motor direction that maps to nanonis-rs MotorDirection.
/// Needed because MotorDirection doesn't derive Serialize/Deserialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotorDirectionParam {
    XPlus,
    XMinus,
    YPlus,
    YMinus,
    ZPlus,
    ZMinus,
}

impl From<MotorDirectionParam> for MotorDirection {
    fn from(p: MotorDirectionParam) -> Self {
        match p {
            MotorDirectionParam::XPlus => MotorDirection::XPlus,
            MotorDirectionParam::XMinus => MotorDirection::XMinus,
            MotorDirectionParam::YPlus => MotorDirection::YPlus,
            MotorDirectionParam::YMinus => MotorDirection::YMinus,
            MotorDirectionParam::ZPlus => MotorDirection::ZPlus,
            MotorDirectionParam::ZMinus => MotorDirection::ZMinus,
        }
    }
}

impl Default for MoveMotor {
    fn default() -> Self {
        Self {
            direction: MotorDirectionParam::ZPlus,
            steps: 1,
            wait: true,
        }
    }
}

impl Action for MoveMotor {
    fn name(&self) -> &str {
        "move_motor"
    }
    fn description(&self) -> &str {
        "Move the coarse motor in a given direction by a number of steps"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Motor]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller
            .move_motor(self.direction.clone().into(), self.steps, self.wait)?;
        Ok(ActionOutput::Unit)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveMotor3D {
    pub x: i16,
    pub y: i16,
    pub z: i16,
    #[serde(default = "super::default_true")]
    pub wait: bool,
}

impl Default for MoveMotor3D {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            z: 0,
            wait: true,
        }
    }
}

impl Action for MoveMotor3D {
    fn name(&self) -> &str {
        "move_motor_3d"
    }
    fn description(&self) -> &str {
        "Move the coarse motor with a 3D displacement vector (x, y, z steps)"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Motor]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let displacement = MotorDisplacement {
            x: self.x,
            y: self.y,
            z: self.z,
        };
        ctx.controller.move_motor_3d(displacement, self.wait)?;
        Ok(ActionOutput::Unit)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveMotorClosedLoop {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    #[serde(default)]
    pub mode: MovementModeParam,
}

/// Serializable movement mode for closed-loop motor control.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MovementModeParam {
    #[default]
    Absolute,
    Relative,
}

impl From<MovementModeParam> for MovementMode {
    fn from(p: MovementModeParam) -> Self {
        match p {
            MovementModeParam::Absolute => MovementMode::Absolute,
            MovementModeParam::Relative => MovementMode::Relative,
        }
    }
}

impl Default for MoveMotorClosedLoop {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            mode: MovementModeParam::Absolute,
        }
    }
}

impl Action for MoveMotorClosedLoop {
    fn name(&self) -> &str {
        "move_motor_closed_loop"
    }
    fn description(&self) -> &str {
        "Move the motor to an absolute position using closed-loop control (meters)"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Motor]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        let target = Position3D {
            x: self.x,
            y: self.y,
            z: self.z,
        };
        ctx.controller
            .move_motor_closed_loop(target, self.mode.clone().into())?;
        Ok(ActionOutput::Unit)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StopMotor;

impl Action for StopMotor {
    fn name(&self) -> &str {
        "stop_motor"
    }
    fn description(&self) -> &str {
        "Stop all motor movement immediately"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Motor]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<ActionOutput> {
        ctx.controller.stop_motor()?;
        Ok(ActionOutput::Unit)
    }
}
