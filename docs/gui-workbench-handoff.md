# GUI workbench: build handoff

Branch: `feat/gui-workbench`, cut from `lab-campaign-prep` on 2026-09-24.

This document is the full brief for turning the tip-prep GUI into a general
workbench that hosts several automation tools against one persistent
controller connection. It records what was decided, what the code looks like
today, and what to build in which order. Line numbers are as of that cut;
verify against the code before trusting one.

## Goal

One desktop app (`rusty-tip-gui`) where the operator connects to the
controller once, sees what it is and what it streams, then picks a tool,
fills in its parameters, runs it, watches it live and can open any past run
from its log. New automation written as a `Routine` should show up in the app
with little or no GUI code.

Audience for tool authors: physicists used to Python. Adding a tool must be
doable by an intermediate Rust programmer without an agent.

## Decisions (Martin, 2026-09-24)

1. **Persistent connection.** Connect once, run many tools against the same
   connection. Runs no longer reconnect.
2. **Connection pane.** A pane with a connect/disconnect toggle and controller
   information: state, stream rate, capabilities, signal table.
3. **Live readouts while idle.** The pane shows current values (bias, Z,
   current, freq shift) between runs, so the data stream stays up for the
   whole session, not per run.
4. **Layout and settings files load once, on connect.** A tool that depends on
   particular Nanonis module settings does its own layout/settings load as
   part of its run. Loading a settings file is sometimes the only way to set
   things over TCP, so this is a first-class operation, not a workaround.

Standing decisions from earlier work that still bind:

- Sync and multithreaded only, no async. A few threads with simple channel
  contracts.
- State machines stay plain Rust `enum` + `match`. No FSM framework.
- Subsystem traits must stay usable without the harness (`Rt`); the harness
  adds shutdown, events and budgets on top.

## Where things stand

### The current GUI

`bin/tip-prep-gui/app.rs` (2052 lines) is tip-prep only:

- `EditableConfig` (`:188`) mirrors `AppConfig` field by field, with hand
  written `from_app_config` (`:301`) and `to_app_config` (`:474`). Every
  config change has to be made twice.
- `drain_events` (`:856`) matches event kinds by string (`"tip_prep/cycle"`,
  `"tip_prep/phase"`, `"stable_read"`) and feeds two hardcoded plots.
- `run_controller` (`:1806`), `build_nanonis_backend` (`:1877`),
  `build_mock_backend` (`:1916`), `build_signal_registry` (`:1948`) and
  `setup_tcp_stream` (`:1968`) duplicate what `bin/tip-prep/main.rs` does.
- Every run connects from scratch in a spawned thread and drops the
  controller at the end.
- Known wart: `RunStatus::Error` exists but a clean `Ok(())` from
  `run_controller` hides routine errors because `run_controller` logs them and
  returns `Ok(())` (`:1860` onward).
- **Divergence:** the CLI sets `disable_safe_tip: true`
  (`bin/tip-prep/main.rs`, the `NanonisSetupConfig` literal), the GUI leaves
  it at the default `false`. The new tip-prep tool must pick one on purpose;
  match the CLI unless Martin says otherwise.

**Leave `tip-prep-gui` untouched** until the new app reaches parity. It is in
use for the lab campaign.

### Library pieces the workbench builds on

| Piece | Where | Notes |
|---|---|---|
| `SpmController` trait | `src/spm_controller.rs:111` | `prepare`/`teardown`/`is_connected`/`reconnect`, `capabilities()`, `stream_rate_hz()` |
| `NanonisController` | `src/nanonis_controller.rs` | `NanonisSetupConfig` (`:29`), `start_streaming` (`:271`), `prepare` (`:593`), `teardown` (`:651`) |
| `MockController` | `src/mock_controller.rs` | builder + `MockObservations` for assertions |
| Routine harness | `src/routine/` | `Routine` trait, `Rt`, `run_routine` (`mod.rs:142`), subsystem handles in `subsystems.rs` |
| Events | `src/event/mod.rs` | `Event` enum, `EventBus`, observers `FileLogger`, `ChannelForwarder`, `EventAccumulator`, `ConsoleLogger` |
| Experiment log | `src/experiment_log/` | `RunHeader`, `ToolSchema`, `LogEvent: JsonSchema`, `ControllerFacts`; `reader::Log` parses a `.jsonl` back |
| Tip prep | `src/tip_prep/runner.rs` | `TipPrep<'a>` routine borrows `&'a AppConfig`; `run_tip_prep` wraps `run_routine` |
| Log viewer (terminal) | `bin/rt-log/main.rs` | reads logs with `experiment_log::reader` |
| const-distance | `bin/const-distance/main.rs` | `plan` (offline), `baseline` and `drift` (connected); none of them is a `Routine` yet |

`AppConfig` and its sub-structs do **not** derive `JsonSchema` yet. Only the
log event types do.

### Lifecycle problems a persistent session exposes

`NanonisController::prepare`/`teardown` combine two lifetimes:

| Step | Today in | Belongs to |
|---|---|---|
| load layout file, settings file | `prepare` | session (connect) |
| start data stream + TCP reader | the bins, via `start_streaming` | session (connect) |
| z-home mode and position | `prepare` | run |
| safe-tip snapshot, override, optional disable | `prepare` | run |
| stop data stream, stop TCP reader | `teardown` | session (disconnect) |
| restore safe-tip | `teardown` | run |

`teardown` is also one-shot (`torn_down` flag), and `run_routine` takes
`Box<dyn SpmController>` by value and drops it. Its own doc comment names the
limit ("one routine per controller").

Second problem: `run_routine` always withdraws the tip at the end. Right for
tip prep, wrong for a job that only reconfigures the controller or measures
drift with the loop closed. `bin/const-distance/main.rs:785` already bypasses
`Rt` for this reason. `Routine::exit_retract_steps` is the only exit knob.

## Target architecture

```
┌ Connection pane ───────────────────────────────────────────────────────────┐
│ Backend [Nanonis ▾]  host 127.0.0.1  port 6501  data 6590   [Connect]      │
│ ● connected · stream 2000 Hz · settings: tip-prep.ini (on connect, 14:02)  │
│ Bias 0.200 V   Z 12.3 nm   I 50.1 pA   Δf -3.2 Hz        [signals ▸] [caps ▸]│
├──────────┬─────────────────────────────────────────────────────────────────┤
│ Tools    │  [Setup] [Run] [History]                                        │
│ Tip prep │                                                                 │
│ Drift    │  Setup:   schema-driven params form, load/save TOML             │
│ Baseline │  Run:     start/stop, outcome, tool panel + generic panels      │
│ Plan     │  History: logs in the log dir; opening one shows the Run view   │
└──────────┴─────────────────────────────────────────────────────────────────┘
```

### Threads

```
GUI thread ──SessionCmd──▶ session thread (sole owner of the controller)
           ◀─SessionUpdate─   idle: polls readouts ~2 Hz, publishes snapshots
           ◀─Event──────────  running: runs the job, events via ChannelForwarder
```

Exactly one thread touches the controller. No `Mutex<dyn SpmController>`, no
hardware calls from the GUI thread. Stop is the existing `ShutdownFlag`,
shared with the GUI when a job starts.

Sketch (adjust names freely, keep the shape):

```rust
pub enum Backend {
    Nanonis { host: String, port: u16, data_port: u16, sample_rate_hz: f64,
              layout_file: Option<PathBuf>, settings_file: Option<PathBuf>,
              tcp_channel_mapping: Vec<TcpChannelMapping> },
    Mock,
}

pub enum SessionCmd {
    Connect(Backend),
    Disconnect,
    Reconnect,
    Run { job: Box<dyn Job>, shutdown: ShutdownFlag, events: Sender<Event> },
    ReloadSettings,
}

pub enum SessionUpdate {
    State(ConnState),             // Disconnected | Connecting | Connected | Poisoned | Running
    Facts(ControllerFacts),       // on connect and after a settings load
    Capabilities(HashSet<Capability>),
    Readouts(Vec<(String, f64)>), // idle only
    SettingsLoaded { path: PathBuf, by: String, at: SystemTime },
    JobFinished(Result<Outcome, String>),
    Error(String),
}
```

The session (controller, registry, stream, the command loop) is GUI-agnostic
and goes in the library as `src/session.rs`, so the CLIs can use it too and it
is testable against the mock without egui. Everything egui goes in
`bin/rusty-tip-gui/`.

### Jobs and tools

Two layers, so the session never knows about egui and tools never know about
threads:

```rust
// library: what the session runs
pub trait Job: Send {
    fn name(&self) -> &str;
    fn log_schema(&self) -> ToolSchema;
    fn header_config(&self) -> serde_json::Value;
    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError>;
}

pub struct JobCx<'a> {
    pub controller: &'a mut dyn SpmController,
    pub registry: &'a SignalRegistry,
    pub events: &'a EventBus,
    pub shutdown: &'a ShutdownFlag,
}
```

A routine-backed job owns its params and builds the routine inside `run`, so
`TipPrep<'a>`'s borrow of `AppConfig` needs no change:

```rust
struct TipPrepJob { config: AppConfig }
impl Job for TipPrepJob {
    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        let fs = cx.registry.get_by_name("freq shift") /* ... */;
        let mut routine = TipPrep::new(&self.config, fs);
        run_routine(cx.controller, cx.events, cx.shutdown, &mut routine)
    }
}
```

The session wraps every job the same way: open the JSONL `FileLogger`, emit
`run_started` with `RunHeader::new(job.log_schema(), job.header_config(),
facts)`, run, emit `run_finished`. That replaces `open_run_log` /
`finish_run_log` in const-distance and the logging block in
`run_controller`.

GUI side, object safe because params travel as JSON:

```rust
// bin/rusty-tip-gui/tools/mod.rs
pub trait Tool {
    fn id(&self) -> &'static str;
    fn label(&self) -> &str;
    fn needs_connection(&self) -> bool { true }
    fn schema(&self) -> schemars::Schema;
    fn defaults(&self) -> serde_json::Value;
    fn job(&self, params: serde_json::Value) -> Result<Box<dyn Job>, String>;
    fn panel(&mut self, _ui: &mut egui::Ui, _view: &RunView) {}
}
```

Plus a generic helper so a typical tool is a few lines:

```rust
pub fn routine_tool<P>(id: &'static str, label: &str,
                       make: fn(P) -> Box<dyn Job>) -> impl Tool
where P: Serialize + DeserializeOwned + JsonSchema + Default;
```

`plan` from const-distance needs no controller (`needs_connection = false`).
Run it on a plain worker thread with its own `EventBus`, not through the
session, so it works while disconnected and while another job holds the
session.

### Exit policy

Replace `Routine::exit_retract_steps` with an exit policy the harness obeys:

```rust
pub enum ExitPolicy {
    /// Withdraw, then back the coarse motor off this many steps. Tip prep.
    Withdraw { retract_steps: u16 },
    /// Leave Z and the motor alone. Config-only jobs, drift compensation.
    LeaveInPlace,
}
fn exit_policy(&self) -> ExitPolicy { ExitPolicy::Withdraw { retract_steps: 0 } }
```

Default keeps today's behaviour for routines that don't override it. With
that, `baseline` and `drift` can become routines instead of bypassing `Rt`.

### Tool-owned settings loads

New subsystem handle on `Rt`, with a matching capability:

```rust
rt.presets()?.load_settings(&path)?;
rt.presets()?.load_layout(&path)?;
```

- New `Capability::Presets`. `NanonisController` implements it with
  `util_settings_load` / `util_layout_load` (code currently inside `prepare`,
  `src/nanonis_controller.rs:593`). Canonicalize the path as `prepare` does.
- The handle emits a typed `settings_loaded` / `layout_loaded` event
  (`LogEvent`) carrying the path, so the run log records it and the session
  can update "settings last loaded" from the event stream.
- `MockController` supports it as a recorded no-op
  (`MockObservations::settings_loaded: Vec<String>`), so simulated runs of a
  tool with a settings file still work.
- The connect-time load calls the same controller methods. One code path.
- A load persists for the rest of the session and cannot be undone. The pane
  shows the last load (file, which tool or "on connect", time) so the next
  operator knows the state has moved.
- Tool params carry the path as `Option<PathBuf>` so it appears in the form
  and the run header.

### Schema-driven forms

`bin/rusty-tip-gui/form.rs`: render an editable form for a
`serde_json::Value` against a `schemars::Schema`. Cover what the configs
actually use: objects (collapsible sections), numbers, integers, booleans,
strings, paths, `Option<T>` (checkbox + field), string enums (combo box),
internally tagged enums (`PulseMethod`: variant combo + that variant's
fields), fixed arrays of two numbers (`sharp_tip_bounds`). Anything else
falls back to a raw JSON text field, never a panic.

Units: store SI, display human units. Annotate fields with a schemars
extension and have the form convert:

```rust
#[schemars(extend("x-unit" = "m", "x-display-unit" = "nm"))]
pub lift: f64,
```

Doc comments become hover text (schemars puts them in `description`).
Validation stays in Rust: on Start, deserialize into `P`, call the params'
`validate()` where one exists (`AppConfig::validate`), show the error next to
the Start button.

Load/save TOML per tool keeps working through `toml` + serde as today. Also
persist the last-used params per tool (eframe storage) so the form survives a
restart.

### Run view

`bin/rusty-tip-gui/run_view.rs`: a pure fold `RunView::apply(&mut self,
record)` over events, with no egui in it, so it is unit testable. It holds:

- header (tool, config, controller facts)
- current action and depth (from `ActionStarted`/`Completed`/`Failed`)
- numeric series keyed by label or kind+field: every `DataCollected` value
  and every scalar field of a declared `Custom` kind
  (`reader::KindDecl::scalar_fields` already extracts these from the header
  schema). Cap each series (the old GUI uses 20k points).
- phase transitions (any `Custom` kind whose data has a `phase` string)
- outcome and duration from `RunFinished`

Generic panels draw from it: series plot with a series picker, action
timeline, log tail. A tool's `panel` adds its own view on top (tip prep: the
freq-shift plot with the sharp-tip band, pulse voltage history, tip shape
readout, which are the old Control tab's contents).

Feed it from two sources:

- live: the `ChannelForwarder` receiver, drained each frame
- replay: `experiment_log::reader::Log::read(path)` records

Live `Event`s and log `Record`s are different types today. Either convert an
`Event` to a `Record` via `serde_json` (they share the wire format, since
`FileLogger` writes the `Event` and `reader` parses the same line), or fold
over `Record` only and serialize live events through that path. Prefer the
single `Record` fold. One code path means replay can't drift from live.

History tab: list `*.jsonl` in the configured log dir (tool, start time,
outcome from `Log::finished`), open one into a read-only Run view.

## Build order

Each step is a mergeable PR with CI green (`fmt`, `clippy -D warnings` with
`--features gui`, `test`, `test --doc`, `typos`). Those four job names are
required checks in the main ruleset; don't rename them.

### Step 1: library lifecycle

**Done 2026-09-24** on this branch. What landed, where it differs from the
sketch below, and what the next step inherits:

- Names: `SpmController::disconnect` is the session-level stop; there is
  no `connect_setup`. Connect-time work is done by the owner through
  `load_layout`/`load_settings` (the `Presets` capability) and
  `NanonisController::start_streaming`, so `NanonisSetupConfig` is down to
  `tcp_refresh_output`.
- Z home and safe-tip are `Routine::run_setup() -> RunSetup`, applied and
  restored by `run_routine` through the trait, so the harness owns the
  policy and `NanonisController::prepare`/`teardown` are gone (the trait
  defaults are no-ops; the mock still records both). `run_routine` logs
  each step as an action (`set_z_home`, `safe_tip_configure`,
  `safe_tip_set_enabled`, `safe_tip_restore`). The default `RunSetup` is
  the relative 50 nm home every run used to get, safe-tip untouched
  (Martin, 2026-09-24); `RunSetup::NONE` touches nothing. The CLI and the
  old GUI share `tip_prep::load_presets` for the connect-time file loads.
- `src/session.rs`: `Session` (sync, testable), `Job`/`JobCx`,
  `Backend::{Nanonis, Mock}`, `PresetFiles`, `PresetLoad` (path, by, at),
  `spawn`/`spawn_with` → `SessionHandle`, `SessionCmd::{Connect,
  Disconnect, Reconnect, ReloadPresets, Run, Quit}`,
  `SessionUpdate::{State, Facts, Capabilities, Readouts, PresetsLoaded,
  JobFinished, Error}`. The session writes `run_finished` only when the
  job did not (an observer watches for it), so routine jobs get exactly
  one. Idle readouts poll `read_signals` for bias, Z, current and freq
  shift by registry name.
- Tests: `tests/session.rs` covers the list below; harness tests for
  `RunSetup`/`ExitPolicy` are in `src/routine/mod.rs`. The tip-prep schema
  snapshot gained the two preset kinds.
- Not done: the CLIs still build their own controller rather than using
  `Session`; `const-distance` still has `open_run_log`/`finish_run_log`
  (step 5).

- Split the controller lifecycle: add `connect_setup` / `disconnect` (names
  open) for files, stream and TCP reader; keep `prepare` / `teardown` for the
  run-level parts. `teardown` must be callable once per run; move the
  `torn_down` guard so it protects only the stream shutdown in `disconnect`.
- Move z-home and safe-tip settings out of `NanonisSetupConfig` into per-run
  params the routine or job supplies. Keep a compatibility path so
  `tip-prep`, `tip-prep-gui` and `const-distance` behave exactly as now.
- `run_routine(&mut dyn SpmController, ...)`. `run_tip_prep` keeps its
  signature (it can own the box and pass `&mut *box`).
- `ExitPolicy` replacing `exit_retract_steps`; tip prep returns
  `Withdraw { retract_steps: config.tip_prep.timing.exit_retract_steps }`
  (default 2 since the lab-campaign-prep retract fix).
- `Capability::Presets` and `rt.presets()`, Nanonis implementation, mock
  no-op with observations, typed load events.
- `src/session.rs`: `Session` owning controller + registry + stream,
  `Job`, `JobCx`, the command loop, readouts polling, per-job log file.
- Tests (mock): two jobs run back to back on one session without a
  reconnect; `teardown` restores safe-tip after each run; `LeaveInPlace`
  issues no withdraw (`MockObservations::withdraw_count`); a settings load is
  recorded and emitted; a job error or panic leaves the session usable;
  disconnect stops the stream once. All existing tests in
  `tests/tip_prep_routine.rs` pass unchanged.

### Step 2: new app, connection pane, tip prep only

- `bin/rusty-tip-gui/` with `required-features = ["gui"]` in `Cargo.toml`.
  Modules: `main.rs`, `app.rs`, `connection.rs` (pane), `tools/mod.rs`,
  `tools/tip_prep.rs`, `run_view.rs`.
- Connection pane per the decisions: backend (Nanonis/Mock), host, ports,
  sample rate, layout/settings files, TCP channel mapping, Connect/Disconnect
  (disabled while running), Reconnect when poisoned, state, stream rate,
  capabilities, signal table (from `ControllerFacts`), live readouts,
  "settings last loaded", Reload settings.
- Tip prep tool with its old Setup form reused as a stopgap if step 3 isn't
  ready; its Run view reproduces the old Control tab.
- Surface job errors as an error state (fixes the swallowed-error wart for
  the new app).
- Keep the theme selector (light prints better; see the old app's hover
  text).
- Acceptance: against the mock, connect once, run tip prep twice, stop one
  mid-run, disconnect; the second run does not reconnect.

### Step 3: schema forms

- Derive `JsonSchema` on `AppConfig` and every nested config type (in
  `src/config.rs`), with unit annotations.
- `form.rs` as specified; tip prep switches to it; `EditableConfig` is not
  copied into the new app.
- Test: round trip `AppConfig::default()` → JSON → form edit (no-op) → JSON →
  `AppConfig` is lossless; every file in `configs/` loads into the form and
  saves back to an equivalent config.

### Step 4: generic run view and history

- `RunView` fold over `Record`, generic panels, History tab.
- Test: fold a recorded tip-prep log from the mock and assert series, phase
  and outcome; the same run viewed live and replayed produces the same
  `RunView`.

### Step 5: const-distance tools

- Port `drift` (status, measure, compensate, off) and `baseline` onto
  `Routine` with `ExitPolicy::LeaveInPlace`; keep the CLI subcommands as thin
  wrappers over the same routines so both front ends share one
  implementation.
- `start_z_stream` (`bin/const-distance/main.rs:476`) configures its own
  stream. With a session stream already up, the drift job must instead check
  that Z is in the session's stream and fail clearly if not, never restart
  the stream underneath the session.
- `plan` as an offline tool (`needs_connection = false`).
- Tools: Drift, Baseline, Plan in the sidebar.

### After step 5

- Retire `tip-prep-gui` once Martin confirms parity in the lab.
- Candidates that fit this shape next: CuOx tip prep, the Python classifier
  interface (`Classifier` trait, HTTP + npy sidecar), the double-pass routine.

## Conventions

- Commits via the `commit-writer` skill, author
  `kronberger-droid <kronberger@proton.me>`, conventional prefixes as in
  `git log`. No `Co-Authored-By` unless asked.
- PR bodies follow `.github/pull_request_template.md`; the `github-voice`
  skill covers the prose.
- Nothing gets pushed, and no PR is opened, without asking Martin first.
- Docs: verify every default and name against the code. Stale docs have bitten
  this repo before.
- `typos` runs on docs too; `settings/*.ini` is excluded on purpose.
- Update `CHANGELOG.md` for user-visible changes.

## Open questions

- Method names for the connection-level hooks (`connect_setup` /
  `disconnect` are placeholders).
- Whether the session also owns the `SignalRegistry`'s TCP channel mapping
  UI, or it stays a per-backend config block.
- How a tool declares which live readouts matter to it (fixed four in the
  pane for now).
- Whether idle readouts should come from the TCP stream buffer or from
  `read_signals` polling. The stream is cheaper if the reader exposes a
  latest-frame read.
