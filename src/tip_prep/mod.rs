pub mod events;
pub mod pulse_state;
pub mod runner;

pub use events::{CycleEvent, MaxPulseEvent, PhaseEvent, log_schema};
pub use pulse_state::PulseState;
pub use runner::{Outcome, TipPrep, TipPrepParams, run_tip_prep};

use crate::config::AppConfig;
use crate::nanonis_controller::{NanonisSetupConfig, StreamSetup};
use crate::spm_controller::ZHomeMode;

/// How tip prep sets the Nanonis up, shared by the CLI and the GUI.
pub fn nanonis_setup(config: &AppConfig) -> NanonisSetupConfig {
    NanonisSetupConfig {
        layout_file: config.nanonis.layout_file.clone(),
        settings_file: config.nanonis.settings_file.clone(),
        safe_tip_threshold_a: config.tip_prep.safe_tip_threshold,
        // Spelled out rather than defaulted: the home step of every
        // calibrated approach is "back off 50 nm from wherever the tip is".
        // Absolute mode would make it "go to Z = +50 nm", surface or not.
        z_home_mode: ZHomeMode::Relative,
        z_home_position_m: 50e-9,
        // Off for the run, restored on exit by teardown.
        disable_safe_tip: true,
        ..Default::default()
    }
}

/// The data stream tip prep reads its stable signals from.
pub fn stream_setup(config: &AppConfig) -> StreamSetup {
    StreamSetup::new(
        &config.nanonis.host_ip,
        config.data_acquisition.data_port,
        f64::from(config.data_acquisition.sample_rate),
    )
}
