use serde::Serialize;

use crate::action::{Action, ActionContext};
use crate::spm_controller::Capability;

#[derive(Debug, Clone, Default, Serialize)]
pub struct CenterFreqShift;

impl Action for CenterFreqShift {
    type Output = ();

    fn name(&self) -> &str {
        "center_freq_shift"
    }
    fn description(&self) -> &str {
        "Auto-center the PLL frequency shift"
    }
    fn requires(&self) -> Vec<Capability> {
        vec![Capability::Pll]
    }
    fn execute(&self, ctx: &mut ActionContext) -> super::Result<Self::Output> {
        ctx.controller.pll_center_freq_shift()?;
        Ok(())
    }
}
