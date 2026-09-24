//! The connection: what to connect to, and what the session says about the
//! controller once connected.
//!
//! Two views of one thing. The bar along the top says whether we are
//! connected and to what, with the connect button. The Connection page in
//! the middle holds the form (backend, host, ports, files, TCP channel
//! mapping, log directory) and everything the session reports: stream,
//! preset loads, live readouts, the signal table, capabilities.
//!
//! The form is the operator's and is persisted between starts. The rest is
//! a mirror of the session's updates; the pane never asks the controller
//! anything.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

use eframe::egui;
use serde::{Deserialize, Serialize};

use rusty_tip::config::TcpChannelMapping;
use rusty_tip::experiment_log::ControllerFacts;
use rusty_tip::session::{Backend, ConnState, NanonisBackend, PresetLoad, Readout, SessionUpdate};
use rusty_tip::spm_controller::Capability;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BackendKind {
    #[default]
    Nanonis,
    Mock,
}

/// The form, as saved between starts. Strings where the operator types, so
/// a half-typed port never fails to persist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionForm {
    pub kind: BackendKind,
    pub host: String,
    pub port: String,
    pub data_port: String,
    pub sample_rate_hz: String,
    pub layout_file: String,
    pub settings_file: String,
    /// `(signal index, TCP channel)` pairs beyond the standard map.
    pub tcp_channel_mapping: Vec<(String, String)>,
    pub log_dir: String,
}

impl Default for ConnectionForm {
    fn default() -> Self {
        let d = NanonisBackend::default();
        Self {
            kind: BackendKind::Nanonis,
            host: d.host,
            port: d.port.to_string(),
            data_port: d.data_port.to_string(),
            sample_rate_hz: format!("{}", d.sample_rate_hz),
            layout_file: String::new(),
            settings_file: String::new(),
            tcp_channel_mapping: Vec::new(),
            log_dir: "./experiments".into(),
        }
    }
}

impl ConnectionForm {
    /// The backend the form describes, or the first thing wrong with it.
    pub fn backend(&self) -> Result<Backend, String> {
        match self.kind {
            BackendKind::Mock => Ok(Backend::Mock),
            BackendKind::Nanonis => {
                let port = self
                    .port
                    .trim()
                    .parse::<u16>()
                    .map_err(|_| format!("port {:?} is not a port number", self.port))?;
                let data_port =
                    self.data_port.trim().parse::<u16>().map_err(|_| {
                        format!("data port {:?} is not a port number", self.data_port)
                    })?;
                let sample_rate_hz = self
                    .sample_rate_hz
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|r| r.is_finite() && *r > 0.0)
                    .ok_or_else(|| {
                        format!("sample rate {:?} is not a rate in Hz", self.sample_rate_hz)
                    })?;
                let mut tcp_channel_mapping = Vec::new();
                for (i, (index, channel)) in self.tcp_channel_mapping.iter().enumerate() {
                    let nanonis_index = index.trim().parse::<u8>().map_err(|_| {
                        format!("TCP mapping {}: signal index {index:?} is not 0-255", i + 1)
                    })?;
                    let tcp_channel = channel.trim().parse::<u8>().map_err(|_| {
                        format!("TCP mapping {}: channel {channel:?} is not 0-23", i + 1)
                    })?;
                    tcp_channel_mapping.push(TcpChannelMapping {
                        nanonis_index,
                        tcp_channel,
                    });
                }
                let optional = |s: &str| (!s.trim().is_empty()).then(|| PathBuf::from(s.trim()));
                Ok(Backend::Nanonis(NanonisBackend {
                    host: self.host.trim().to_string(),
                    port,
                    data_port,
                    sample_rate_hz,
                    layout_file: optional(&self.layout_file),
                    settings_file: optional(&self.settings_file),
                    tcp_channel_mapping,
                }))
            }
        }
    }

    pub fn log_dir(&self) -> Option<PathBuf> {
        let dir = self.log_dir.trim();
        (!dir.is_empty()).then(|| PathBuf::from(dir))
    }

    /// One line saying what the form points at.
    fn target(&self) -> String {
        match self.kind {
            BackendKind::Mock => "Mock".into(),
            BackendKind::Nanonis => format!("Nanonis {}:{}", self.host.trim(), self.port.trim()),
        }
    }
}

/// What the pane asks the app to send to the session.
#[derive(Debug)]
pub enum PaneAction {
    Connect(Backend),
    Disconnect,
    Reconnect,
    ReloadPresets,
    /// The log directory changed.
    LogDir(Option<PathBuf>),
}

pub struct ConnectionPane {
    pub form: ConnectionForm,
    pub state: ConnState,
    pub facts: Option<ControllerFacts>,
    pub capabilities: HashSet<Capability>,
    pub readouts: Vec<Readout>,
    pub layout_load: Option<PresetLoad>,
    pub settings_load: Option<PresetLoad>,
    pub error: Option<String>,
}

impl ConnectionPane {
    pub fn new(form: ConnectionForm) -> Self {
        Self {
            form,
            state: ConnState::Disconnected,
            facts: None,
            capabilities: HashSet::new(),
            readouts: Vec::new(),
            layout_load: None,
            settings_load: None,
            error: None,
        }
    }

    /// Mirror what the session reported.
    pub fn apply(&mut self, update: &SessionUpdate) {
        match update {
            SessionUpdate::State(state) => {
                self.state = *state;
                if *state == ConnState::Disconnected {
                    self.facts = None;
                    self.capabilities.clear();
                    self.readouts.clear();
                    self.layout_load = None;
                    self.settings_load = None;
                }
            }
            SessionUpdate::Facts(facts) => self.facts = Some(facts.clone()),
            SessionUpdate::Capabilities(caps) => self.capabilities = caps.clone(),
            SessionUpdate::Readouts(readouts) => self.readouts = readouts.clone(),
            SessionUpdate::PresetsLoaded { layout, settings } => {
                self.layout_load = layout.clone();
                self.settings_load = settings.clone();
            }
            SessionUpdate::Error(e) => self.error = Some(e.clone()),
            SessionUpdate::JobFinished(_) => {}
        }
    }

    pub fn connected(&self) -> bool {
        matches!(self.state, ConnState::Connected | ConnState::Running)
    }

    /// The state as a colour and a word.
    pub fn state_badge(&self) -> (egui::Color32, &'static str) {
        match self.state {
            ConnState::Disconnected => (egui::Color32::GRAY, "disconnected"),
            ConnState::Connecting => (egui::Color32::YELLOW, "connecting"),
            ConnState::Connected => (egui::Color32::GREEN, "connected"),
            ConnState::Running => (egui::Color32::LIGHT_BLUE, "running"),
            ConnState::Poisoned => (egui::Color32::RED, "connection lost"),
        }
    }

    /// The bar along the top: are we connected, to what, and the button.
    pub fn render_bar(&mut self, ui: &mut egui::Ui, running: bool) -> Option<PaneAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            let (color, word) = self.state_badge();
            ui.colored_label(color, "●");
            match self.state {
                ConnState::Disconnected => {
                    ui.label(format!("{word} · {}", self.form.target()));
                }
                _ => {
                    ui.label(format!("{word} · {}", self.form.target()));
                    if let Some(facts) = &self.facts {
                        ui.separator();
                        match facts.stream_rate_hz {
                            Some(hz) => ui.label(format!("stream {hz:.0} Hz")),
                            None => ui.label("no stream"),
                        };
                    }
                }
            }
            if let Some(e) = &self.error {
                ui.separator();
                ui.colored_label(egui::Color32::RED, e);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                action = self.render_button(ui, running);
            });
        });
        action
    }

    fn render_button(&mut self, ui: &mut egui::Ui, running: bool) -> Option<PaneAction> {
        let mut action = None;
        match self.state {
            ConnState::Disconnected => {
                if ui.button("Connect").clicked() {
                    match self.form.backend() {
                        Ok(backend) => {
                            self.error = None;
                            action = Some(PaneAction::Connect(backend));
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
            ConnState::Connecting => {
                ui.add_enabled(false, egui::Button::new("Connecting…"));
            }
            ConnState::Connected | ConnState::Running => {
                if ui
                    .add_enabled(!running, egui::Button::new("Disconnect"))
                    .on_disabled_hover_text("Stop the running job first")
                    .clicked()
                {
                    action = Some(PaneAction::Disconnect);
                }
            }
            ConnState::Poisoned => {
                if ui.button("Reconnect").clicked() {
                    self.error = None;
                    action = Some(PaneAction::Reconnect);
                }
                if ui.button("Disconnect").clicked() {
                    action = Some(PaneAction::Disconnect);
                }
            }
        }
        action
    }

    /// The Connection page: the form, then what the session reports.
    pub fn render_page(&mut self, ui: &mut egui::Ui, running: bool) -> Option<PaneAction> {
        let mut action = None;
        let editable = self.state == ConnState::Disconnected;

        ui.heading("Controller");
        if !editable {
            ui.label(egui::RichText::new("Disconnect to change the connection.").weak());
        }
        ui.add_space(4.0);
        ui.add_enabled_ui(editable, |ui| {
            egui::Grid::new("connection_form")
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Backend");
                    egui::ComboBox::from_id_salt("backend_kind")
                        .selected_text(match self.form.kind {
                            BackendKind::Nanonis => "Nanonis",
                            BackendKind::Mock => "Mock",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.form.kind,
                                BackendKind::Nanonis,
                                "Nanonis",
                            );
                            ui.selectable_value(&mut self.form.kind, BackendKind::Mock, "Mock")
                                .on_hover_text(
                                    "The in-memory mock with a realistic tip model. No \
                                     hardware is contacted.",
                                );
                        });
                    ui.end_row();

                    if self.form.kind == BackendKind::Nanonis {
                        ui.label("Host");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.host).desired_width(200.0),
                        );
                        ui.end_row();

                        ui.label("Command port");
                        ui.add(egui::TextEdit::singleline(&mut self.form.port).desired_width(80.0));
                        ui.end_row();

                        ui.label("Data port");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.data_port)
                                .desired_width(80.0),
                        )
                        .on_hover_text("The TCP logger's data port, 6590 on a stock install");
                        ui.end_row();

                        ui.label("Stream rate (Hz)");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.sample_rate_hz)
                                .desired_width(80.0),
                        )
                        .on_hover_text(
                            "What to ask the TCP logger for. It delivers its base rate over \
                             an integer, so the nearest such rate is what arrives.",
                        );
                        ui.end_row();

                        ui.label("Layout file");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.form.layout_file)
                                    .desired_width(320.0),
                            );
                            if ui.button("…").clicked()
                                && let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Nanonis layout", &["lyt"])
                                    .pick_file()
                            {
                                self.form.layout_file = path.display().to_string();
                            }
                        });
                        ui.end_row();

                        ui.label("Settings file");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.form.settings_file)
                                    .desired_width(320.0),
                            );
                            if ui.button("…").clicked()
                                && let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Nanonis settings", &["ini"])
                                    .pick_file()
                            {
                                self.form.settings_file = path.display().to_string();
                            }
                        })
                        .response
                        .on_hover_text(
                            "Loaded once, on connect, before the stream starts. A load \
                             persists for the rest of the session.",
                        );
                        ui.end_row();

                        ui.label("TCP channel mapping");
                        ui.vertical(|ui| {
                            let mut remove = None;
                            for (i, (index, channel)) in
                                self.form.tcp_channel_mapping.iter_mut().enumerate()
                            {
                                ui.horizontal(|ui| {
                                    ui.label("signal");
                                    ui.add(egui::TextEdit::singleline(index).desired_width(40.0));
                                    ui.label("channel");
                                    ui.add(egui::TextEdit::singleline(channel).desired_width(40.0));
                                    if ui.small_button("remove").clicked() {
                                        remove = Some(i);
                                    }
                                });
                            }
                            if let Some(i) = remove {
                                self.form.tcp_channel_mapping.remove(i);
                            }
                            if ui.small_button("add").clicked() {
                                self.form
                                    .tcp_channel_mapping
                                    .push((String::new(), String::new()));
                            }
                        })
                        .response
                        .on_hover_text(
                            "Signal index to TCP logger channel, beyond the standard map",
                        );
                        ui.end_row();
                    }
                });
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Log directory");
            let before = self.form.log_dir.clone();
            ui.add(egui::TextEdit::singleline(&mut self.form.log_dir).desired_width(320.0))
                .on_hover_text("Each run writes a .jsonl log here. Empty for no logs.");
            if ui.button("…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.form.log_dir = path.display().to_string();
            }
            if self.form.log_dir != before {
                action = Some(PaneAction::LogDir(self.form.log_dir()));
            }
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if let Some(a) = self.render_button(ui, running) {
                action = Some(a);
            }
            if self.connected()
                && self.form.kind == BackendKind::Nanonis
                && !(self.form.layout_file.trim().is_empty()
                    && self.form.settings_file.trim().is_empty())
                && ui
                    .add_enabled(!running, egui::Button::new("Reload files"))
                    .on_hover_text("Load the layout and settings files again")
                    .clicked()
            {
                action = Some(PaneAction::ReloadPresets);
            }
        });

        if self.state != ConnState::Disconnected {
            ui.add_space(12.0);
            ui.separator();
            self.render_details(ui);
        }
        action
    }

    fn render_details(&mut self, ui: &mut egui::Ui) {
        ui.heading("Session");
        egui::Grid::new("connection_details")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.label("State");
                let (color, word) = self.state_badge();
                ui.colored_label(color, word);
                ui.end_row();
                if let Some(facts) = &self.facts {
                    ui.label("Stream");
                    ui.label(match facts.stream_rate_hz {
                        Some(hz) => format!("{hz:.0} Hz"),
                        None => "none".into(),
                    });
                    ui.end_row();
                }
                ui.label("Settings file");
                ui.label(describe_load(self.settings_load.as_ref()));
                ui.end_row();
                ui.label("Layout file");
                ui.label(describe_load(self.layout_load.as_ref()));
                ui.end_row();
            });

        if self.connected() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Live readouts").strong());
            if self.readouts.is_empty() {
                ui.label(egui::RichText::new("none").weak());
            }
            ui.horizontal(|ui| {
                for r in &self.readouts {
                    ui.label(egui::RichText::new(format_readout(&r.name, r.value)).size(16.0));
                    ui.add_space(16.0);
                }
            });
        }

        if let Some(facts) = &self.facts {
            ui.add_space(8.0);
            ui.collapsing(format!("Signals ({})", facts.signals.len()), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        egui::Grid::new("signal_table")
                            .num_columns(3)
                            .striped(true)
                            .spacing([16.0, 2.0])
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new("index").strong());
                                ui.label(egui::RichText::new("name").strong());
                                ui.label(egui::RichText::new("TCP channel").strong());
                                ui.end_row();
                                for s in &facts.signals {
                                    ui.label(s.index.to_string());
                                    ui.label(&s.name);
                                    ui.label(
                                        s.tcp_channel
                                            .map(|c| c.to_string())
                                            .unwrap_or_else(|| "-".into()),
                                    );
                                    ui.end_row();
                                }
                            });
                    });
            });
        }

        if !self.capabilities.is_empty() {
            let mut caps: Vec<String> =
                self.capabilities.iter().map(|c| format!("{c:?}")).collect();
            caps.sort();
            ui.collapsing(format!("Capabilities ({})", caps.len()), |ui| {
                ui.label(caps.join(", "));
            });
        }
    }
}

fn describe_load(load: Option<&PresetLoad>) -> String {
    match load {
        Some(load) => format!(
            "{} (by {}, {})",
            load.path.display(),
            load.by,
            clock(load.at)
        ),
        None => "none loaded".into(),
    }
}

/// A readout in the unit its name promises, since the controller reports
/// everything in SI base units.
fn format_readout(name: &str, value: f64) -> String {
    let lower = name.to_lowercase();
    let short = name.split('(').next().unwrap_or(name).trim();
    if lower.contains("(m)") {
        format!("{short} {:.3} nm", value * 1e9)
    } else if lower.contains("(a)") {
        format!("{short} {:.1} pA", value * 1e12)
    } else if lower.contains("(v)") {
        format!("{short} {value:.3} V")
    } else if lower.contains("freq") {
        format!("{short} {value:.2} Hz")
    } else {
        format!("{short} {value:.4}")
    }
}

/// Wall-clock time of day, local.
fn clock(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at)
        .format("%H:%M")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_form_is_a_local_nanonis() {
        let form = ConnectionForm::default();
        match form.backend().unwrap() {
            Backend::Nanonis(b) => {
                assert_eq!(b.host, "127.0.0.1");
                assert_eq!(b.port, 6501);
                assert_eq!(b.data_port, 6590);
                assert_eq!(b.sample_rate_hz, 1000.0);
                assert!(b.layout_file.is_none());
            }
            Backend::Mock => panic!("not the mock"),
        }
        assert_eq!(form.target(), "Nanonis 127.0.0.1:6501");
    }

    #[test]
    fn a_bad_port_is_named_in_the_error() {
        let form = ConnectionForm {
            port: "65a".into(),
            ..Default::default()
        };
        assert!(form.backend().unwrap_err().contains("65a"));
    }

    #[test]
    fn mappings_and_files_carry_through() {
        let form = ConnectionForm {
            layout_file: " a.lyt ".into(),
            tcp_channel_mapping: vec![("76".into(), "3".into())],
            ..Default::default()
        };
        let Backend::Nanonis(b) = form.backend().unwrap() else {
            panic!("not nanonis");
        };
        assert_eq!(b.layout_file, Some(PathBuf::from("a.lyt")));
        assert_eq!(b.tcp_channel_mapping[0].nanonis_index, 76);
        assert_eq!(b.tcp_channel_mapping[0].tcp_channel, 3);
    }

    #[test]
    fn readouts_show_human_units() {
        assert_eq!(format_readout("Z (m)", 12.3e-9), "Z 12.300 nm");
        assert_eq!(format_readout("Current (A)", 50.1e-12), "Current 50.1 pA");
        assert_eq!(format_readout("Bias (V)", 0.2), "Bias 0.200 V");
        assert_eq!(format_readout("freq shift", -3.2), "freq shift -3.20 Hz");
    }
}
