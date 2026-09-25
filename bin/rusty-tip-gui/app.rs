//! The window: a connection bar on top, a tree down the side (Connection,
//! then the tools), and in the middle either the Connection page or the
//! selected tool's Setup / Run / History.
//!
//! The GUI thread never touches the controller. It sends [`SessionCmd`]s to
//! the session thread, mirrors its [`SessionUpdate`]s into the connection
//! pane, and folds the running job's events into a [`RunView`] each frame.
//! Stop is the job's [`ShutdownFlag`].

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use crossbeam_channel::{Receiver, unbounded};
use eframe::egui;
use serde::{Deserialize, Serialize};

use rusty_tip::event::Event;
use rusty_tip::experiment_log::reader::{self, Log, LogEntry};
use rusty_tip::routine::Outcome;
use rusty_tip::session::{self, ConnState, SessionCmd, SessionHandle, SessionUpdate};
use rusty_tip::shutdown::ShutdownFlag;

use crate::connection::{ConnectionForm, ConnectionPane, ConnectionSettings, PaneAction};
use crate::run_view::RunView;
use crate::tools::{self, SetupCx, Tool};
use crate::widgets::{Note, note, status_dot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
enum Tab {
    Setup,
    #[default]
    Run,
    History,
}

/// What the Run tab reports about the last or current job.
#[derive(Debug, Clone, PartialEq)]
enum RunStatus {
    Idle,
    Running,
    Finished(Result<Outcome, String>),
    /// A log opened from the History tab.
    Replay(PathBuf),
}

/// A job in flight.
struct ActiveRun {
    tool_id: &'static str,
    shutdown: ShutdownFlag,
    events: Receiver<Event>,
}

/// What the middle shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Connection,
    Tool(usize),
}

/// Saved between starts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Prefs {
    connection: ConnectionForm,
    theme: egui::ThemePreference,
    tab: Tab,
    /// `"connection"` or a tool id.
    page: String,
    tools: BTreeMap<String, serde_json::Value>,
}

const PREFS_KEY: &str = "rusty-tip-workbench";

/// How often the window redraws on its own, by what could have changed.
fn repaint_after(state: ConnState, running: bool) -> Duration {
    Duration::from_millis(match (running, state) {
        (true, _) => 100,
        (false, ConnState::Connected) => 250,
        _ => 1000,
    })
}

pub struct WorkbenchApp {
    session: SessionHandle,
    pane: ConnectionPane,
    tools: Vec<Box<dyn Tool>>,
    page: Page,
    tab: Tab,
    run: Option<ActiveRun>,
    status: RunStatus,
    view: RunView,
    theme: egui::ThemePreference,
    message: Option<Note>,
    log_lines: Vec<String>,
    log_rx: Receiver<String>,
    history: Vec<LogEntry>,
}

impl WorkbenchApp {
    pub fn new(cc: &eframe::CreationContext<'_>, log_rx: Receiver<String>) -> Self {
        let prefs: Prefs = cc
            .storage
            .and_then(|s| eframe::get_value(s, PREFS_KEY))
            .unwrap_or_default();
        let mut tools = tools::all();
        for tool in &mut tools {
            if let Some(saved) = prefs.tools.get(tool.id()) {
                tool.restore(saved);
            }
        }
        let page = tools
            .iter()
            .position(|t| t.id() == prefs.page)
            .map_or(Page::Connection, Page::Tool);
        let pane = ConnectionPane::new(prefs.connection);
        let session = session::spawn(pane.form.log_dir());
        Self {
            session,
            pane,
            tools,
            page,
            tab: prefs.tab,
            run: None,
            status: RunStatus::Idle,
            view: RunView::default(),
            theme: prefs.theme,
            message: None,
            log_lines: Vec::new(),
            log_rx,
            history: Vec::new(),
        }
    }

    fn running(&self) -> bool {
        self.run.is_some()
    }

    /// The tool the middle shows, if a tool page is up.
    fn tool(&self) -> Option<usize> {
        match self.page {
            Page::Tool(i) => Some(i),
            Page::Connection => None,
        }
    }

    fn send(&mut self, cmd: SessionCmd) {
        if let Err(e) = self.session.send(cmd) {
            self.message = Some(Note::err(e.to_string()));
        }
    }

    /// Pull everything the other threads produced since the last frame.
    fn drain(&mut self) {
        while let Ok(line) = self.log_rx.try_recv() {
            self.log_lines.push(line);
            if self.log_lines.len() > 1000 {
                self.log_lines.drain(0..200);
            }
        }
        for update in self.session.drain() {
            self.pane.apply(&update);
            if let SessionUpdate::JobFinished(result) = update {
                self.finish(result);
            }
        }
        if let Some(run) = &self.run {
            while let Ok(event) = run.events.try_recv() {
                self.view.apply_event(&event);
            }
        }
    }

    fn finish(&mut self, result: Result<Outcome, String>) {
        // Whatever is still in the channel belongs to this run.
        if let Some(run) = self.run.take() {
            while let Ok(event) = run.events.try_recv() {
                self.view.apply_event(&event);
            }
        }
        self.message = Some(match &result {
            Ok(outcome) => Note::ok(format!("Run finished: {}", outcome_text(*outcome))),
            Err(e) => Note::err(format!("Run failed: {e}")),
        });
        self.status = RunStatus::Finished(result);
    }

    fn start(&mut self, tool: usize) {
        let job = match self.tools[tool].job() {
            Ok(job) => job,
            Err(e) => {
                self.message = Some(Note::err(format!("Cannot start: {e}")));
                self.tab = Tab::Setup;
                return;
            }
        };
        let shutdown = ShutdownFlag::new();
        let (tx, rx) = unbounded();
        self.view = RunView::default();
        self.status = RunStatus::Running;
        self.message = None;
        self.tab = Tab::Run;
        self.send(SessionCmd::Run {
            job,
            shutdown: shutdown.clone(),
            events: tx,
        });
        self.run = Some(ActiveRun {
            tool_id: self.tools[tool].id(),
            shutdown,
            events: rx,
        });
    }

    fn stop(&mut self) {
        if let Some(run) = &self.run {
            run.shutdown.request();
            self.message = Some(Note::ok("Stop requested…"));
        }
    }

    fn refresh_history(&mut self) {
        self.history = match self.pane.form.log_dir() {
            Some(dir) => reader::list_dir(&dir).unwrap_or_default(),
            None => Vec::new(),
        };
    }

    fn open_log(&mut self, path: PathBuf) {
        match Log::read(&path) {
            Ok(log) => {
                let mut view = RunView::default();
                for record in &log.records {
                    view.apply(record);
                }
                if let Some(tool) = view.header.as_ref().map(|h| h.tool.clone())
                    && let Some(i) = self.tools.iter().position(|t| t.id() == tool)
                {
                    self.page = Page::Tool(i);
                }
                self.view = view;
                self.status = RunStatus::Replay(path);
                self.tab = Tab::Run;
            }
            Err(e) => {
                self.message = Some(Note::err(format!("Cannot open {}: {e}", path.display())));
            }
        }
    }

    /// A loaded file's connection tables: taken into the Connection page
    /// when disconnected, mentioned when not.
    fn import_connection(&mut self, settings: ConnectionSettings) {
        if self.pane.form.settings().ok().as_ref() == Some(&settings) {
            return;
        }
        if self.pane.state != ConnState::Disconnected {
            self.message = Some(Note::ok(
                "The file's connection settings differ from the Connection page; \
                 disconnect and load again to take them",
            ));
            return;
        }
        let log_dir_before = self.pane.form.log_dir();
        self.pane.form.apply_settings(&settings);
        if self.pane.form.log_dir() != log_dir_before {
            self.send(SessionCmd::SetLogDir(self.pane.form.log_dir()));
        }
        self.message = Some(Note::ok(
            "Connection page updated from the file's connection tables",
        ));
    }

    // -- Rendering --

    fn apply_pane_action(&mut self, action: Option<PaneAction>) {
        match action {
            Some(PaneAction::Connect(backend)) => self.send(SessionCmd::Connect(backend)),
            Some(PaneAction::Disconnect) => self.send(SessionCmd::Disconnect),
            Some(PaneAction::Reconnect) => self.send(SessionCmd::Reconnect),
            Some(PaneAction::ReloadPresets) => self.send(SessionCmd::ReloadPresets),
            Some(PaneAction::LogDir(dir)) => self.send(SessionCmd::SetLogDir(dir)),
            None => {}
        }
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            status_dot(ui, self.pane.state_color());
            if ui
                .selectable_label(self.page == Page::Connection, "Connection")
                .clicked()
            {
                self.page = Page::Connection;
            }
        });
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Tools").strong());
        let running_id = self.run.as_ref().map(|r| r.tool_id);
        for i in 0..self.tools.len() {
            let marker = if running_id == Some(self.tools[i].id()) {
                "▶ "
            } else {
                "   "
            };
            let text = format!("{marker}{}", self.tools[i].label());
            if ui
                .selectable_label(self.page == Page::Tool(i), text)
                .clicked()
            {
                self.page = Page::Tool(i);
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            egui::ComboBox::from_id_salt("theme")
                .selected_text(match self.theme {
                    egui::ThemePreference::Light => "Light",
                    egui::ThemePreference::Dark => "Dark",
                    egui::ThemePreference::System => "System",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.theme, egui::ThemePreference::System, "System");
                    ui.selectable_value(&mut self.theme, egui::ThemePreference::Light, "Light");
                    ui.selectable_value(&mut self.theme, egui::ThemePreference::Dark, "Dark");
                })
                .response
                .on_hover_text(
                    "Light prints far better than dark; the plots pick higher-contrast \
                     colours to match.",
                );
            ui.label("Theme");
        });
    }

    fn render_center(&mut self, ui: &mut egui::Ui) {
        let Some(tool) = self.tool() else {
            let running = self.running();
            let action = egui::ScrollArea::vertical()
                .show(ui, |ui| self.pane.render_page(ui, running))
                .inner;
            self.apply_pane_action(action);
            return;
        };
        ui.horizontal(|ui| {
            for (tab, label) in [
                (Tab::Setup, "Setup"),
                (Tab::Run, "Run"),
                (Tab::History, "History"),
            ] {
                if ui.selectable_label(self.tab == tab, label).clicked() {
                    self.tab = tab;
                    if tab == Tab::History {
                        self.refresh_history();
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(self.tools[tool].label()).strong());
            });
        });
        ui.separator();
        match self.tab {
            Tab::Setup => {
                let mut cx = SetupCx {
                    connection: self.pane.form.settings(),
                    import: None,
                };
                self.tools[tool].setup(ui, &mut cx);
                if let Some(settings) = cx.import {
                    self.import_connection(settings);
                }
            }
            Tab::Run => self.render_run(ui, tool),
            Tab::History => self.render_history(ui),
        }
    }

    fn render_run(&mut self, ui: &mut egui::Ui, tool: usize) {
        let running = self.running();
        let this_running = self
            .run
            .as_ref()
            .is_some_and(|r| r.tool_id == self.tools[tool].id());
        let can_start = !running && self.pane.state == ConnState::Connected;

        ui.horizontal(|ui| {
            if ui
                .add_enabled(can_start, egui::Button::new("Start"))
                .on_disabled_hover_text(if running {
                    "A job is running"
                } else {
                    "Connect first"
                })
                .clicked()
            {
                self.start(tool);
            }
            if ui
                .add_enabled(this_running, egui::Button::new("Stop"))
                .clicked()
            {
                self.stop();
            }
            ui.separator();
            match &self.status {
                RunStatus::Idle => {
                    ui.label("ready");
                }
                RunStatus::Running => {
                    ui.label("running");
                }
                RunStatus::Finished(Ok(outcome)) => {
                    ui.label(outcome_text(*outcome));
                }
                RunStatus::Finished(Err(e)) => {
                    ui.colored_label(egui::Color32::RED, "error")
                        .on_hover_text(e);
                }
                RunStatus::Replay(path) => {
                    ui.label(format!("replay of {}", file_name(path)));
                }
            }
            if let Some((action, depth)) = &self.view.current_action {
                ui.separator();
                ui.label(format!("{}{action}", "  ".repeat(*depth)));
            }
            if let Some(elapsed) = self.view.elapsed_s() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{elapsed:.0} s"));
                });
            }
        });
        note(ui, &self.message);
        ui.add_space(6.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            // The tool's own view first; its series names are its own.
            self.tools[tool].panel(ui, &self.view);

            ui.add_space(8.0);
            ui.collapsing("Run events", |ui| {
                log_lines(ui, "run_events", self.view.tail.iter().map(String::as_str));
            });
            ui.collapsing("Process log", |ui| {
                log_lines(ui, "process_log", self.log_lines.iter().map(String::as_str));
                if ui.button("Clear").clicked() {
                    self.log_lines.clear();
                }
            });
        });
    }

    fn render_history(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match self.pane.form.log_dir() {
                Some(dir) => ui.label(format!("logs in {}", dir.display())),
                None => ui.label("no log directory set"),
            };
            if ui.button("Refresh").clicked() {
                self.refresh_history();
            }
        });
        ui.add_space(4.0);
        let mut open = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("history")
                .num_columns(4)
                .striped(true)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for h in ["file", "tool", "outcome", ""] {
                        ui.label(egui::RichText::new(h).strong());
                    }
                    ui.end_row();
                    for entry in &self.history {
                        ui.label(file_name(&entry.path));
                        ui.label(entry.tool.as_deref().unwrap_or("(no header)"));
                        ui.label(match &entry.finished {
                            Some((outcome, Some(detail), _)) => format!("{outcome} ({detail})"),
                            Some((outcome, None, _)) => outcome.clone(),
                            None => "cut off".into(),
                        });
                        if ui.button("Open").clicked() {
                            open = Some(entry.path.clone());
                        }
                        ui.end_row();
                    }
                });
            if self.history.is_empty() {
                ui.label(egui::RichText::new("no logs").weak());
            }
        });
        if let Some(path) = open {
            self.open_log(path);
        }
    }
}

impl eframe::App for WorkbenchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        ctx.set_theme(self.theme);
        ctx.request_repaint_after(repaint_after(self.pane.state, self.running()));

        egui::TopBottomPanel::top("connection")
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                let running = self.running();
                let action = self.pane.render_bar(ui, running);
                self.apply_pane_action(action);
                ui.add_space(4.0);
            });
        egui::SidePanel::left("tools")
            .resizable(false)
            .default_width(130.0)
            .show(ctx, |ui| self.render_sidebar(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.render_center(ui));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let prefs = Prefs {
            connection: self.pane.form.clone(),
            theme: self.theme,
            tab: self.tab,
            page: match self.page {
                Page::Connection => "connection".to_string(),
                Page::Tool(i) => self.tools[i].id().to_string(),
            },
            tools: self
                .tools
                .iter()
                .map(|t| (t.id().to_string(), t.prefs()))
                .collect(),
        };
        eframe::set_value(storage, PREFS_KEY, &prefs);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // A job still running is asked to stop; the session thread then
        // disconnects on its way out.
        if let Some(run) = &self.run {
            run.shutdown.request();
        }
    }
}

/// A scrolling list of monospace lines, laying out only the visible ones.
fn log_lines<'a>(ui: &mut egui::Ui, id: &str, lines: impl Iterator<Item = &'a str>) {
    let lines: Vec<&str> = lines.collect();
    if lines.is_empty() {
        ui.label(egui::RichText::new("nothing yet").weak());
        return;
    }
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(220.0)
        .stick_to_bottom(true)
        .show_rows(ui, row_height, lines.len(), |ui, rows| {
            for line in &lines[rows] {
                ui.label(egui::RichText::new(*line).monospace().size(11.0));
            }
        });
}

/// The outcome as the log spells it, with underscores as spaces.
fn outcome_text(outcome: Outcome) -> String {
    let (name, detail) = outcome.log_name();
    let name = name.replace('_', " ");
    match detail {
        Some(detail) => format!("{name} ({detail})"),
        None => name,
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
