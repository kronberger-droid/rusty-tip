//! One connection, many runs.
//!
//! A [`Session`] owns the controller for as long as it is connected and runs
//! [`Job`]s against it one at a time. Connecting loads the layout and
//! settings files, builds the signal registry and starts the data stream;
//! each job then gets `prepare`/`teardown` around it and its own JSONL log,
//! and the connection, the stream and the registry carry over to the next.
//!
//! `Session` is synchronous and knows nothing about threads or windows, so
//! a test can drive it against the mock. [`spawn`] puts one on a thread of
//! its own behind a pair of channels, which is how a GUI uses it: exactly
//! one thread touches the controller, the GUI sends [`SessionCmd`]s and
//! reads [`SessionUpdate`]s, and while no job runs the session polls a few
//! live readouts.
//!
//! ```no_run
//! use rusty_tip::session::{Backend, Session};
//! use rusty_tip::ShutdownFlag;
//! # use rusty_tip::session::Job;
//! # fn tip_prep_job() -> Box<dyn Job> { unimplemented!() }
//!
//! # fn main() -> Result<(), rusty_tip::spm_error::SpmError> {
//! let mut session = Session::new(Some("./experiments".into()));
//! session.connect(&Backend::Mock)?;
//! let shutdown = ShutdownFlag::new();
//! let mut job = tip_prep_job();
//! let first = session.run(&mut *job, &shutdown, Vec::new())?;
//! let second = session.run(&mut *job, &shutdown, Vec::new())?; // no reconnect
//! session.disconnect();
//! # Ok(())
//! # }
//! ```

use std::collections::HashSet;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parking_lot::Mutex;

use crate::config::TcpChannelMapping;
use crate::event::{ChannelForwarder, Event, EventBus, EventEmitter, FileLogger, Observer};
use crate::experiment_log::{ControllerFacts, LogEvent, RunHeader, ToolSchema};
use crate::mock_controller::{MockController, models};
use crate::nanonis_controller::{NanonisController, NanonisSetupConfig, StreamSetup};
use crate::routine::{LayoutLoadedEvent, Outcome, SettingsLoadedEvent, panic_message};
use crate::shutdown::ShutdownFlag;
use crate::signal_registry::{SignalIndex, SignalRegistry};
use crate::spm_controller::{Capability, SpmController};
use crate::spm_error::SpmError;

/// The signals the session polls while idle, by registry name. Fixed for
/// now; a tool cannot yet say which readouts matter to it.
const READOUT_NAMES: [&str; 4] = ["bias", "z", "current", "freq shift"];

/// How often the session thread polls readouts while idle.
const READOUT_PERIOD: Duration = Duration::from_millis(500);

/// A Nanonis controller to connect to.
#[derive(Debug, Clone, PartialEq)]
pub struct NanonisBackend {
    pub host: String,
    /// Command port, 6501 by default.
    pub port: u16,
    /// TCP logger data port, 6590 by default.
    pub data_port: u16,
    /// Stream rate to ask the TCP logger for, in Hz.
    pub sample_rate_hz: f64,
    pub layout_file: Option<PathBuf>,
    pub settings_file: Option<PathBuf>,
    /// Signal index to TCP channel assignments beyond the standard map.
    pub tcp_channel_mapping: Vec<TcpChannelMapping>,
}

impl Default for NanonisBackend {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 6501,
            data_port: 6590,
            sample_rate_hz: 1000.0,
            layout_file: None,
            settings_file: None,
            tcp_channel_mapping: Vec::new(),
        }
    }
}

/// What a session connects to.
#[derive(Debug, Clone, PartialEq)]
pub enum Backend {
    Nanonis(NanonisBackend),
    /// The in-memory mock with the `realistic` tip model and no data stream,
    /// for trying tools without hardware. Seeded, so a run is reproducible.
    Mock,
}

/// The layout and settings files a session loads on connect.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PresetFiles {
    pub layout: Option<PathBuf>,
    pub settings: Option<PathBuf>,
}

/// A settings or layout load that has happened this session. Shown so the
/// next operator knows the controller's state has moved.
#[derive(Debug, Clone, PartialEq)]
pub struct PresetLoad {
    pub path: PathBuf,
    /// `"connect"`, `"reload"`, or the name of the job that loaded it.
    pub by: String,
    pub at: SystemTime,
}

/// Where the session stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Disconnected,
    Connecting,
    Connected,
    /// A job is running; the controller is busy.
    Running,
    /// The connection is broken; [`Session::reconnect`] may fix it.
    Poisoned,
}

/// One live value from the idle poll.
#[derive(Debug, Clone, PartialEq)]
pub struct Readout {
    /// The signal's name as the controller reports it.
    pub name: String,
    pub index: SignalIndex,
    pub value: f64,
}

/// What the session runs.
///
/// A job owns its parameters and does its work against the borrowed
/// controller; the session brackets it with a run log, so a job that is a
/// [`Routine`](crate::routine::Routine) wrapper is a few lines:
///
/// ```ignore
/// impl Job for TipPrepJob {
///     fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
///         let fs = cx.registry.get_by_name("freq shift")
///             .ok_or_else(|| SpmError::Workflow("no freq shift signal".into()))?
///             .signal_index();
///         let mut routine = TipPrep::new(&self.config, fs);
///         run_routine(cx.controller, cx.events, cx.shutdown, &mut routine)
///     }
///     // ...
/// }
/// ```
///
/// A job that is not a routine gets the bare controller and is responsible
/// for its own cleanup. Either way the session writes `run_started` before
/// and, if the job did not (routines do), `run_finished` after.
pub trait Job: Send {
    /// Short identifier: the log file name starts with it.
    fn name(&self) -> &str;

    /// The custom event kinds this job can write, for the run header.
    fn log_schema(&self) -> ToolSchema;

    /// The job's parameters as loaded, for the run header.
    fn header_config(&self) -> serde_json::Value;

    /// Do the work.
    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError>;
}

/// What a [`Job`] runs against.
pub struct JobCx<'a> {
    pub controller: &'a mut dyn SpmController,
    pub registry: &'a SignalRegistry,
    pub events: &'a EventBus,
    pub shutdown: &'a ShutdownFlag,
}

struct Connection {
    controller: Box<dyn SpmController>,
    registry: SignalRegistry,
    files: PresetFiles,
    facts: ControllerFacts,
    settings: Option<PresetLoad>,
    layout: Option<PresetLoad>,
    readouts: Vec<(String, SignalIndex)>,
}

/// A connection and the runs on it. See the [module docs](self).
pub struct Session {
    log_dir: Option<PathBuf>,
    conn: Option<Connection>,
    poisoned: bool,
}

impl Session {
    /// A disconnected session. Each job's JSONL log goes into `log_dir`;
    /// `None` writes no logs.
    pub fn new(log_dir: Option<PathBuf>) -> Self {
        Self {
            log_dir,
            conn: None,
            poisoned: false,
        }
    }

    /// Where the next job's log goes; `None` writes none. Takes effect from
    /// the next run.
    pub fn set_log_dir(&mut self, log_dir: Option<PathBuf>) {
        self.log_dir = log_dir;
    }

    pub fn state(&self) -> ConnState {
        match (&self.conn, self.poisoned) {
            (None, _) => ConnState::Disconnected,
            (Some(_), true) => ConnState::Poisoned,
            (Some(_), false) => ConnState::Connected,
        }
    }

    /// Connect to `backend`: build the controller, load its preset files,
    /// build the registry and, for Nanonis, start the data stream. An
    /// existing connection is closed first.
    pub fn connect(&mut self, backend: &Backend) -> Result<(), SpmError> {
        self.disconnect();
        match backend {
            Backend::Nanonis(nanonis) => self.connect_nanonis(nanonis),
            Backend::Mock => {
                let mut mock = MockController::builder()
                    .freq_shift_index(SignalIndex(2))
                    .freq_shift(models::realistic(models::RealisticParams::default()))
                    // Within-batch scatter, well under the default 1.0 Hz
                    // `max_std_dev`, so the stable-read statistics do real
                    // work without stalling in retries.
                    .sample_noise_hz(0.25)
                    .build();
                let registry = build_registry(&mut mock, &[])?;
                // The model answers for index 2; make sure the registry
                // agrees, so a change to the mock's names cannot silently
                // point the tool at a constant channel.
                let resolved = registry.get_by_name("freq shift").map(|s| s.signal_index());
                if resolved != Some(SignalIndex(2)) {
                    return Err(SpmError::Workflow(format!(
                        "mock freq-shift index drifted: registry resolved {resolved:?}, \
                         the model answers for 2"
                    )));
                }
                self.connect_with(Box::new(mock), registry, PresetFiles::default())
            }
        }
    }

    fn connect_nanonis(&mut self, backend: &NanonisBackend) -> Result<(), SpmError> {
        let client = crate::NanonisClient::builder()
            .address(&backend.host)
            .port(backend.port)
            .build()?;
        let mut controller = NanonisController::new(client, NanonisSetupConfig::default());
        log::info!("Connected to Nanonis at {}:{}", backend.host, backend.port);

        // Files first, stream second: a settings file can change the TCP
        // logger's channel list, and Nanonis stops a live stream on that.
        let files = PresetFiles {
            layout: backend.layout_file.clone(),
            settings: backend.settings_file.clone(),
        };
        let (layout, settings) = load_presets(&mut controller, &files, "connect")?;

        let mapping: Vec<(u8, u8)> = backend
            .tcp_channel_mapping
            .iter()
            .map(|m| (m.nanonis_index, m.tcp_channel))
            .collect();
        let registry = build_registry(&mut controller, &mapping)?;

        let stream = StreamSetup::new(
            backend.host.as_str(),
            backend.data_port,
            backend.sample_rate_hz,
        );
        controller.start_streaming(&registry, &stream)?;

        self.install(Box::new(controller), registry, files, layout, settings);
        Ok(())
    }

    /// Connect with a controller and registry built elsewhere, loading
    /// `files` on the way in. For tests, and for controllers the session
    /// does not know how to build.
    pub fn connect_with(
        &mut self,
        mut controller: Box<dyn SpmController>,
        registry: SignalRegistry,
        files: PresetFiles,
    ) -> Result<(), SpmError> {
        self.disconnect();
        let (layout, settings) = load_presets(&mut *controller, &files, "connect")?;
        self.install(controller, registry, files, layout, settings);
        Ok(())
    }

    fn install(
        &mut self,
        mut controller: Box<dyn SpmController>,
        registry: SignalRegistry,
        files: PresetFiles,
        layout: Option<PresetLoad>,
        settings: Option<PresetLoad>,
    ) {
        let facts = ControllerFacts::gather(&mut *controller, Some(&registry));
        let readouts = READOUT_NAMES
            .iter()
            .filter_map(|name| registry.get_by_name(name))
            .map(|s| (s.name.clone(), s.signal_index()))
            .collect();
        self.poisoned = false;
        self.conn = Some(Connection {
            controller,
            registry,
            files,
            facts,
            settings,
            layout,
            readouts,
        });
    }

    /// Stop the stream and drop the controller. A no-op when disconnected.
    pub fn disconnect(&mut self) {
        if let Some(mut conn) = self.conn.take() {
            conn.controller.disconnect();
        }
        self.poisoned = false;
    }

    /// Re-establish a poisoned connection.
    pub fn reconnect(&mut self) -> Result<(), SpmError> {
        let conn = self.conn.as_mut().ok_or_else(not_connected)?;
        conn.controller.reconnect()?;
        self.poisoned = false;
        Ok(())
    }

    /// Load the connect-time layout and settings files again.
    pub fn reload_presets(&mut self) -> Result<(), SpmError> {
        let conn = self.connected_mut()?;
        let (layout, settings) = load_presets(&mut *conn.controller, &conn.files, "reload")?;
        if layout.is_some() {
            conn.layout = layout;
        }
        if settings.is_some() {
            conn.settings = settings;
        }
        conn.facts = ControllerFacts::gather(&mut *conn.controller, Some(&conn.registry));
        Ok(())
    }

    /// What was learned about the controller on connect.
    pub fn facts(&self) -> Option<&ControllerFacts> {
        self.conn.as_ref().map(|c| &c.facts)
    }

    pub fn capabilities(&self) -> HashSet<Capability> {
        self.conn
            .as_ref()
            .map(|c| c.controller.capabilities())
            .unwrap_or_default()
    }

    pub fn registry(&self) -> Option<&SignalRegistry> {
        self.conn.as_ref().map(|c| &c.registry)
    }

    /// The last settings file loaded this session, by whom and when.
    pub fn settings_load(&self) -> Option<&PresetLoad> {
        self.conn.as_ref().and_then(|c| c.settings.as_ref())
    }

    /// The last layout file loaded this session.
    pub fn layout_load(&self) -> Option<&PresetLoad> {
        self.conn.as_ref().and_then(|c| c.layout.as_ref())
    }

    /// Read the idle readouts once. A connection error poisons the session.
    pub fn read_readouts(&mut self) -> Result<Vec<Readout>, SpmError> {
        let conn = self.connected_mut()?;
        let indices: Vec<SignalIndex> = conn.readouts.iter().map(|(_, i)| *i).collect();
        if indices.is_empty() {
            return Ok(Vec::new());
        }
        let values = match conn.controller.read_signals(&indices, false) {
            Ok(values) => values,
            Err(e) => {
                if e.is_connection_error() {
                    self.poisoned = true;
                }
                return Err(e);
            }
        };
        Ok(conn
            .readouts
            .iter()
            .zip(values)
            .map(|((name, index), value)| Readout {
                name: name.clone(),
                index: *index,
                value,
            })
            .collect())
    }

    /// Run one job on the connection.
    ///
    /// Opens the job's log (when the session has a log directory), writes
    /// `run_started` with the job's schema, config and the controller facts,
    /// runs the job with `observers` also attached, and writes `run_finished`
    /// if the job did not. A panic inside the job is caught and reported as
    /// an error; the connection stays usable unless the controller reports
    /// itself poisoned afterwards.
    pub fn run(
        &mut self,
        job: &mut dyn Job,
        shutdown: &ShutdownFlag,
        observers: Vec<Box<dyn Observer>>,
    ) -> Result<Outcome, SpmError> {
        let log_dir = self.log_dir.clone();
        let conn = self.connected_mut()?;
        let started = Instant::now();

        let mut events = EventBus::new();
        let finished = Arc::new(FinishedFlag::default());
        events.add_observer(Box::new(Arc::clone(&finished)));
        let loads = Arc::new(LoadWatcher::default());
        events.add_observer(Box::new(Arc::clone(&loads)));
        if let Some(dir) = log_dir {
            let path = log_path(&dir, job.name())
                .map_err(|e| SpmError::Workflow(format!("cannot open the run log: {e}")))?;
            let file = fs::File::create(&path)
                .map_err(|e| SpmError::Workflow(format!("cannot open the run log: {e}")))?;
            log::info!("Event log: {}", path.display());
            events.add_observer(Box::new(FileLogger::new(file)));
        }
        for observer in observers {
            events.add_observer(observer);
        }

        events.emit(Event::run_started(RunHeader::new(
            job.log_schema(),
            job.header_config(),
            conn.facts.clone(),
        )));

        let name = job.name().to_string();
        // AssertUnwindSafe: after a panic the job is not touched again, and
        // the controller's own harness has already restored the hardware.
        let caught = panic::catch_unwind(AssertUnwindSafe(|| {
            job.run(JobCx {
                controller: &mut *conn.controller,
                registry: &conn.registry,
                events: &events,
                shutdown,
            })
        }));
        let result = match caught {
            Ok(result) => result,
            Err(payload) => {
                let message = panic_message(&*payload);
                log::error!("Job '{name}' panicked: {message}");
                Err(SpmError::Workflow(format!(
                    "job '{name}' panicked: {message}"
                )))
            }
        };

        if !finished.seen() {
            let (outcome, detail) = match &result {
                Ok(Outcome::Completed) => ("completed", None),
                Ok(Outcome::StoppedByUser) => ("stopped_by_user", None),
                Ok(Outcome::CycleLimit(n)) => ("cycle_limit", Some(n.to_string())),
                Ok(Outcome::TimedOut(d)) => ("timed_out", Some(format!("{:.0}s", d.as_secs_f64()))),
                Err(e) => ("error", Some(e.to_string())),
            };
            events.emit(Event::run_finished(outcome, detail, started.elapsed()));
        }

        // A load during the run moved the controller's state for good.
        let (layout, settings) = loads.take();
        if let Some(path) = layout {
            conn.layout = Some(PresetLoad {
                path,
                by: name.clone(),
                at: SystemTime::now(),
            });
        }
        if let Some(path) = settings {
            conn.settings = Some(PresetLoad {
                path,
                by: name.clone(),
                at: SystemTime::now(),
            });
        }
        if !conn.controller.is_connected() {
            log::warn!("The connection is poisoned after job '{name}'; reconnect before the next");
            self.poisoned = true;
        }
        result
    }

    fn connected_mut(&mut self) -> Result<&mut Connection, SpmError> {
        if self.poisoned {
            return Err(SpmError::Workflow(
                "the connection is poisoned; reconnect first".into(),
            ));
        }
        self.conn.as_mut().ok_or_else(not_connected)
    }
}

fn not_connected() -> SpmError {
    SpmError::Workflow("not connected".into())
}

/// The registry every backend gets: the standard TCP map plus `mapping`,
/// names from the controller, aliases.
fn build_registry(
    controller: &mut dyn SpmController,
    mapping: &[(u8, u8)],
) -> Result<SignalRegistry, SpmError> {
    Ok(SignalRegistry::builder()
        .with_standard_map()
        .add_tcp_map(mapping)
        .from_controller(controller)?
        .create_aliases()
        .build())
}

/// Load the layout, then the settings, through the same controller methods
/// a routine uses. Returns what was loaded, stamped with `by`.
fn load_presets(
    controller: &mut dyn SpmController,
    files: &PresetFiles,
    by: &str,
) -> Result<(Option<PresetLoad>, Option<PresetLoad>), SpmError> {
    let stamp = |path: &Path| PresetLoad {
        path: path.to_path_buf(),
        by: by.to_string(),
        at: SystemTime::now(),
    };
    let layout = match &files.layout {
        Some(path) => {
            controller.load_layout(path)?;
            Some(stamp(path))
        }
        None => None,
    };
    let settings = match &files.settings {
        Some(path) => {
            controller.load_settings(path)?;
            Some(stamp(path))
        }
        None => None,
    };
    Ok((layout, settings))
}

/// `<dir>/<job>_<UTC timestamp>.jsonl`, creating `dir`.
fn log_path(dir: &Path, job: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    Ok(dir.join(format!(
        "{job}_{}.jsonl",
        chrono::Utc::now().format("%Y%m%d_%H%M%S")
    )))
}

/// Notices whether a `run_finished` went through the bus, so the session
/// closes a log the job left open without doubling one the job closed.
#[derive(Default)]
struct FinishedFlag(AtomicBool);

impl FinishedFlag {
    fn seen(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

impl Observer for Arc<FinishedFlag> {
    fn on_event(&self, event: &Event) {
        if matches!(event, Event::RunFinished { .. }) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}

/// Picks the preset loads a job reports out of its event stream.
#[derive(Default)]
struct LoadWatcher(Mutex<(Option<PathBuf>, Option<PathBuf>)>);

impl LoadWatcher {
    fn take(&self) -> (Option<PathBuf>, Option<PathBuf>) {
        std::mem::take(&mut *self.0.lock())
    }
}

impl Observer for Arc<LoadWatcher> {
    fn on_event(&self, event: &Event) {
        let Event::Custom { kind, data, .. } = event else {
            return;
        };
        let path = data.get("path").and_then(|p| p.as_str()).map(PathBuf::from);
        let mut loads = self.0.lock();
        if kind == LayoutLoadedEvent::KIND {
            loads.0 = path;
        } else if kind == SettingsLoadedEvent::KIND {
            loads.1 = path;
        }
    }
}

// ============================================================================
// The session thread
// ============================================================================

/// What a GUI asks the session thread to do.
pub enum SessionCmd {
    Connect(Backend),
    Disconnect,
    Reconnect,
    ReloadPresets,
    /// Where the next job's log goes; `None` writes none.
    SetLogDir(Option<PathBuf>),
    /// Run a job. `events` receives every event of the run; `shutdown` is
    /// how to stop it.
    Run {
        job: Box<dyn Job>,
        shutdown: ShutdownFlag,
        events: Sender<Event>,
    },
    /// End the thread, disconnecting first.
    Quit,
}

/// What the session thread reports.
#[derive(Debug, Clone)]
pub enum SessionUpdate {
    State(ConnState),
    /// On connect and after a preset reload.
    Facts(ControllerFacts),
    Capabilities(HashSet<Capability>),
    /// Idle only, about twice a second.
    Readouts(Vec<Readout>),
    /// The last loads, after a connect, a reload or a job that loaded one.
    PresetsLoaded {
        layout: Option<PresetLoad>,
        settings: Option<PresetLoad>,
    },
    JobFinished(Result<Outcome, String>),
    Error(String),
}

/// The GUI's end of a session thread. Dropping it asks the thread to quit.
pub struct SessionHandle {
    commands: Sender<SessionCmd>,
    updates: Receiver<SessionUpdate>,
    thread: Option<JoinHandle<()>>,
}

impl SessionHandle {
    /// Send a command. Fails only if the thread is gone.
    pub fn send(&self, cmd: SessionCmd) -> Result<(), SpmError> {
        self.commands
            .send(cmd)
            .map_err(|_| SpmError::Workflow("the session thread has ended".into()))
    }

    /// Updates since the last call, oldest first. Never blocks.
    pub fn drain(&self) -> Vec<SessionUpdate> {
        self.updates.try_iter().collect()
    }

    /// The update channel, for a caller that wants to block on it.
    pub fn updates(&self) -> &Receiver<SessionUpdate> {
        &self.updates
    }

    /// Ask the thread to quit and wait for it. Blocks until a running job
    /// ends, so request its shutdown first.
    pub fn join(mut self) {
        let _ = self.commands.send(SessionCmd::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(SessionCmd::Quit);
    }
}

/// Put a disconnected session on its own thread.
pub fn spawn(log_dir: Option<PathBuf>) -> SessionHandle {
    spawn_with(Session::new(log_dir))
}

/// Put an existing session, connected or not, on its own thread.
pub fn spawn_with(session: Session) -> SessionHandle {
    let (commands, command_rx) = crossbeam_channel::unbounded();
    let (update_tx, updates) = crossbeam_channel::unbounded();
    let thread = thread::Builder::new()
        .name("session".into())
        .spawn(move || run_loop(session, command_rx, update_tx))
        .expect("spawning the session thread");
    SessionHandle {
        commands,
        updates,
        thread: Some(thread),
    }
}

fn run_loop(mut session: Session, commands: Receiver<SessionCmd>, updates: Sender<SessionUpdate>) {
    let report = |update: SessionUpdate| {
        let _ = updates.send(update);
    };
    report(SessionUpdate::State(session.state()));
    loop {
        match commands.recv_timeout(READOUT_PERIOD) {
            Ok(SessionCmd::Quit) => break,
            Ok(cmd) => handle(&mut session, cmd, &report),
            Err(RecvTimeoutError::Timeout) => {
                if session.state() == ConnState::Connected {
                    match session.read_readouts() {
                        Ok(readouts) => report(SessionUpdate::Readouts(readouts)),
                        Err(e) => {
                            report(SessionUpdate::Error(format!("readout failed: {e}")));
                            report(SessionUpdate::State(session.state()));
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    session.disconnect();
    report(SessionUpdate::State(ConnState::Disconnected));
}

fn handle(session: &mut Session, cmd: SessionCmd, report: &dyn Fn(SessionUpdate)) {
    let describe = |session: &Session| {
        if let Some(facts) = session.facts() {
            report(SessionUpdate::Facts(facts.clone()));
        }
        report(SessionUpdate::Capabilities(session.capabilities()));
        report(SessionUpdate::PresetsLoaded {
            layout: session.layout_load().cloned(),
            settings: session.settings_load().cloned(),
        });
    };
    match cmd {
        SessionCmd::Connect(backend) => {
            report(SessionUpdate::State(ConnState::Connecting));
            match session.connect(&backend) {
                Ok(()) => describe(session),
                Err(e) => report(SessionUpdate::Error(format!("connect failed: {e}"))),
            }
            report(SessionUpdate::State(session.state()));
        }
        SessionCmd::Disconnect => {
            session.disconnect();
            report(SessionUpdate::State(session.state()));
        }
        SessionCmd::Reconnect => {
            if let Err(e) = session.reconnect() {
                report(SessionUpdate::Error(format!("reconnect failed: {e}")));
            }
            report(SessionUpdate::State(session.state()));
        }
        SessionCmd::SetLogDir(dir) => session.set_log_dir(dir),
        SessionCmd::ReloadPresets => {
            match session.reload_presets() {
                Ok(()) => describe(session),
                Err(e) => report(SessionUpdate::Error(format!("reload failed: {e}"))),
            }
            report(SessionUpdate::State(session.state()));
        }
        SessionCmd::Run {
            mut job,
            shutdown,
            events,
        } => {
            report(SessionUpdate::State(ConnState::Running));
            let forwarder: Box<dyn Observer> = Box::new(ChannelForwarder::new(events));
            let result = session.run(&mut *job, &shutdown, vec![forwarder]);
            report(SessionUpdate::JobFinished(
                result.map_err(|e| e.to_string()),
            ));
            report(SessionUpdate::PresetsLoaded {
                layout: session.layout_load().cloned(),
                settings: session.settings_load().cloned(),
            });
            report(SessionUpdate::State(session.state()));
        }
        SessionCmd::Quit => unreachable!("Quit is handled by the loop"),
    }
}
