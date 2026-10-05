# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`rusty-tip psd`, the spectrum of a streamed signal.** Welch-averaged
  over evenly spaced stream samples (16384 by default, segments of 1024
  overlapping by half, Hann window), at the stream's rate as its divisor
  sets it. The reply has the one-sided PSD in the signal's unit squared per
  hertz, the eight highest peaks as amplitude densities with the noise
  floor to judge them against, and the RMS, so an agent can tell a loop
  that rings from a line the room puts there. Only
  signals on the data stream qualify; polled samples have no time base.
  `control::spectrum` holds the FFT and the Welch estimate.
- **`rusty-tip`, a command line for scripts and agents.** Every command
  prints one JSON reply and exits with a code per outcome; `describe` lists
  the commands, the request schema and the exit codes. Commands go to the
  workbench's agent socket by default, to `rusty-tip serve` when it runs
  headless, or connect for one command with `--one-shot`. A server started
  read-only refuses every command that acts; the workbench's socket is
  read-only unless switched. This version reads only: `status`, `read`,
  `controllers` for the Z and PLL loops, `scan` for frame, buffer and speed,
  and `frame`, which writes one signal's frame to a file and replies with
  per-line RMS and trace-retrace statistics for tuning the loop.
  `SpmController::scan_frame_get` and `Session::query` back them.
  `control` holds the protocol, the TCP server and `Limits`, and
  `SessionCmd::Call` and `SessionHandle::remote` let a server share a session
  thread with the window. See `docs/agent-cli.md`.
- **The coarse motor is declared, not assumed.** `[nanonis]` has
  `motor_group` (1 to 6, as the Motor module numbers them) and
  `motor_z_approach` (`"plus"` or `"minus"`: which of the group's Z
  directions moves the tip toward the sample). Every Z step count in the
  library is signed against that, positive approaches and negative
  retracts, where it used to hard-code group 1 and `Z+`. The defaults are
  those two, so no existing file changes behaviour. `NanonisSetupConfig`
  carries it as `CoarseMotor`, `NanonisBackend` as `motor`, and the
  workbench edits it on the Connection page next to the TCP channel
  mapping.
- **A log level on the workbench's Connection page.** Error to trace,
  applied at once and kept between starts. It is the window's setting,
  not the connection's, so a config file's `[console]` neither carries
  it nor overrides it.
- **Controller presets.** A `Preset` is one controller's parameters under
  a name with a `tuned_at` block, the setpoint, bias and amplitude the
  gains were tuned at; a `PresetFile` is `[[presets]]` tables, and
  `PresetStore` with its `TomlPresetStore` is how they are listed, saved
  and removed, so a database can stand behind the same calls later.
  `ApplyPreset` writes one, keeping the setpoint the loop holds and
  refusing a Z-controller name the module has not defined, and reads it
  back as `controller/applied` and `controller/read`. The operating point
  is recorded and never written since the gains' dependence on it goes by
  loop law: a log current loop's do not depend on the setpoint or the
  bias, a linear or a frequency loop's do (`Preset::depends_on_operating_point`).
  The config has `[controllers].presets_file` (`./controllers.toml`) and
  `[tip_prep].z_controller_preset`, which tip prep writes before its first
  approach with `initial_z_setpoint_a`, warning when a preset whose gains
  depend on the operating point was tuned elsewhere than the run's bias
  and setpoint, and failing before the tip moves on a name the file does
  not have. The workbench's Connection page has the file's path next to
  the log directory; the Controllers page lists the presets for the
  selected controller, loads one into the form, applies it, deletes it,
  and saves the form as one with the live setpoint, bias and amplitude
  recorded. `configs/presets/controllers.toml` ships the lab's log current
  loop for tip prep and its frequency loop for imaging. `TipPrep`'s log
  schema declares the two controller events.
- **`rusty-tip-gui`**, the workbench: one window, one controller
  connection, several tools. The connection pane connects once (backend,
  host, ports, layout and settings files, TCP channel mapping) and shows
  what the session reports: state, stream rate, the signal table,
  capabilities, four live readouts while idle, and which settings file was
  loaded last, by whom and when. Tools sit in a sidebar with Run, Setup
  and History tabs; a job's error is an error state, not a clean exit. Tip
  prep is the first tool, with the old Control tab as its run panel. The
  run view is a fold over log records shared by live runs and replayed
  logs, so History opens any `.jsonl` in the log directory into the same
  view. `tip-prep-gui` keeps its screens and stays until the workbench
  reaches parity in the lab; the one change to it is that its files load
  before the stream starts, as the CLI's now do. Built with
  `--features gui`, which now enables eframe's `persistence` so the pane
  and each tool's setup survive a restart.
- **Schema-driven setup forms.** `AppConfig` and every config type under
  it derive `JsonSchema`, with unit annotations (`x-unit`,
  `x-display-unit`) so a current stored in amperes is edited in
  picoamperes. The workbench draws the tip-prep Setup page from that
  schema: checkboxes, drag values with units, combo boxes for enums and
  the pulse method, a checkbox plus field for optional limits, and the
  settings that decide a run at the top. A shipped config survives the
  form unchanged, which a test checks for every file in `configs/`. The
  file's connection tables (`[nanonis]`, `[data_acquisition]`,
  `[experiment_logging]`, the TCP mapping) are not on the form: the
  Connection page owns them in the workbench, loading a file offers them
  to the page when disconnected, and Save writes the page's settings back
  into the file, so one file serves the CLI and the workbench. A field
  annotated `x-enabled-by` is drawn only while the sibling boolean it
  names is on, which is how the stability settings follow
  `check_stability`. Units have one home in the workbench: a bare `A` or
  `m` field is edited in `pA` or `nm` without an annotation, hovering a
  scaled field shows the stored SI value, and readouts, the run panel and
  the events tail take the prefix that fits the value (`120.0 pA`,
  `-500.0 mV`).
- **Drift as a tool and a routine.** `rusty_tip::drift::DriftRoutine`
  wraps the drift actions (status, measure, compensate, off) as a
  `Routine` with `LeaveInPlace` and `RunSetup::NONE`, writing typed
  `drift/status`, `drift/measured` and `drift/compensated` events next to
  the actions' `drift/burst`. It refuses to measure when the data stream
  does not carry Z unless polling is allowed. The workbench has it as the
  Drift tool, and `const-distance drift` is now a thin wrapper over it, so
  the two front ends share one implementation.
- **Feedback controllers as parameter sets.** `rusty_tip::controllers`
  gives the Z-controller and the PLL's amplitude and phase loops one
  struct each of the fields the protocol lets you set, in SI with unit
  annotations, and `SpmController` gains `controllers`, `read_controller`,
  `write_controller` and `set_controller_enabled` behind
  `Capability::Controllers`. A read also reports whether the loop is on,
  its status word and, for the Z-controller, the loops Nanonis has
  defined; which one runs is the `active` parameter. A
  `ControllerProfile` is a TOML file naming a settings file to load first
  and the parameters to write; `ApplyProfile` loads, writes each
  controller, reads it back and logs `controller/applied` with before and
  after, then `controller/read` for everything. Switching a loop on or off
  is a separate job, never a side effect of applying. The mock holds the
  same loops. The workbench's Controllers tool is the bare Setup tab:
  read, edit one controller at a time in a form drawn from its schema,
  see what differs from the last reading, apply, revert, and load or
  save profiles; the Run tab lists what an apply changed.
- **A loop behind the mock, and the stream live in the workbench.**
  `rusty_tip::loop_model::ZLoop` is a Z feedback loop as arithmetic: a PI
  controller on a log or linear input, a tunnelling current and a
  frequency shift that fall off with the gap, and an actuator lag. The
  mock runs one behind its Z-controller (`MockControllerBuilder::
  loop_model`), so Z and the current answer to the setpoint and gains,
  to withdraw and to approach, and stream at 1 kHz; the `Mock` backend
  has it on. `SpmController::stream_since` hands out the stream in
  pieces, the session thread taps it ten times a second while idle
  (`SessionUpdate::Samples`), and the workbench keeps the last twelve
  seconds per signal. Under the Z-controller's form, two strip charts
  show the loop's input against the setpoint the form holds, and Z.

- **One connection, many runs.** `run_routine` borrows the controller
  instead of consuming it, so a second routine can start on the same
  connection with the data stream still up. `rusty_tip::session::Session`
  builds on that: connect once (layout and settings files, signal
  registry, stream), run `Job`s one after another, each with its own run
  log, disconnect at the end; `session::spawn` puts one on a thread behind
  command and update channels for a GUI, polling live readouts while idle.
  `SpmController` gains `disconnect` for the session-level stop, next to
  the per-run `prepare`/`teardown`.
- **`Routine::exit_policy`** replaces `exit_retract_steps`. `Withdraw {
  retract_steps }` is what every routine did before; `LeaveInPlace` leaves
  Z and the coarse motor alone, for routines that reconfigure the
  controller or measure drift between passes with the tip engaged.
- **`Routine::run_setup`** says what the harness sets before the run: Z
  home mode and position, and the safe-tip threshold, optionally with
  safe-tip switched off for the run and restored after. These were fields
  of `NanonisSetupConfig` and applied inside `prepare`; now the routine
  owns them, they read the same on every controller, and each step is an
  action in the run log. The default is the relative 50 nm home every run
  used to get, with safe-tip untouched; tip prep adds safe-tip off, as both
  front ends did; `RunSetup::NONE` touches nothing.
- **`rt.presets()`** with `load_settings` and `load_layout`, behind the new
  `Capability::Presets`, so a routine that depends on particular Nanonis
  settings loads them as part of its run. Each load is logged as an action
  and as a typed `routine/settings_loaded` or `routine/layout_loaded`
  event. The mock records loads in `MockObservations::settings_loaded` and
  `layouts_loaded`.

- **`rt-log`**, a terminal reader for experiment logs: `ls` a directory of
  runs, `summary` a run (outcome, time per top-level action, measurement
  spread, event counts), `timeline` the action tree with durations and
  params, `plot` any numeric series against time, `export` flat CSV
  tables plus a `run.json`. The series and columns come from the schema
  in the log's header, so it needs no code per tool. The reader behind it
  is `experiment_log::reader`, for anything else that wants a log back as
  data; it tolerates logs from before the header and lines a crash cut
  short. `tip-prep-mock --log <path>` writes a log from a dry run to try
  it on.
- **Changing a running tip prep's config.** `TipPrep::with_reload` takes a
  `ConfigReload` mailbox; a config sent into it is taken between two pulse
  cycles, never inside a stability check, and writes nothing to the
  controller. The run keeps what it was set up with (initial bias and
  setpoint, Z preset, safe-tip threshold, the connection tables) and says
  so in a `tip_prep/config_reloaded` event, which also records the config
  as applied. `Cycles::set_limits` lets a lowered `max_cycles` or
  `max_duration_secs` end the loop before another pulse. The workbench's
  tip-prep Run tab has an Apply to run button beside Start and Stop that
  sends the Setup form, and Setup greys out the fields a run keeps.

### Changed

- **Tip-prep pulses are bars from 0 V.** The pulse-voltage plot draws every
  pulse as a bar from a 0 V line instead of a point on a line, so sign and
  size read at a glance. A failed stability check's pulse is no longer a
  series of its own: it is a bar like the rest, between the cycles it came
  after. The frequency-shift plot is unchanged.
- **The workbench in base16 colours.** The desktop's base16 scheme is one
  palette, `widgets::SCHEME`, and dark mode takes it whole: backgrounds,
  text, selection, links, warnings and errors, and the plots' blue, amber
  and olive in place of light blue, bright orange and neon green. Light mode
  keeps egui's theme, since a dark scheme's shades do not flip into a light
  one, and its plots take the same hues darker. Buttons keep their own
  colours.
- **The drift panel puts its plot first.** The burst plot sits above the
  status, results and burst table, and is taller.
- **The workbench reads at arm's length.** Bigger buttons and rows
  throughout; Start, Connect and Reconnect green, Stop, Disconnect and
  Delete red, and the buttons that write to the controller amber, each
  plain while disabled. Headings with their explanations on hover replace
  the paragraphs, and results are labelled values: the tip-prep status, the
  live readouts, the drift result, and drift's before and after as a
  table. The tip-prep plots fill the tab, share one x scale and axis width
  so their cycles line up, draw whole-cycle grid lines, and keep their key
  beside the title rather than over the first cycles. The Connection page
  is one aligned form that no longer moves when the backend changes, and
  Connect lives on the top bar only. Controllers groups its buttons under
  Profile, All loops, Presets and Parameters, shows a loop as ON or OFF
  beside its switch, and has no Run tab: its last action shows under
  Setup. Setup's file reload is Re-read file, the preset one Refresh
  presets.
- **History is per tool.** The History tab lists the current tool's logs,
  newest first, with readable outcomes and durations; All tools shows the
  rest.
- **Drift compensation waits out creep first.** `compensate` takes baseline
  bursts until three in a row agree, then corrects; a drift still changing
  after `settle_max_ms` (180 s, `--settle`) fails with nothing touched. Only
  the trial's sign is used, corrections average but never apply less than
  half a reading, and a burst reads the stream without gaps through the new
  `SpmController::read_signal_samples_after`.
- **The tip-prep plots** are one height under plain headings, explained on
  hover and in a legend, with a line every volt on the pulse plot. The
  Connection tab lost its status dot, which the top bar already shows.
- **No fixed settle on top of the landing gate.** `post_approach_settle_ms`
  and `post_reposition_settle_ms` default to 0, since the gate already
  waits for the loop; they stay as an extra wait for a frequency shift
  that needs longer. A config that sets them keeps its values.
- **The lab presets ship as `controllers.toml` at the repo root**, where
  `[controllers].presets_file` looks by default, instead of under
  `configs/presets/`.
- **Tip prep's Z preset is picked from the preset file** in the workbench's
  key settings, not typed; the form hides a field annotated `x-hidden`.
- **The freq-shift plot shows the readings between cycles again**, hollow:
  the initial read and a stability check's confirmations and final read.
- **`TipPrepSignals::resolve`** finds the frequency-shift and current
  signals by name, for every caller of `TipPrep::new`.
- **A landing is judged on the loop, and safe-tip is armed only while the
  tip is parked.** The lab logs of 2026-09-28 and 29 ended in safe-tip
  trips on phases where nothing should trip. After a landing the current
  was not on the setpoint but a spike train to the preamp rail, dying out
  over a few hundred milliseconds: the loop's own step response to the
  Auto Approach ramp, and after coarse steps the stage's creep on top,
  growing with the number of steps. The calibrated approach armed
  safe-tip 200 ms after the first landing and kept it armed through the
  second, so either landing could trip it. Now the first landing is
  followed by `settle_on_setpoint`, a wait on the Z loop's input rather
  than on a clock: batches of `stable_signal_samples` of the current until
  the mean is within `landing_tolerance` of `initial_z_setpoint_a` and the
  batch holds still, bounded by `landing_timeout_ms`, after which the
  landing is taken as done with a warning. Safe-tip is armed after that,
  only for the home, settle and centre steps, and disarmed before the
  second landing, which is made by switching the Z loop on from 50 nm
  off (`z_controller_set`) and waiting on the same gate: the loop walks in
  on its integrator and lands softly where the Auto Approach ramp does
  not. Every batch is in the log as a `landing` measurement. The fixed
  settle between the coarse steps and the approach (`post_move_settle_ms`,
  500 ms since 0.2.3) is gone, since the gate covers the creep it was
  for, and a reposition takes one coarse step in x and y instead of three:
  one puts the last pulse's debris behind the tip, every further step is
  more creep to wait out. z retract stays at three, since steps and
  unlevel samples need the height. `TipPrepParams` and `TipPrep::new`
  take the current signal's index; `CalibratedApproach`, `Reposition` and
  `RepositionSpec` carry an optional `LandingGate`, without which the
  sequence falls back to fixed waits and a second auto-approach;
  `ZCtrl::calibrated_approach_within` takes the gate. The mock's current
  channel now follows its Z setpoint while the loop is closed
  (`MockControllerBuilder::current_index`, index 0 by default).
- **The tip-prep plots run over cycles and can be navigated.** The
  frequency-shift and pulse plots used the run clock for x and had every
  interaction switched off, and their y strip was sized to its widest tick
  label, which on a short run started too thin to draw any. Both now take
  the cycle number as x from the `tip_prep/cycle` events, one point per
  cycle, with max pulses drawn as diamonds half a cycle after the cycle
  they followed; the y strip has a fixed minimum width and tick labels in
  the unit; and drag, wheel zoom, box zoom and double-click reset are on,
  with the two plots sharing one x range and one cursor. The hover label
  names the cycle and the value in its unit.
- **`NanonisSetupConfig` is down to `tcp_refresh_output`.** Layout and
  settings files are loaded by whoever owns the connection through
  `SpmController::load_layout`/`load_settings`, before the stream starts
  (a settings file can change the TCP logger's channel list, and Nanonis
  stops a live stream on that; before, they loaded after it). Z home and
  safe-tip come from the routine's `run_setup`. `tip-prep` and
  `tip-prep-gui` behave as before; `tip_prep::nanonis_setup` became
  `tip_prep::load_presets`.
- **The experiment log is self-describing** (`docs/experiment-log.md`).
  Every run starts with a `run_started` line carrying the tool, version,
  git commit, the config as loaded, the resolved signals and stream rate,
  and the JSON Schema of every custom event the tool can write; it ends
  with `run_finished` and the outcome. Every line has a `seq`. Actions log
  their own fields as `params` (a pulse has its voltage now) and a `depth`,
  so the steps inside a calibrated approach or a reposition appear as
  children and the tree can be rebuilt. Custom events are typed structs
  with a `tool/name` kind, declared once per tool and pinned by a schema
  snapshot test; tip prep's `tip_prep_state` became `tip_prep/cycle`,
  `tip_prep/phase` and `tip_prep/max_pulse`, and the harness's events are
  `routine/cleanup_failed` and `routine/panicked`. const-distance writes a
  log too (`--log-dir`, default `./experiments`); its actions had been
  running with no observer attached. `schemars` is a new dependency.
- **Fewer crates in the build.** A CLI build resolved 208 crates and the
  GUI build 367; they are now 89 and 269. `config` no longer pulls its
  default JSON5, RON, YAML and INI readers, only TOML is used. `image`,
  which only `cuox-finder` needs, is optional behind the new `cuox`
  feature, so `cargo build --release` no longer compiles a hundred crates
  of codecs for tools that never open an image; build `cuox-finder` with
  `--features cuox`. `byteorder` was unused and is gone.

### Removed

- `frame`'s `trace_retrace_rms_mirrored`. It was there until a frame on
  hardware settled which way Nanonis sends backward rows; it sends them the
  same way round as forward ones, so the as-sent comparison is the one.
- `data_acquisition.oversampling`. `sample_rate` is the one value now: the
  controller reads the RT frequency, derives the logger divisor that comes
  closest, measures what the stream delivers, and corrects the divisor
  once if its guess at the logger base was wrong. A config that still sets
  `oversampling` loads, the key is ignored. The default `sample_rate` is
  1000 Hz, the rate the old default divisor produced on an RC5.
- `tip_prep.stability.max_duration_secs`. It was never enforced, and a sweep's
  length is already fixed by `bias_steps × step_period_ms`; the run-level
  `tip_prep.max_duration_secs` keeps counting through the check. Configs
  that still set it load fine, the key is ignored.

### Fixed

- **`psd`'s RMS disagreed with its spectrum.** It was taken over the whole
  series about one mean, while each segment of the spectrum has its own
  mean removed, so drift slower than a segment counted in one and not the
  other. `rms` is the spectrum's integral now, and `rms_total` the whole
  series', larger by the drift.
- **`busy` blamed a job for whatever held the session.** A long read from
  another client holds the session thread as a job does; the message now
  says which of the two it was.
- **Z limits read high first.** `ZControllerParams::limits_m` followed
  Nanonis, high then low, where everything else in rusty-tip, and the form
  that shows it as "a to b", writes a range low then high. It is low then
  high now; the controller turns it round for Nanonis. A preset saved high
  first still loads as meant, since the smaller of the two is taken as the
  low limit.
- **`rusty-tip` answered `busy` to a read it was running.** The 10 s busy
  timeout covered the whole request, so a sampled read longer than that,
  three signals at 5000 samples on a 1 kHz stream, came back "busy, nothing
  was done" while it held the session. The timeout now covers only the wait
  for the session to start a request; one that started is waited for up to
  60 s, and whether it started or was dropped is settled atomically, so
  `busy` always means nothing ran. A read takes at most 20000 samples over
  its signals.
- **`rusty-tip --addr localhost:…` missed a server on `127.0.0.1`.** Windows
  resolves `localhost` to `::1` first and the client tried only that; it now
  tries each address the name resolves to.
- **`rusty-tip serve` connected before it knew it could listen.** With the
  port taken it connected and loaded the config's layout and settings, then
  gave up. It binds first now, and `Server::serve` takes the bound listener.
- **The scan buffer named the wrong signals on Nanonis.** `Scan.BufferGet`,
  `Scan.BufferSet` and `Scan.FrameDataGrab` number channels by Signals
  Manager slot (0 to 23), not by signal, and they were passed through as
  signal indexes. A scan of Current, Z, the PLL's phase and amplitude, the
  frequency shift and the excitation read as Current and five unused
  inputs, `rusty-tip frame "Z (m)"` was refused, and multi-pass asked the
  buffer for slot 30, which does not exist. `NanonisController` now
  translates through `Signals.InSlotsGet`, asked on each use since slots can
  be reassigned, and `SpmController::scan_frame_data_grab` takes a
  `SignalIndex`. The query is sent by hand: `nanonis-rs` 0.5's
  `signals_in_slots_get` parses its reply without the slot names that come
  first.
- **Tool names in the workbench sidebar sat indented.** Every tool's label
  kept room for the running marker; only the running tool carries one now,
  and the rest line up with Connection.
- **One tool's run showed on another's tabs.** The last run's status,
  elapsed time and "Run finished" message appeared on every tool. A run
  now shows only on its own tool, and a message goes with the page it came
  up on.
- **A loop's "on" marker drew as an empty box.** The UI font has no
  circle glyph; the Controllers tabs say "(on)" instead.
- **No pulse voltage during cycle 1.** The tip-prep status read the voltage
  off the cycle event, which only follows the reposition and the read. It
  now shows the `bias_pulse` action's voltage as the pulse fires, and the
  sharp band is the run's own, not the Setup form's.
- **The phase stuck after a stability check.** Phases were only emitted on
  the way into a check, so one that did not end the run left the GUI on its
  last step. `PhaseEvent::Pulsing` marks every return to the pulse cycles.
- **A mapped TCP channel took two signals.** `tcp_channel_mapping`
  added its pairs over the standard map without removing the index that
  already held the channel, so the lab's `76 -> 19` left `77 -> 19` in
  place: the excitation read the frequency shift under its own name, in
  stable reads and in the stream dump. A mapped channel now leaves the
  index that held it; one that should stream too needs a mapping of its
  own.
- **A dead data stream went unnoticed between runs.** The TCP reader stops
  for good once the logger goes quiet, and a run that made no stream read
  went through and dumped a buffer hours old. `NanonisController::prepare`
  now refuses the run with the stream's error, and the end-of-run dump
  holds only the run's own frames.
- **A preset tuned at another amplitude went on without a word.** The
  operating-point warning compares the amplitude loop's setpoint too.
- **The first pulse ignored the tip.** The pulse method chose each voltage
  from the reading at the end of the previous cycle, so the first pulse of
  a run fired at the method's floor whatever the initial reading said, and
  so did the first pulse after a stability reset. The initial reading now
  feeds the method before the first pulse, and a stability check, which
  repositions, hands on the reading at the site it left the tip on, read
  after any reset, so every pulse follows a reading taken where it fires. With the linear method a blunt initial
  reading now gets the voltage its shift maps to instead of the minimum.
- **The drift gate trusted the configured sample rate.** The stable read
  converts a per-sample slope into Hz/s using `data_acquisition.sample_rate`,
  while the delivered rate is the controller's base rate over
  `oversampling`. The shipped configs said 2000 Hz for a stream that
  delivers 1000, so the 0.5 Hz/s gate acted as 0.25. The routine now uses
  the rate the controller measured when the stream started
  (`SpmController::stream_rate_hz`); the config value is a fallback, with a
  warning when it is more than ten percent off.
- **The stream rate read low.** It was counted over a window that opened
  before the logger sent its first frame, so a 500 Hz stream measured
  458. The measurement now spans first to last frame received, and the
  rate recorded in the log header and used by the drift gate is the
  nominal base over divisor once the measurement has confirmed it, with
  the measured value kept in the startup line as evidence.
- **The stable-read backoff ignored a stop request.** The 100, 200 and
  400 ms waits between retries were plain sleeps; they go through the
  interruptible settle now.
- **A stop during an approach did nothing until the approach finished.**
  The approach was a blocking poll inside the controller, out of reach of
  the shutdown flag, so Ctrl+C or the GUI's stop button during the initial
  approach (a budget of ten minutes) was honoured only afterwards. The
  approach actions now start the approach and poll it through the
  interruptible settle; a stop lands within 100 ms and switches the
  auto-approach off before the cleanup withdraws, so the controller is not
  still stepping toward the surface while the tip is being pulled away. A
  budget overrun is handled the same way. `SpmController` gains
  `auto_approach_running` and `auto_approach_stop` for this.
- **A run ended with a withdraw and nothing else.** 0.2.3 backed the coarse
  motor off ten steps after the final withdraw; the v2 harness only
  withdrew, leaving the tip parked at the top of the piezo range and still
  within reach of the surface. The harness now asks the routine how far to
  retract on exit: tip prep answers with the new
  `tip_prep.timing.exit_retract_steps` (default 2), other routines keep
  their spot with zero. Noticed on the LT system during the September
  campaign.
- **The GUI showed every pulse as positive.** The `tip_prep_state`
  snapshot carried the pulse method's voltage magnitude rather than the
  signed voltage that was fired, so a polarity switch was visible in the
  console log and on the instrument but never in the GUI's status panel or
  pulse history. The snapshot now reports the pulse as fired, and the
  max-voltage pulse after a failed stability check emits one too, so it
  appears in the history instead of vanishing.
- **The Z-home mode defaulted to absolute.** `NanonisSetupConfig::default()`
  set `ZHomeMode::Absolute`, and both `tip-prep` and `tip-prep-gui` took the
  default. The calibrated approach homes the tip to back 50 nm off the
  surface before centring the frequency shift; in absolute mode that same
  call drives Z to the coordinate +50 nm instead, which is toward the
  surface whenever the surface sits above it. 0.2.3 used relative mode.
  The default is relative again, both binaries say so explicitly, and a
  test pins it. Neither 0.3 nor 0.4 met a tip, so this never fired.
- **The max-voltage pulse after a failed stability check fired withdrawn.**
  A v2 revision added a withdraw before it, so the pulse reshaped nothing
  and the next cycle inherited the same unstable apex. It now fires with the
  tip engaged, as 0.2.3 did, and the routine test checks the call order.
- **A tripped safe-tip no longer gets approached into.** 0.2.3 checked the
  Z-controller status after every step of the calibrated approach and
  aborted the run if safe-tip had fired; the v2 sequence had dropped the
  check, so the final approach would have re-approached whatever tripped
  it. The check is back, with a test for the abort and one for an
  unreadable status not counting as a trip.
- `tip-prep-gui` did not build: the `oversampling` field added to
  `DataAcquisitionConfig` was missing from the GUI's config conversion.
- A `status_interval` of zero is rejected at config load instead of
  panicking on the first cycle.

### Added

- `tip_prep.timing.approach_timeout_ms` (default 600 s) for the approaches
  that start from a full withdraw, and `reposition_approach_timeout_ms`
  (default 300 s) for the short one inside a reposition. 0.2.3 gave the
  long approaches ten minutes; v2 had capped everything at five.
- `const-distance drift status|measure|compensate|off`: the Z drift
  measurement and compensation from the action layer, runnable between
  scans from the command line. It streams Z through the TCP logger,
  finding Z's channel in the controller's signal slots and checking the
  stream against a plain read of Z before trusting it.
- `examples/folme_probe.rs`: measures what a FolMe constant-height trace
  depends on (round-trip latency, feedback-off timing and TipLift, Z step
  response with the loop open, whether FolMe's wait flag blocks). Every
  open-loop move retracts first, and the retract direction is measured
  from TipLift rather than assumed.
- **Multi-pass** (`multi_pass` module): read and write Nanonis `.mpas`
  configuration files, byte-exactly, and load one on a controller with
  `multi_pass::apply`. The format is undocumented by the vendor and was
  worked out by diffing GUI-saved files; the module doc is the write-up.
  A `[PassN]` section is one scan *direction*, so two passes need four
  sections, and `MultiPassConfig::constant_lift` builds that. `MPass.Load`
  resolves its path on the controller, thus `apply` takes both a local path
  and a host path rather than assuming a shared filesystem.
- **Drift compensation** (`MeasureZDrift`, `CompensateDrift`): measures how
  fast Z is drifting and leaves the controller cancelling it. A measurement
  is a burst, every sample the data stream delivers for the window, fitted
  through block means so the rate comes with a standard error that slow
  noise does not flatter. Compensation is a loop of a fixed number of
  bursts: measure, correct, measure what is left. The k-th correction
  takes a k-th of its reading, which leaves the velocity at the mean of
  every estimate so far, and every burst corrects whether or not its
  reading clears the error bar, since correcting only the ones that do
  overshoots. Each burst is a `drift/burst` log event. The sign convention for the compensation velocity is
  undocumented, so it is learned from one deliberately large trial step
  unless the caller says it is known, and a channel that does not respond
  is refused with the previous velocity put back. Needs the feedback loop
  closed, thus it is a between-passes operation.
- **Scan buffer** (`SpmController::scan_buffer_get`/`_set`/`_ensure`): which
  signals a scan records, and at what resolution. `_ensure` adds channels
  without dropping the ones already there.
- **End-of-line waiting** (`SpmController::scan_wait_end_of_line`):
  line-level progress, reporting line number, direction and multi-pass pass
  number separately.
- `const-distance baseline`: configures the two-pass constant-lift scan that
  native multi-pass runs, as the published baseline (Moreno et al., Nano
  Lett. 2015) the rolling-ellipsoid trajectory has to beat at step edges.
- **Routine harness** (`routine` module): automations are structs
  implementing `Routine`, run against an `Rt` that hands out
  capability-checked subsystem handles (`rt.bias()?.set(v)?`), an
  interruptible `settle()`, a `cycles()` driver that turns cycle/time
  budgets into `Outcome`s, and `guarded()` for cleanup that runs however
  the body ends. `run_routine` owns the controller life cycle (prepare,
  withdraw on exit, teardown) around any routine, including when the
  routine panics: it catches the unwind, restores the hardware, and
  re-raises. Handle operations emit the same started/completed/failed
  events `execute_logged` used to, so JSONL logs keep their shape.
- Two new events so failures during cleanup stay visible in the JSONL
  log rather than only in `log`: `cleanup_failed` when `guarded`
  swallows a cleanup error to preserve the body's, and
  `routine_panicked` when a routine unwinds.
- `scan().props_set()` and `scan().speed_set()` now emit
  started/completed/failed events like every other state change, so a
  scan speed altered mid-run is visible in the JSONL log. The scan
  reads (`status`, `props_get`, `speed_get`) stay silent.
- Tip preparation is now `TipPrep`, the reference `Routine`
  implementation; `run_tip_prep` keeps its exact signature and behaviour
  as a thin wrapper over `run_routine`.

### Changed

- `nanonis-rs` 0.4 to 0.5, for `Scan.WaitEndOfLine`.

- **Breaking (library):** `ShutdownFlag` is backed by a condition variable
  so `request()` wakes sleeping waiters immediately (new `wait_timeout`);
  `from_arc()` and `arc()` are gone since writes to a raw
  `Arc<AtomicBool>` could never notify a waiter. Handlers call
  `request()` on a clone instead.
- **Breaking (library):** `Outcome` moved to the `routine` module
  (re-exported from `tip_prep` unchanged) and now derives
  `Debug`/`Clone`/`Copy`/`PartialEq`/`Eq`.
- **Breaking (library):** `tip_prep::runner::execute_logged` and
  `interruptible_sleep` are gone; their jobs moved into the harness
  (`Rt`'s event-logged execution and `ShutdownFlag::wait_timeout`).
- The stability check now restores the scan speed even when a sweep
  errors out (previously only on completion or shutdown), and waits
  inside the routine wake immediately on a stop request instead of at
  the next poll tick.

### Removed

- **Breaking (library):** the `workflow` module. Its declarative
  `Step`/`Condition` executor is replaced by the routine harness, which
  puts control flow in Rust where the compiler can see it.
- **Breaking (library):** the `machine_state` module and the `Action`
  methods that fed it (`kind`, `expects`, `effects`, `resolves`,
  `apply_to_state`). The state model existed so the executor could
  decide at runtime whether a step was legal; a `Routine` cannot express
  an illegal call in the first place, since the subsystem handle is the
  only way to reach the operation.
- **Breaking (library):** `ActionRegistry`, `ActionFactory`,
  `ActionInfo` and `action::builtin_registry()`. Constructing actions by
  string name existed only to deserialize workflow steps.
- **Breaking (library):** `Action::execute_and_store` and
  `execute_and_store_as`, which had no callers.

## [0.4.0] - 2026-08-10

An API-cleanup release on the experimental v2 line. The public surface now
matches what the production tip-prep path actually supports, and the safety
metadata the action layer always advertised is enforced on every execution
path. Like 0.3.0 this is mock-validated only; nothing here has met a tip yet.

Config files from 0.3.0 parse unchanged: the float widening accepts the same
TOML, and the one new timing field defaults to the previously hard-coded value.

### Changed

- **Breaking (library):** `run_tip_prep` returns `Result<Outcome, SpmError>`
  instead of a boxed error, takes its context as one `TipPrepParams` struct
  instead of four positional arguments, and maps a shutdown request to
  `Outcome::StoppedByUser` itself, thus callers no longer downcast to tell
  Ctrl+C apart from a real failure.
- **Breaking (library):** every signal-read command takes a `SignalIndex`
  newtype instead of a bare `u32`, so a signal index can no longer be confused
  with a TCP channel or a frame position. `serde(transparent)` keeps action
  JSON and event logs byte-identical. `SignalRegistry::from_controller` builds
  the name lookup straight from the controller that will serve the reads.
- **Breaking (library):** `ShutdownFlag` moved from `workflow::` to a
  top-level `shutdown` module (re-exported at the crate root). The `workflow`
  module is marked experimental pending the routine-harness redesign.
- **Breaking (library):** config structs store `f64` end to end; the `as f64`
  casts at every consumer are gone. The only remaining cast sits at the
  nanonis-rs wire boundary, where `ScanConfig` genuinely carries `f32`.
- `Action::requires()` is now enforced on every execution path: an action
  whose capability the controller lacks fails with `SpmError::Unsupported`
  before any command reaches hardware. Previously only the unused workflow
  executor checked.
- TCP stream setup lives in `NanonisController::start_streaming`; the CLI and
  GUI no longer carry hand-rolled copies of the channel-map plumbing.
- The settle between motor move and approach during a reposition is
  configurable as `tip_prep.timing.post_move_settle_ms` (default 500 ms, the
  previously hard-coded value).

### Removed

- **Breaking (library):** dead v1 API: `Error`/`RunOutcome`, `Logger`,
  `ControllerAction`, `ControllerState`, `TipStateConfig`,
  `TipControllerConfig`, the unused `BufferedTCPReader` query methods and
  `poll_with_timeout`. `buffered_tcp_reader` and `utils` are private now.

### Fixed

- The GUI attached the TCP stream reader before stopping and restarting the
  logger, so a run could consume stale frames left over from a previous
  session. Both frontends now share the CLI's corrected ordering via
  `start_streaming`.

## [0.3.0] - 2026-08-04

**This release is an experimental rewrite. Treat it as a preview of where the
project is going, not as the version to run an instrument from.**

The v2 stack (`SpmController` trait, action/event system, `tip_prep::runner`)
replaces the v1 `ActionDriver`/`TipController` code.
It has not been validated on a real microscope yet, since every test here runs
against `MockController`.
The stability gates in particular were re-derived into the new read path and
carry the thresholds calibrated for v1, thus they may well need retuning once
this meets a tip.

Expect `SpmController`, the action system and the config format to keep moving
through 0.3.x, and expect the rough edges to come off as the routine gets
machine time.
If you need the last version that has run on hardware, use
[0.2.3](https://github.com/kronberger-droid/rusty-tip/releases/tag/v0.2.3).

Only the stability-gate changes are listed below; see the commit history for the
rewrite itself.

### Changed

- **Breaking (CLI):** the v2 binary is now called `tip-prep`, not `tip-prep-v2`.
  It replaces the v1 binary of the same name, which is gone.
- **Breaking (config):** the signal-read gates moved out of `[data_acquisition]`
  and into `[tip_prep.signal_stability]`, which is now the only place they live.
  `max_std_dev`, `max_slope` and `stable_read_retries` under `[data_acquisition]`
  are gone; use `max_std_dev_hz`, `max_slope_hz_per_s` and `read_retry_count`.
  An old config still parses, but its gates are silently ignored, so it runs on
  the defaults. `stable_signal_samples` stays where it is.
- `signal_stability.data_collection_duration_ms` and `read_timeout_secs` are
  accepted but unused: the v2 read path sizes its batch from
  `stable_signal_samples` and bounds a read by `read_retry_count`.

### Fixed

- `nix build .#tip-prep` built nothing: the flake still passed `--bin tip-prep`
  while the crate only defined `tip-prep-v2`. The package versions are now read
  from `Cargo.toml` rather than hardcoded, so they cannot drift again.
- Re-applied the 0.2.2 drift fix to the v2 read path. `ReadStableSignal` had
  inherited the per-sample regression slope, so its drift tolerance still scaled
  with the batch size. It now converts to Hz/s using the stream's sample rate,
  and its defaults are the 0.2.3 values (1.5 Hz, 0.5 Hz/s) rather than the
  pre-fix 1.0 Hz / 0.01 Hz-per-sample pair.

## [0.2.3] - 2026-05-27

### Added

- `[tip_prep.signal_stability]` config section exposing the signal-read stability
  gates at runtime: `max_std_dev_hz`, `max_slope_hz_per_s`,
  `data_collection_duration_ms`, `read_timeout_secs`, `read_retry_count`. These
  were previously compile-time only. Honored by both the CLI and the GUI (the GUI
  carries them through from the loaded config file).

### Changed

- Loosened the default noise gate `max_std_dev` from 0.3 → 1.5 Hz, which was too
  tight for typical tips (they fluctuate but hold a stable mean). Tune per setup
  via the new config section.

## [0.2.2] - 2026-05-27

### Fixed

- Frequency-shift drift is now measured in Hz/s instead of Hz-per-sample, so the
  stability check no longer varies with how many TCP frames were buffered
  (oversampling). A genuinely stable tip is now judged consistently.
- When a stable signal can't be confirmed, the reading falls back to the *mean* of
  the raw buffer instead of its *minimum*, removing a systematic negative bias in
  the reported frequency shift.
- Reading scan properties at the start of a stability sweep no longer fails with an
  `UnexpectedEof` IO error on older Nanonis firmware (via nanonis-rs 0.4.0's
  version-tolerant `Scan.PropsGet`).

### Changed

- Tightened the signal-stability gates to a realistic tip scale: noise threshold
  `max_std_dev` 1.0 → 0.3 Hz and drift threshold `max_slope` 2.0 → 0.5 Hz/s
  (≈0.25 Hz over the 500 ms collection window).
- Updated the nanonis-rs backend to 0.4.0.

## [0.2.1] - 2026-05-21

### Added

- `CITATION.cff` and Zenodo DOI badges for software citation metadata.
- crates.io publishing workflow that runs on version tags.

### Fixed

- Malformed author email in `Cargo.toml`.

## [0.2.0] - 2026-03-09

- First tagged release.
