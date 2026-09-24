//! The connection pane: what to connect to, and what the session says
//! about the controller once connected.
//!
//! The pane holds two things. The form (backend, host, ports, files, TCP
//! channel mapping) is the operator's, persisted between starts. The mirror
//! (state, facts, capabilities, readouts, preset loads) is whatever the
//! session last reported; the pane never asks the controller anything.

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
    show_signals: bool,
    show_capabilities: bool,
    show_mapping: bool,
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
            show_signals: false,
            show_capabilities: false,
            show_mapping: false,
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

    /// Draw the pane. `running` disables what must not change under a job.
    pub fn render(&mut self, ui: &mut egui::Ui, running: bool) -> Option<PaneAction> {
        let mut action = None;
        let editable = self.state == ConnState::Disconnected;

        ui.horizontal(|ui| {
            ui.label("Backend");
            ui.add_enabled_ui(editable, |ui| {
                egui::ComboBox::from_id_salt("backend_kind")
                    .selected_text(match self.form.kind {
                        BackendKind::Nanonis => "Nanonis",
                        BackendKind::Mock => "Mock",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.form.kind, BackendKind::Nanonis, "Nanonis");
                        ui.selectable_value(&mut self.form.kind, BackendKind::Mock, "Mock")
                            .on_hover_text(
                                "The in-memory mock with a realistic tip model. \
                                 No hardware is contacted.",
                            );
                    });
            });

            if self.form.kind == BackendKind::Nanonis {
                ui.add_enabled_ui(editable, |ui| {
                    ui.label("host");
                    ui.add(egui::TextEdit::singleline(&mut self.form.host).desired_width(110.0));
                    ui.label("port");
                    ui.add(egui::TextEdit::singleline(&mut self.form.port).desired_width(48.0));
                    ui.label("data");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.data_port).desired_width(48.0),
                    );
                    ui.label("rate");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.sample_rate_hz)
                            .desired_width(56.0),
                    )
                    .on_hover_text("Stream rate to ask the TCP logger for, in Hz");
                });
            }

            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match self.state {
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
                },
            );
        });

        if self.form.kind == BackendKind::Nanonis {
            ui.horizontal(|ui| {
                ui.add_enabled_ui(editable, |ui| {
                    ui.label("layout");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.layout_file).desired_width(220.0),
                    );
                    if ui.button("…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Nanonis layout", &["lyt"])
                            .pick_file()
                    {
                        self.form.layout_file = path.display().to_string();
                    }
                    ui.label("settings");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.settings_file)
                            .desired_width(220.0),
                    );
                    if ui.button("…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Nanonis settings", &["ini"])
                            .pick_file()
                    {
                        self.form.settings_file = path.display().to_string();
                    }
                });
                if self.connected()
                    && (self.form.layout_file.trim().len() + self.form.settings_file.trim().len())
                        > 0
                    && ui
                        .add_enabled(!running, egui::Button::new("Reload"))
                        .on_hover_text("Load the layout and settings files again")
                        .clicked()
                {
                    action = Some(PaneAction::ReloadPresets);
                }
            });
        }

        ui.horizontal(|ui| {
            ui.label("log dir");
            let before = self.form.log_dir.clone();
            ui.add(egui::TextEdit::singleline(&mut self.form.log_dir).desired_width(220.0))
                .on_hover_text("Each run writes a .jsonl log here. Empty for no logs.");
            if ui.button("…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.form.log_dir = path.display().to_string();
            }
            if self.form.log_dir != before {
                action = Some(PaneAction::LogDir(self.form.log_dir()));
            }
            if self.form.kind == BackendKind::Nanonis {
                ui.add_enabled_ui(editable, |ui| {
                    ui.toggle_value(&mut self.show_mapping, "TCP mapping")
                        .on_hover_text(
                            "Signal index to TCP logger channel, beyond the standard map",
                        );
                });
            }
        });

        if self.show_mapping && self.form.kind == BackendKind::Nanonis {
            ui.add_enabled_ui(editable, |ui| {
                let mut remove = None;
                for (i, (index, channel)) in self.form.tcp_channel_mapping.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label("signal");
                        ui.add(egui::TextEdit::singleline(index).desired_width(40.0));
                        ui.label("channel");
                        ui.add(egui::TextEdit::singleline(channel).desired_width(40.0));
                        if ui.button("remove").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    self.form.tcp_channel_mapping.remove(i);
                }
                if ui.button("add mapping").clicked() {
                    self.form
                        .tcp_channel_mapping
                        .push((String::new(), String::new()));
                }
            });
        }

        ui.separator();
        self.render_status(ui);
        action
    }

    fn render_status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (dot, text) = match self.state {
                ConnState::Disconnected => (egui::Color32::GRAY, "disconnected"),
                ConnState::Connecting => (egui::Color32::YELLOW, "connecting"),
                ConnState::Connected => (egui::Color32::GREEN, "connected"),
                ConnState::Running => (egui::Color32::LIGHT_BLUE, "running"),
                ConnState::Poisoned => (egui::Color32::RED, "connection lost"),
            };
            ui.colored_label(dot, "●");
            ui.label(text);
            if let Some(facts) = &self.facts {
                ui.separator();
                match facts.stream_rate_hz {
                    Some(hz) => ui.label(format!("stream {hz:.0} Hz")),
                    None => ui.label("no stream"),
                };
                ui.separator();
                ui.label(format!("{} signals", facts.signals.len()));
            }
            if let Some(load) = &self.settings_load {
                ui.separator();
                ui.label(format!(
                    "settings: {} ({}, {})",
                    file_name(&load.path),
                    load.by,
                    clock(load.at)
                ))
                .on_hover_text(load.path.display().to_string());
            }
            if let Some(load) = &self.layout_load {
                ui.separator();
                ui.label(format!(
                    "layout: {} ({}, {})",
                    file_name(&load.path),
                    load.by,
                    clock(load.at)
                ))
                .on_hover_text(load.path.display().to_string());
            }
            if let Some(e) = &self.error {
                ui.separator();
                ui.colored_label(egui::Color32::RED, e);
            }
        });

        if self.connected() {
            ui.horizontal(|ui| {
                if self.readouts.is_empty() {
                    ui.label(egui::RichText::new("no readouts").weak());
                }
                for r in &self.readouts {
                    ui.label(format_readout(&r.name, r.value));
                    ui.add_space(12.0);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.toggle_value(&mut self.show_capabilities, "capabilities");
                    ui.toggle_value(&mut self.show_signals, "signals");
                });
            });
        }

        if self.show_signals
            && let Some(facts) = &self.facts
        {
            egui::ScrollArea::vertical()
                .max_height(160.0)
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
        }

        if self.show_capabilities && !self.capabilities.is_empty() {
            let mut caps: Vec<String> =
                self.capabilities.iter().map(|c| format!("{c:?}")).collect();
            caps.sort();
            ui.label(caps.join(", "));
        }
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

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
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
