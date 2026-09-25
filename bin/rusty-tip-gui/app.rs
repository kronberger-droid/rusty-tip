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

use rusty_tip::event::{ChannelForwarder, Event, EventBus, EventEmitter};
use rusty_tip::experiment_log::reader::Log;
use rusty_tip::routine::Outcome;
use rusty_tip::session::{self, ConnState, Job, JobCx, SessionCmd, SessionHandle, SessionUpdate};
use rusty_tip::shutdown::ShutdownFlag;

use crate::connection::{ConnectionForm, ConnectionPane, PaneAction, status_dot};
use crate::run_view::RunView;
use crate::tools::{self, SetupCx, Tool};

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
    Finished(Outcome),
    Error(String),
    /// A log opened from the History tab.
    Replay(PathBuf),
}

/// A job in flight.
struct ActiveRun {
    tool_id: &'static str,
    shutdown: ShutdownFlag,
    events: Receiver<Event>,
    /// A tool that needs no connection runs here rather than in the session.
    worker: Option<std::thread::JoinHandle<Result<Outcome, String>>>,
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
    theme: String,
    tab: Tab,
    /// `"connection"` or a tool id.
    page: String,
    tools: BTreeMap<String, serde_json::Value>,
}

const PREFS_KEY: &str = "rusty-tip-workbench";

pub struct WorkbenchApp {
    session: SessionHandle,
    pane: ConnectionPane,
    tools: Vec<Box<dyn Tool>>,
    /// The tool whose tabs the middle shows, and whose job Start runs.
    selected: usize,
    page: Page,
    tab: Tab,
    run: Option<ActiveRun>,
    status: RunStatus,
    view: RunView,
    theme: egui::ThemePreference,
    message: Option<(String, bool)>,
    log_lines: Vec<String>,
    log_rx: Receiver<String>,
    history: Vec<HistoryEntry>,
}

struct HistoryEntry {
    path: PathBuf,
    tool: String,
    outcome: String,
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
        let (page, selected) = match tools.iter().position(|t| t.id() == prefs.page) {
            Some(i) => (Page::Tool(i), i),
            None => (Page::Connection, 0),
        };
        let theme = match prefs.theme.as_str() {
            "light" => egui::ThemePreference::Light,
            "dark" => egui::ThemePreference::Dark,
            _ => egui::ThemePreference::System,
        };
        let pane = ConnectionPane::new(prefs.connection);
        let session = session::spawn(pane.form.log_dir());
        Self {
            session,
            pane,
            tools,
            selected,
            page,
            tab: prefs.tab,
            run: None,
            status: RunStatus::Idle,
            view: RunView::default(),
            theme,
            message: None,
            log_lines: Vec::new(),
            log_rx,
            history: Vec::new(),
        }
    }

    fn running(&self) -> bool {
        self.run.is_some()
    }

    fn send(&mut self, cmd: SessionCmd) {
        if let Err(e) = self.session.send(cmd) {
            self.message = Some((e.to_string(), true));
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
            if let Some(worker) = &run.worker
                && worker.is_finished()
            {
                let worker = self.run.as_mut().unwrap().worker.take().unwrap();
                let result = match worker.join() {
                    Ok(result) => result,
                    Err(_) => Err("the worker thread panicked".into()),
                };
                self.finish(result);
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
        self.status = match result {
            Ok(outcome) => {
                self.message = Some((format!("Run finished: {}", outcome_text(outcome)), false));
                RunStatus::Finished(outcome)
            }
            Err(e) => {
                self.message = Some((format!("Run failed: {e}"), true));
                RunStatus::Error(e)
            }
        };
    }

    fn start(&mut self) {
        let tool = &self.tools[self.selected];
        let job = match tool.job() {
            Ok(job) => job,
            Err(e) => {
                self.message = Some((format!("Cannot start: {e}"), true));
                self.tab = Tab::Setup;
                return;
            }
        };
        let tool_id = tool.id();
        let shutdown = ShutdownFlag::new();
        let (tx, rx) = unbounded();
        self.view = RunView::default();
        self.status = RunStatus::Running;
        self.message = None;
        self.tab = Tab::Run;

        let worker = if tool.needs_connection() {
            self.send(SessionCmd::Run {
                job,
                shutdown: shutdown.clone(),
                events: tx,
            });
            None
        } else {
            Some(run_offline(job, shutdown.clone(), tx))
        };
        self.run = Some(ActiveRun {
            tool_id,
            shutdown,
            events: rx,
            worker,
        });
    }

    fn stop(&mut self) {
        if let Some(run) = &self.run {
            run.shutdown.request();
            self.message = Some(("Stop requested…".into(), false));
        }
    }

    fn refresh_history(&mut self) {
        self.history.clear();
        let Some(dir) = self.pane.form.log_dir() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        paths.sort();
        paths.reverse();
        for path in paths {
            let (tool, outcome) = match Log::read(&path) {
                Ok(log) => (
                    log.header().map(|h| h.tool.clone()).unwrap_or_default(),
                    log.finished()
                        .map(|(o, _, _)| o.to_string())
                        .unwrap_or_else(|| "cut off".into()),
                ),
                Err(e) => (String::new(), format!("unreadable: {e}")),
            };
            self.history.push(HistoryEntry {
                path,
                tool,
                outcome,
            });
        }
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
                    self.selected = i;
                    self.page = Page::Tool(i);
                }
                self.view = view;
                self.status = RunStatus::Replay(path);
                self.tab = Tab::Run;
            }
            Err(e) => self.message = Some((format!("Cannot open {}: {e}", path.display()), true)),
        }
    }

    /// A loaded file's connection tables: taken into the Connection page
    /// when disconnected, mentioned when not.
    fn import_connection(&mut self, settings: crate::connection::ConnectionSettings) {
        if settings == self.pane.form.settings() {
            return;
        }
        if self.pane.state != ConnState::Disconnected {
            self.message = Some((
                "The file's connection settings differ from the Connection page; \
                 disconnect and load again to take them"
                    .into(),
                false,
            ));
            return;
        }
        let log_dir_before = self.pane.form.log_dir();
        self.pane.form.apply_settings(&settings);
        if self.pane.form.log_dir() != log_dir_before {
            self.send(SessionCmd::SetLogDir(self.pane.form.log_dir()));
        }
        self.message = Some((
            "Connection page updated from the file's connection tables".into(),
            false,
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

    fn render_top(&mut self, ui: &mut egui::Ui) {
        let running = self.running();
        let action = self.pane.render_bar(ui, running);
        self.apply_pane_action(action);
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
            let label = self.tools[i].label().to_string();
            let is_running = running_id == Some(self.tools[i].id());
            let text = if is_running {
                format!("▶ {label}")
            } else {
                format!("   {label}")
            };
            if ui
                .selectable_label(self.page == Page::Tool(i), text)
                .clicked()
            {
                self.page = Page::Tool(i);
                self.selected = i;
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
                .on_hover_text("Light prints far better than dark; the plots pick higher-contrast colours to match.");
            ui.label("Theme");
        });
    }

    fn render_center(&mut self, ui: &mut egui::Ui) {
        if self.page == Page::Connection {
            let running = self.running();
            let action = egui::ScrollArea::vertical()
                .show(ui, |ui| self.pane.render_page(ui, running))
                .inner;
            self.apply_pane_action(action);
            return;
        }
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
                ui.label(egui::RichText::new(self.tools[self.selected].label()).strong());
            });
        });
        ui.separator();
        match self.tab {
            Tab::Setup => {
                let mut cx = SetupCx {
                    connection: self.pane.form.settings(),
                    import: None,
                };
                self.tools[self.selected].setup(ui, &mut cx);
                if let Some(settings) = cx.import {
                    self.import_connection(settings);
                }
            }
            Tab::Run => self.render_run(ui),
            Tab::History => self.render_history(ui),
        }
    }

    fn render_run(&mut self, ui: &mut egui::Ui) {
        let running = self.running();
        let selected_running = self
            .run
            .as_ref()
            .is_some_and(|r| r.tool_id == self.tools[self.selected].id());
        let can_start = !running
            && (self.pane.state == ConnState::Connected
                || !self.tools[self.selected].needs_connection());

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
                self.start();
            }
            if ui
                .add_enabled(selected_running, egui::Button::new("Stop"))
                .clicked()
            {
                self.stop();
            }
            ui.separator();
            let text = match &self.status {
                RunStatus::Idle => "ready".to_string(),
                RunStatus::Running => "running".to_string(),
                RunStatus::Finished(o) => outcome_text(*o),
                RunStatus::Error(_) => "error".to_string(),
                RunStatus::Replay(p) => format!(
                    "replay of {}",
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ),
            };
            match &self.status {
                RunStatus::Error(e) => {
                    ui.colored_label(egui::Color32::RED, text).on_hover_text(e);
                }
                _ => {
                    ui.label(text);
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
        if let Some((msg, is_error)) = &self.message {
            if *is_error {
                ui.colored_label(egui::Color32::RED, msg);
            } else {
                ui.label(msg);
            }
        }
        ui.add_space(6.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            // The tool's own view first; its series names are its own.
            let view = &self.view;
            self.tools[self.selected].panel(ui, view);

            ui.add_space(8.0);
            ui.collapsing("Log", |ui| {
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("run events").weak());
                        for line in &self.view.tail {
                            ui.label(egui::RichText::new(line).monospace().size(11.0));
                        }
                        if !self.log_lines.is_empty() {
                            ui.separator();
                            ui.label(egui::RichText::new("process log").weak());
                            for line in &self.log_lines {
                                ui.label(egui::RichText::new(line).monospace().size(11.0));
                            }
                        }
                        if self.view.tail.is_empty() && self.log_lines.is_empty() {
                            ui.label(egui::RichText::new("nothing yet").weak());
                        }
                    });
                if ui.button("Clear process log").clicked() {
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
                    ui.label(egui::RichText::new("file").strong());
                    ui.label(egui::RichText::new("tool").strong());
                    ui.label(egui::RichText::new("outcome").strong());
                    ui.label("");
                    ui.end_row();
                    for entry in &self.history {
                        ui.label(
                            entry
                                .path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        );
                        ui.label(&entry.tool);
                        ui.label(&entry.outcome);
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
        ctx.request_repaint_after(Duration::from_millis(100));

        egui::TopBottomPanel::top("connection")
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                self.render_top(ui);
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
            theme: match self.theme {
                egui::ThemePreference::Light => "light",
                egui::ThemePreference::Dark => "dark",
                egui::ThemePreference::System => "system",
            }
            .into(),
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

/// Run a job that needs no controller on a thread of its own, with an
/// event bus that only forwards to the window.
fn run_offline(
    mut job: Box<dyn Job>,
    shutdown: ShutdownFlag,
    events: crossbeam_channel::Sender<Event>,
) -> std::thread::JoinHandle<Result<Outcome, String>> {
    std::thread::spawn(move || {
        let mut bus = EventBus::new();
        bus.add_observer(Box::new(ChannelForwarder::new(events)));
        let registry = rusty_tip::SignalRegistry::default();
        let started = std::time::Instant::now();
        bus.emit(Event::run_started(
            rusty_tip::experiment_log::RunHeader::new(
                job.log_schema(),
                job.header_config(),
                serde_json::Value::Null,
            ),
        ));
        let mut none = rusty_tip::mock_controller::MockController::builder().build();
        let result = job.run(JobCx {
            controller: &mut none,
            registry: &registry,
            events: &bus,
            shutdown: &shutdown,
        });
        let (outcome, detail) = match &result {
            Ok(o) => (outcome_text(*o), None),
            Err(e) => ("error".to_string(), Some(e.to_string())),
        };
        bus.emit(Event::run_finished(&outcome, detail, started.elapsed()));
        result.map_err(|e| e.to_string())
    })
}

fn outcome_text(outcome: Outcome) -> String {
    match outcome {
        Outcome::Completed => "completed".into(),
        Outcome::StoppedByUser => "stopped by user".into(),
        Outcome::CycleLimit(n) => format!("cycle limit ({n})"),
        Outcome::TimedOut(d) => format!("timed out ({:.0} s)", d.as_secs_f64()),
    }
}
