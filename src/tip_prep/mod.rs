pub mod events;
pub mod pulse_state;
pub mod runner;

pub use events::{CycleEvent, MaxPulseEvent, PhaseEvent, log_schema};
pub use pulse_state::PulseState;
pub use runner::{Outcome, TipPrep, TipPrepParams, run_tip_prep};

use std::path::Path;

use crate::config::AppConfig;
use crate::nanonis_controller::StreamSetup;
use crate::spm_controller::SpmController;
use crate::spm_error::SpmError;

/// Load the config's layout and settings files, shared by the CLI and the
/// GUI. Call it before the stream starts: a settings file can change the
/// TCP logger's channel list, and Nanonis stops a live stream on that. Z
/// home and safe-tip are the routine's business ([`TipPrep::run_setup`]).
pub fn load_presets(
    controller: &mut dyn SpmController,
    config: &AppConfig,
) -> Result<(), SpmError> {
    if let Some(path) = &config.nanonis.layout_file {
        controller.load_layout(Path::new(path))?;
    }
    if let Some(path) = &config.nanonis.settings_file {
        controller.load_settings(Path::new(path))?;
    }
    Ok(())
}

/// The data stream tip prep reads its stable signals from.
pub fn stream_setup(config: &AppConfig) -> StreamSetup {
    StreamSetup::new(
        &config.nanonis.host_ip,
        config.data_acquisition.data_port,
        f64::from(config.data_acquisition.sample_rate),
    )
}
