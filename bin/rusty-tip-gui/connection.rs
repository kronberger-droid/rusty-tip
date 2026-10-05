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

use std::path::PathBuf;
use std::time::SystemTime;

use eframe::egui;
use log::LevelFilter;
use serde::{Deserialize, Serialize};

use rusty_tip::config::{MotorZApproach, TcpChannelMapping};
use rusty_tip::nanonis_controller::CoarseMotor;
use rusty_tip::session::{
    Backend, ConnState, NanonisBackend, PresetLoad, SessionStatus, SessionUpdate,
};

use crate::units::{format_si, number};
use crate::widgets::{Tone, path_field, section, stat, status_dot, toned, toned_if};

/// Width of the label column on the Connection page, one for both
/// sections and fixed, so the fields do not move when the backend changes
/// which rows there are.
const LABEL_WIDTH: f32 = 150.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BackendKind {
    #[default]
    Nanonis,
    Mock,
}

/// Whether the window serves the control socket, and what it lets through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentSocket {
    Off,
    /// Answers what only reads; refuses everything that acts.
    #[default]
    ReadOnly,
    Full,
}

impl AgentSocket {
    pub const ALL: [AgentSocket; 3] = [AgentSocket::Off, AgentSocket::ReadOnly, AgentSocket::Full];

    pub fn label(self) -> &'static str {
        match self {
            AgentSocket::Off => "off",
            AgentSocket::ReadOnly => "read-only",
            AgentSocket::Full => "full",
        }
    }
}

fn default_agent_addr() -> String {
    rusty_tip::control::DEFAULT_ADDR.into()
}

fn default_log_level() -> LevelFilter {
    LevelFilter::Info
}

fn default_motor_group() -> u8 {
    1
}

/// The group as a number, or as the text a form saved before it was one.
fn group_number_or_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Group {
        Number(u8),
        Text(String),
    }
    match Group::deserialize(d)? {
        Group::Number(n) => Ok(n),
        Group::Text(s) => s.trim().parse().map_err(serde::de::Error::custom),
    }
}

/// The form, as saved between starts. Strings where the operator types, so
/// a half-typed port never fails to persist. Fields added since the first
/// release default, so a saved form from before them still loads.
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
    /// Coarse motor group, 1 to 6.
    #[serde(
        default = "default_motor_group",
        deserialize_with = "group_number_or_text"
    )]
    pub motor_group: u8,
    /// Which coarse Z direction approaches the sample.
    #[serde(default)]
    pub motor_z_approach: MotorZApproach,
    pub log_dir: String,
    /// How much the activity log says. The pane's setting, kept between
    /// starts; it is about this window, not the connection, so a config
    /// file neither carries it nor overrides it.
    #[serde(default = "default_log_level")]
    pub log_level: LevelFilter,
    /// The control socket `rusty-tip` talks to. Read-only unless set
    /// otherwise, so a command line can watch but not move by default.
    #[serde(default)]
    pub agent_socket: AgentSocket,
    #[serde(default = "default_agent_addr")]
    pub agent_addr: String,
    /// The controller preset file; a saved form from before it had one
    /// gets the default.
    #[serde(default = "default_presets_file")]
    pub presets_file: String,
    /// The operating points file; a saved form from before it had one gets
    /// the default.
    #[serde(default = "default_operating_points_file")]
    pub operating_points_file: String,
}

/// Next to the config and the log directory, where the app is launched.
fn default_presets_file() -> String {
    rusty_tip::config::DEFAULT_PRESETS_FILE.into()
}

fn default_operating_points_file() -> String {
    rusty_tip::operating_point::DEFAULT_OPERATING_POINTS_FILE.into()
}

impl Default for ConnectionForm {
    fn default() -> Self {
        let mut form = Self {
            kind: BackendKind::Nanonis,
            host: String::new(),
            port: String::new(),
            data_port: String::new(),
            sample_rate_hz: String::new(),
            layout_file: String::new(),
            settings_file: String::new(),
            tcp_channel_mapping: Vec::new(),
            motor_group: default_motor_group(),
            motor_z_approach: MotorZApproach::default(),
            log_dir: "./experiments".into(),
            log_level: default_log_level(),
            agent_socket: AgentSocket::default(),
            agent_addr: default_agent_addr(),
            presets_file: default_presets_file(),
            operating_points_file: default_operating_points_file(),
        };
        form.apply_settings(&ConnectionSettings {
            backend: NanonisBackend::default(),
            log_dir: None,
            presets_file: PathBuf::from(default_presets_file()),
            operating_points_file: PathBuf::from(default_operating_points_file()),
        });
        form
    }
}

/// The connection as a config file carries it: where the Nanonis is,
/// where logs go, and where the controller presets are. What the
/// Connection page edits and a tip-prep file stores, so the CLI and the
/// workbench read the same file.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionSettings {
    pub backend: NanonisBackend,
    pub log_dir: Option<PathBuf>,
    /// The controller preset file (`[controllers].presets_file`).
    pub presets_file: PathBuf,
    /// The operating points file (`[controllers].operating_points_file`).
    pub operating_points_file: PathBuf,
}

/// A path field's path, or the default when it was emptied.
fn or_default(field: &str, default: fn() -> String) -> PathBuf {
    let path = field.trim();
    PathBuf::from(if path.is_empty() {
        default()
    } else {
        path.to_string()
    })
}

impl ConnectionForm {
    /// The Nanonis the form describes, or the first thing wrong with it.
    fn nanonis(&self) -> Result<NanonisBackend, String> {
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .map_err(|_| format!("port {:?} is not a port number", self.port))?;
        let data_port = self
            .data_port
            .trim()
            .parse::<u16>()
            .map_err(|_| format!("data port {:?} is not a port number", self.data_port))?;
        let sample_rate_hz = self
            .sample_rate_hz
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|r| r.is_finite() && *r > 0.0)
            .ok_or_else(|| format!("sample rate {:?} is not a rate in Hz", self.sample_rate_hz))?;
        let mut tcp_channel_mapping = Vec::new();
        for (i, (index, channel)) in self.tcp_channel_mapping.iter().enumerate() {
            let nanonis_index = index.trim().parse::<u8>().map_err(|_| {
                format!("TCP mapping {}: signal index {index:?} is not 0-255", i + 1)
            })?;
            let tcp_channel = channel
                .trim()
                .parse::<u8>()
                .map_err(|_| format!("TCP mapping {}: channel {channel:?} is not 0-23", i + 1))?;
            tcp_channel_mapping.push(TcpChannelMapping {
                nanonis_index,
                tcp_channel,
            });
        }
        let group = CoarseMotor::group_from_number(self.motor_group)
            .ok_or_else(|| format!("coarse motor group {} is not 1 to 6", self.motor_group))?;
        let motor = CoarseMotor {
            group,
            z_approach: self.motor_z_approach,
        };
        let optional = |s: &str| (!s.trim().is_empty()).then(|| PathBuf::from(s.trim()));
        Ok(NanonisBackend {
            host: self.host.trim().to_string(),
            port,
            data_port,
            sample_rate_hz,
            layout_file: optional(&self.layout_file),
            settings_file: optional(&self.settings_file),
            tcp_channel_mapping,
            motor,
        })
    }

    /// The backend the form describes, or the first thing wrong with it.
    pub fn backend(&self) -> Result<Backend, String> {
        match self.kind {
            BackendKind::Mock => Ok(Backend::Mock),
            BackendKind::Nanonis => Ok(Backend::Nanonis(self.nanonis()?)),
        }
    }

    pub fn log_dir(&self) -> Option<PathBuf> {
        let dir = self.log_dir.trim();
        (!dir.is_empty()).then(|| PathBuf::from(dir))
    }

    /// The form as settings for a config file. The Nanonis fields go out
    /// as typed even with the mock selected: a file says where the Nanonis
    /// is, not whether to use the mock.
    pub fn settings(&self) -> Result<ConnectionSettings, String> {
        Ok(ConnectionSettings {
            backend: self.nanonis()?,
            log_dir: self.log_dir(),
            presets_file: self.presets_file(),
            operating_points_file: self.operating_points_file(),
        })
    }

    /// The preset file, the default when the field was emptied.
    pub fn presets_file(&self) -> PathBuf {
        or_default(&self.presets_file, default_presets_file)
    }

    /// The operating points file, the default when the field was emptied.
    pub fn operating_points_file(&self) -> PathBuf {
        or_default(&self.operating_points_file, default_operating_points_file)
    }

    /// Take a file's settings into the form. The backend kind stays.
    pub fn apply_settings(&mut self, s: &ConnectionSettings) {
        let b = &s.backend;
        self.host = b.host.clone();
        self.port = b.port.to_string();
        self.data_port = b.data_port.to_string();
        self.sample_rate_hz = format!("{}", b.sample_rate_hz);
        self.layout_file = b
            .layout_file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.settings_file = b
            .settings_file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.tcp_channel_mapping = b
            .tcp_channel_mapping
            .iter()
            .map(|m| (m.nanonis_index.to_string(), m.tcp_channel.to_string()))
            .collect();
        self.motor_group = b.motor.group_number();
        self.motor_z_approach = b.motor.z_approach;
        if let Some(dir) = &s.log_dir {
            self.log_dir = dir.display().to_string();
        }
        self.presets_file = s.presets_file.display().to_string();
        self.operating_points_file = s.operating_points_file.display().to_string();
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
    /// The log level changed.
    LogLevel(LevelFilter),
    /// The agent socket's mode or address changed.
    AgentSocket,
}

pub struct ConnectionPane {
    pub form: ConnectionForm,
    /// What the session reported, mirrored the way the control server
    /// mirrors it.
    pub status: SessionStatus,
    /// Where the agent socket listens, or why it does not.
    pub agent_status: Option<Result<String, String>>,
    /// The mode and address the socket was last set up with, so only a
    /// real change restarts it and cuts off its clients.
    pub agent_applied: Option<(AgentSocket, String)>,
}

impl ConnectionPane {
    pub fn new(form: ConnectionForm) -> Self {
        Self {
            form,
            status: SessionStatus::default(),
            agent_status: None,
            agent_applied: None,
        }
    }

    /// Mirror what the session reported.
    pub fn apply(&mut self, update: &SessionUpdate) {
        self.status.apply(update);
    }

    pub fn connected(&self) -> bool {
        matches!(self.status.state, ConnState::Connected | ConnState::Running)
    }

    /// The state as a word.
    pub fn state_word(&self) -> &'static str {
        match self.status.state {
            ConnState::Disconnected => "disconnected",
            ConnState::Connecting => "connecting",
            ConnState::Connected => "connected",
            ConnState::Running => "running",
            ConnState::Poisoned => "connection lost",
        }
    }

    /// The colour of the status dot: green while a connection is up, red
    /// when it broke, grey otherwise.
    pub fn state_color(&self) -> egui::Color32 {
        match self.status.state {
            ConnState::Connected | ConnState::Running => egui::Color32::from_rgb(52, 168, 83),
            ConnState::Poisoned => egui::Color32::from_rgb(217, 48, 37),
            ConnState::Disconnected | ConnState::Connecting => egui::Color32::from_gray(140),
        }
    }

    /// The bar along the top: are we connected, to what, and the button.
    pub fn render_bar(&mut self, ui: &mut egui::Ui, running: bool) -> Option<PaneAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            status_dot(ui, self.state_color());
            ui.label(format!("{} · {}", self.state_word(), self.form.target()));
            if let Some(facts) = &self.status.facts {
                ui.separator();
                match facts.stream_rate_hz {
                    Some(hz) => ui.label(format!("stream {hz:.0} Hz")),
                    None => ui.label("no stream"),
                };
            }
            if let Some(e) = &self.status.error {
                ui.separator();
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                action = self.render_button(ui, running);
            });
        });
        action
    }

    fn render_button(&mut self, ui: &mut egui::Ui, running: bool) -> Option<PaneAction> {
        let mut action = None;
        match self.status.state {
            ConnState::Disconnected => {
                if ui.add(toned(ui, "Connect", Tone::Primary)).clicked() {
                    match self.form.backend() {
                        Ok(backend) => {
                            self.status.error = None;
                            action = Some(PaneAction::Connect(backend));
                        }
                        Err(e) => self.status.error = Some(e),
                    }
                }
            }
            ConnState::Connecting => {
                ui.add_enabled(false, egui::Button::new("Connecting…"));
            }
            ConnState::Connected | ConnState::Running => {
                if ui
                    .add_enabled(!running, toned_if(ui, !running, "Disconnect", Tone::Danger))
                    .on_disabled_hover_text("Stop the running job first")
                    .clicked()
                {
                    action = Some(PaneAction::Disconnect);
                }
            }
            ConnState::Poisoned => {
                if ui.add(toned(ui, "Reconnect", Tone::Primary)).clicked() {
                    self.status.error = None;
                    action = Some(PaneAction::Reconnect);
                }
                if ui.add(toned(ui, "Disconnect", Tone::Danger)).clicked() {
                    action = Some(PaneAction::Disconnect);
                }
            }
        }
        action
    }

    /// The Connection page: the form, then what the session reports.
    pub fn render_page(&mut self, ui: &mut egui::Ui, running: bool) -> Option<PaneAction> {
        let mut action = None;
        let editable = self.status.state == ConnState::Disconnected;

        section(
            ui,
            "Controller",
            Some(
                "Where the controller is and what is loaded on connect. Connect and \
                  disconnect from the bar at the top; disconnect to change these.",
            ),
        );
        ui.add_enabled_ui(editable, |ui| {
            egui::Grid::new("connection_form")
                .num_columns(2)
                .min_col_width(LABEL_WIDTH)
                .spacing([16.0, 6.0])
                .show(ui, |ui| self.render_form(ui));
        });
        let has_files =
            !(self.form.layout_file.trim().is_empty() && self.form.settings_file.trim().is_empty());
        if self.connected()
            && self.form.kind == BackendKind::Nanonis
            && has_files
            && ui
                .add_enabled(!running, egui::Button::new("Reload files"))
                .on_hover_text("Load the layout and settings files again")
                .clicked()
        {
            action = Some(PaneAction::ReloadPresets);
        }

        section(ui, "Files and logging", None);
        egui::Grid::new("connection_files")
            .num_columns(2)
            .min_col_width(LABEL_WIDTH)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("Log directory");
                ui.horizontal(|ui| {
                    let before = self.form.log_dir.clone();
                    path_field(ui, &mut self.form.log_dir, 320.0, || {
                        rfd::FileDialog::new().pick_folder()
                    });
                    if self.form.log_dir != before {
                        action = Some(PaneAction::LogDir(self.form.log_dir()));
                    }
                });
                ui.end_row();

                ui.label("Log level").on_hover_text(
                    "How much the activity log and the terminal say. Takes effect at once.",
                );
                let before = self.form.log_level;
                egui::ComboBox::from_id_salt("log_level")
                    .selected_text(self.form.log_level.as_str().to_lowercase())
                    .show_ui(ui, |ui| {
                        // Everything but off: a window that logs nothing
                        // cannot say why.
                        for level in LevelFilter::iter().skip(1) {
                            ui.selectable_value(
                                &mut self.form.log_level,
                                level,
                                level.as_str().to_lowercase(),
                            );
                        }
                    });
                if self.form.log_level != before {
                    action = Some(PaneAction::LogLevel(self.form.log_level));
                }
                ui.end_row();

                ui.label("Preset file").on_hover_text(
                    "The controller presets, one TOML file. The Controllers page lists, saves \
                     and applies them, and a tip-prep config names one by name. Created on \
                     the first save.",
                );
                ui.horizontal(|ui| {
                    path_field(ui, &mut self.form.presets_file, 320.0, || {
                        rfd::FileDialog::new()
                            .add_filter("TOML", &["toml"])
                            .pick_file()
                    });
                });
                ui.end_row();

                ui.label("Operating points").on_hover_text(
                    "Every loop with its setpoint, the bias, and the scan's size, angle, \
                     resolution and speed, saved under a name. The Controllers page \
                     captures, lists and applies them. Created on the first save.",
                );
                ui.horizontal(|ui| {
                    path_field(ui, &mut self.form.operating_points_file, 320.0, || {
                        rfd::FileDialog::new()
                            .add_filter("TOML", &["toml"])
                            .pick_file()
                    });
                });
                ui.end_row();

                ui.label("Agent socket").on_hover_text(
                    "Where `rusty-tip` reaches this window's connection, on this machine only. \
                     Read-only answers status and reads and refuses anything that could move \
                     the instrument.",
                );
                ui.horizontal(|ui| {
                    let before = self.form.agent_socket;
                    egui::ComboBox::from_id_salt("agent_socket")
                        .selected_text(self.form.agent_socket.label())
                        .show_ui(ui, |ui| {
                            for mode in AgentSocket::ALL {
                                ui.selectable_value(
                                    &mut self.form.agent_socket,
                                    mode,
                                    mode.label(),
                                );
                            }
                        });
                    let addr = ui.add_enabled(
                        self.form.agent_socket != AgentSocket::Off,
                        egui::TextEdit::singleline(&mut self.form.agent_addr).desired_width(140.0),
                    );
                    // The address applies when editing ends, not on every key.
                    let wanted = (
                        self.form.agent_socket,
                        self.form.agent_addr.trim().to_string(),
                    );
                    if (self.form.agent_socket != before || addr.lost_focus())
                        && self.agent_applied.as_ref() != Some(&wanted)
                    {
                        action = Some(PaneAction::AgentSocket);
                    }
                    match &self.agent_status {
                        Some(Ok(listening)) => {
                            ui.weak(listening);
                        }
                        Some(Err(e)) => {
                            ui.colored_label(ui.visuals().error_fg_color, e);
                        }
                        None => {}
                    }
                });
                ui.end_row();
            });

        if self.status.state != ConnState::Disconnected {
            ui.add_space(12.0);
            ui.separator();
            self.render_details(ui);
        }
        action
    }

    /// The rows of the form grid.
    fn render_form(&mut self, ui: &mut egui::Ui) {
        ui.label("Backend");
        egui::ComboBox::from_id_salt("backend_kind")
            .selected_text(match self.form.kind {
                BackendKind::Nanonis => "Nanonis",
                BackendKind::Mock => "Mock",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.form.kind, BackendKind::Nanonis, "Nanonis");
                ui.selectable_value(&mut self.form.kind, BackendKind::Mock, "Mock")
                    .on_hover_text(
                        "The in-memory mock with a realistic tip model. No hardware is \
                         contacted.",
                    );
            });
        ui.end_row();

        if self.form.kind != BackendKind::Nanonis {
            return;
        }

        ui.label("Host");
        ui.add(egui::TextEdit::singleline(&mut self.form.host).desired_width(200.0));
        ui.end_row();

        ui.label("Command port");
        ui.add(egui::TextEdit::singleline(&mut self.form.port).desired_width(80.0));
        ui.end_row();

        ui.label("Data port");
        ui.add(egui::TextEdit::singleline(&mut self.form.data_port).desired_width(80.0))
            .on_hover_text("The TCP logger's data port, 6590 on a stock install");
        ui.end_row();

        ui.label("Stream rate (Hz)");
        ui.add(egui::TextEdit::singleline(&mut self.form.sample_rate_hz).desired_width(80.0))
            .on_hover_text(
                "What to ask the TCP logger for. It delivers its base rate over an \
                 integer, so the nearest such rate is what arrives.",
            );
        ui.end_row();

        ui.label("Layout file");
        ui.horizontal(|ui| {
            path_field(ui, &mut self.form.layout_file, 320.0, || {
                rfd::FileDialog::new()
                    .add_filter("Nanonis layout", &["lyt"])
                    .pick_file()
            });
        });
        ui.end_row();

        ui.label("Settings file");
        ui.horizontal(|ui| {
            path_field(ui, &mut self.form.settings_file, 320.0, || {
                rfd::FileDialog::new()
                    .add_filter("Nanonis settings", &["ini"])
                    .pick_file()
            });
        })
        .response
        .on_hover_text(
            "Loaded once, on connect, before the stream starts. A load persists for \
             the rest of the session.",
        );
        ui.end_row();

        ui.label("TCP channel mapping");
        ui.vertical(|ui| {
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
            if ui.button("add").clicked() {
                self.form
                    .tcp_channel_mapping
                    .push((String::new(), String::new()));
            }
        })
        .response
        .on_hover_text("Signal index to TCP logger channel, beyond the standard map");
        ui.end_row();

        ui.label("Coarse motor");
        ui.horizontal(|ui| {
            ui.label("group");
            egui::ComboBox::from_id_salt("motor_group")
                .width(50.0)
                .selected_text(self.form.motor_group.to_string())
                .show_ui(ui, |ui| {
                    for n in 1..=6u8 {
                        ui.selectable_value(&mut self.form.motor_group, n, n.to_string());
                    }
                });
            ui.label("approach");
            egui::ComboBox::from_id_salt("motor_z_approach")
                .width(50.0)
                .selected_text(self.form.motor_z_approach.label())
                .show_ui(ui, |ui| {
                    for dir in [MotorZApproach::Plus, MotorZApproach::Minus] {
                        ui.selectable_value(&mut self.form.motor_z_approach, dir, dir.label());
                    }
                });
        })
        .response
        .on_hover_text(
            "The Motor module's group to drive, and which of its Z buttons moves the tip \
             toward the sample. Retracts go the other way.",
        );
        ui.end_row();
    }

    fn render_details(&mut self, ui: &mut egui::Ui) {
        section(ui, "Session", None);
        egui::Grid::new("connection_details")
            .num_columns(2)
            .min_col_width(LABEL_WIDTH)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.label("State");
                ui.label(self.state_word());
                ui.end_row();
                if let Some(facts) = &self.status.facts {
                    ui.label("Stream");
                    ui.label(match facts.stream_rate_hz {
                        Some(hz) => format!("{hz:.0} Hz"),
                        None => "none".into(),
                    });
                    ui.end_row();
                }
                ui.label("Settings file");
                ui.label(describe_load(self.status.settings.as_ref()));
                ui.end_row();
                ui.label("Layout file");
                ui.label(describe_load(self.status.layout.as_ref()));
                ui.end_row();
            });

        if self.connected() {
            section(ui, "Live readouts", None);
            if self.status.readouts.is_empty() {
                ui.label(egui::RichText::new("none").weak());
            }
            ui.horizontal(|ui| {
                for r in &self.status.readouts {
                    let (label, value) = format_readout(&r.name, r.value);
                    stat(
                        ui,
                        &label,
                        egui::RichText::new(value).size(18.0).monospace(),
                    );
                    ui.add_space(20.0);
                }
            });
        }

        if let Some(facts) = &self.status.facts {
            ui.add_space(8.0);
            ui.collapsing(format!("Signals ({})", facts.signals.len()), |ui| {
                let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
                egui::ScrollArea::vertical().max_height(240.0).show_rows(
                    ui,
                    row_height,
                    facts.signals.len(),
                    |ui, rows| {
                        egui::Grid::new("signal_table")
                            .num_columns(3)
                            .striped(true)
                            .spacing([16.0, 2.0])
                            .show(ui, |ui| {
                                for s in &facts.signals[rows] {
                                    ui.label(s.index.to_string());
                                    ui.label(&s.name);
                                    ui.label(
                                        s.tcp_channel
                                            .map(|c| format!("TCP {c}"))
                                            .unwrap_or_default(),
                                    );
                                    ui.end_row();
                                }
                            });
                    },
                );
            });
        }

        if !self.status.capabilities.is_empty() {
            ui.collapsing(
                format!("Capabilities ({})", self.status.capabilities.len()),
                |ui| {
                    let mut caps: Vec<String> = self
                        .status
                        .capabilities
                        .iter()
                        .map(|c| format!("{c:?}"))
                        .collect();
                    caps.sort();
                    ui.label(caps.join(", "));
                },
            );
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

/// A readout in the unit its name declares (`Z (m)`), in the prefix that
/// fits the value; a frequency shift is in hertz by name, anything else is
/// a bare number.
/// A readout as its label, capitalised and without the unit the registry
/// name carries, and its value in that unit.
fn format_readout(name: &str, value: f64) -> (String, String) {
    let short = name.split('(').next().unwrap_or(name).trim();
    let mut chars = short.chars();
    let label = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    let declared = name
        .rsplit_once('(')
        .and_then(|(_, rest)| rest.strip_suffix(')'))
        .map(str::trim)
        .filter(|u| !u.is_empty());
    let shown = match declared {
        Some(unit) => format_si(value, unit),
        None if name.to_lowercase().contains("freq") => format_si(value, "Hz"),
        None => number(value),
    };
    (label, shown)
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
                assert_eq!(b, NanonisBackend::default());
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
        assert!(form.settings().unwrap_err().contains("65a"));
    }

    /// A form saved before the log level was a `LevelFilter` and the
    /// group a number still loads, level and group kept. Prefs are RON,
    /// where the old level was a bare `Debug` and the group a string, and a
    /// field that fails to decode loses the whole form.
    #[test]
    fn a_form_saved_by_an_older_version_still_loads() {
        #[derive(Default)]
        struct Memory(std::collections::HashMap<String, String>);
        impl eframe::Storage for Memory {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn flush(&mut self) {}
        }
        let mut storage = Memory::default();
        eframe::set_value(&mut storage, "form", &ConnectionForm::default());
        let now = storage.0["form"].clone();
        let old = now
            .replace("log_level:INFO", "log_level:Debug")
            .replace("motor_group:1", "motor_group:\"3\"");
        assert!(
            old.contains("log_level:Debug") && old.contains("motor_group:\"3\""),
            "the RON layout changed; update the replacements: {now}"
        );
        storage.0.insert("form".into(), old);

        let form: ConnectionForm = eframe::get_value(&storage, "form").unwrap();
        assert_eq!(form.log_level, LevelFilter::Debug);
        assert_eq!(form.motor_group, 3);
    }

    #[test]
    fn a_bad_motor_group_is_named_in_the_error() {
        let form = ConnectionForm {
            motor_group: 7,
            ..Default::default()
        };
        assert!(form.backend().unwrap_err().contains("7"));
    }

    #[test]
    fn settings_round_trip_through_the_form() {
        let settings = ConnectionSettings {
            backend: NanonisBackend {
                host: "192.168.1.10".into(),
                layout_file: Some(PathBuf::from("a.lyt")),
                tcp_channel_mapping: vec![TcpChannelMapping {
                    nanonis_index: 76,
                    tcp_channel: 3,
                }],
                motor: CoarseMotor {
                    group: nanonis_rs::motor::MotorGroup::Group3,
                    z_approach: MotorZApproach::Minus,
                },
                ..NanonisBackend::default()
            },
            log_dir: Some(PathBuf::from("/tmp/logs")),
            presets_file: PathBuf::from("/lab/presets.toml"),
            operating_points_file: PathBuf::from("/lab/points.toml"),
        };
        let mut form = ConnectionForm {
            kind: BackendKind::Mock,
            ..Default::default()
        };
        form.apply_settings(&settings);
        assert_eq!(
            form.kind,
            BackendKind::Mock,
            "the kind is not a file's to set"
        );
        assert_eq!(form.settings().unwrap(), settings);
        assert_eq!(form.backend().unwrap(), Backend::Mock);
    }

    #[test]
    fn readouts_show_human_units() {
        let readout = |name, value| {
            let (label, shown) = format_readout(name, value);
            format!("{label} {shown}")
        };
        assert_eq!(readout("Z (m)", 12.3e-9), "Z 12.30 nm");
        assert_eq!(readout("Current (A)", 50.1e-12), "Current 50.10 pA");
        assert_eq!(readout("Bias (V)", 0.2), "Bias 200.0 mV");
        assert_eq!(readout("freq shift", -3.2), "Freq shift -3.200 Hz");
        assert_eq!(readout("Phase", 1.5), "Phase 1.5");
    }
}
