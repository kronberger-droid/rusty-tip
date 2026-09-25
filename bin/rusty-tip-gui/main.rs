//! The workbench: one window, one controller connection, several tools.
//!
//! Everything egui lives under this binary. The session that owns the
//! controller is `rusty_tip::session`, so nothing in here touches hardware
//! directly: the GUI thread sends commands and reads updates and events.

mod app;
mod connection;
mod form;
mod run_view;
mod tools;

use crossbeam_channel::{Receiver, Sender, unbounded};
use log::LevelFilter;

use app::WorkbenchApp;

/// Sends env_logger output to stderr and to the activity log in the window.
struct TeeWriter {
    sender: Sender<String>,
    stderr: std::io::Stderr,
}

impl std::io::Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stderr.write_all(buf)?;
        if let Ok(s) = std::str::from_utf8(buf) {
            let trimmed = s.trim_end_matches('\n');
            if !trimmed.is_empty() {
                let _ = self.sender.try_send(trimmed.to_string());
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stderr.flush()
    }
}

fn init_logging(level: LevelFilter) -> Receiver<String> {
    let (tx, rx) = unbounded();
    let writer = TeeWriter {
        sender: tx,
        stderr: std::io::stderr(),
    };
    env_logger::Builder::new()
        .filter_level(level)
        .filter_module("winit", LevelFilter::Off)
        .filter_module("eframe", LevelFilter::Off)
        .filter_module("egui_glow", LevelFilter::Off)
        .filter_module("wgpu", LevelFilter::Off)
        .filter_module("naga", LevelFilter::Off)
        .filter_module("zbus", LevelFilter::Off)
        .filter_module("tracing", LevelFilter::Off)
        .filter_module("accesskit", LevelFilter::Off)
        .format_timestamp_millis()
        .target(env_logger::Target::Pipe(Box::new(writer)))
        .init();
    rx
}

fn main() -> eframe::Result<()> {
    let log_receiver = init_logging(LevelFilter::Info);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([800.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Rusty Tip",
        options,
        Box::new(move |cc| Ok(Box::new(WorkbenchApp::new(cc, log_receiver)))),
    )
}
