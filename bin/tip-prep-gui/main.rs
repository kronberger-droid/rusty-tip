// Include shared modules from tip-prep
#[path = "../tip-prep/config.rs"]
mod config;
#[path = "../tip-prep/tip_prep.rs"]
mod tip_prep;

mod app;

use app::{init_logging, TipPrepApp};
use log::LevelFilter;

fn main() -> eframe::Result<()> {
    let log_receiver = init_logging(LevelFilter::Info);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([600.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Rusty Tip Preparation",
        options,
        Box::new(move |cc| {
            let mut app = TipPrepApp::new(cc);
            app.set_log_receiver(log_receiver);
            Ok(Box::new(app))
        }),
    )
}
