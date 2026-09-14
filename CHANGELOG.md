# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

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

- **The drift gate trusted the configured sample rate.** The stable read
  converts a per-sample slope into Hz/s using `data_acquisition.sample_rate`,
  while the delivered rate is the controller's base rate over
  `oversampling`. The shipped configs said 2000 Hz for a stream that
  delivers 1000, so the 0.5 Hz/s gate acted as 0.25. The routine now uses
  the rate the controller measured when the stream started
  (`SpmController::stream_rate_hz`); the config value is a fallback, with a
  warning when it is more than ten percent off.
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
  `tip_prep.timing.exit_retract_steps` (default 10), other routines keep
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
  scans from the command line.
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
- **Drift compensation** (`SpmController::compensate_drift`): measures how
  fast Z is drifting and leaves the controller cancelling it. The sign
  convention for the compensation velocity is undocumented, so this solves
  for it from a trial velocity rather than guessing, and reports a
  compensation channel that does not respond instead of writing a number
  derived from noise. Needs the feedback loop closed, thus it is a
  between-passes operation.
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
