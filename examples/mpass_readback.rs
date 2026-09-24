//! Ask the controller to save its active multi-pass configuration, so what we
//! sent can be diffed against what it kept.
//!
//! The path is resolved by the controller, not by us: under Wine that means a
//! mapped drive (`P:\\readback.mpas`), on a real instrument a path on that PC.
//!
//! Expect the play offset to come back slightly changed. The RT system stores
//! it as `f32`, so 210 pm returns as `210.000003E-12`. Everything else should
//! diff clean against the file that was loaded.
use rusty_tip::nanonis_controller::{NanonisController, NanonisSetupConfig};
use rusty_tip::spm_controller::SpmController;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host_path = std::env::args().nth(1).expect("usage: <host path>");
    let client = rusty_tip::NanonisClient::builder()
        .address("127.0.0.1")
        .port(6501)
        .build()?;
    let mut c = NanonisController::new(client, NanonisSetupConfig::default());
    c.multi_pass_save(&host_path)?;
    println!("saved to {host_path}");
    Ok(())
}
