// Include shared modules from tip-prep
#[path = "../tip-prep/config.rs"]
mod config;
#[path = "../tip-prep/tip_prep.rs"]
mod tip_prep;

mod app;

use app::TipPrepApp;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,zbus=warn,tracing=warn")
    ).init();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([650.0, 600.0])
            .with_min_inner_size([500.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Rusty Tip Preparation",
        options,
        Box::new(|cc| Ok(Box::new(TipPrepApp::new(cc)))),
    )
}
