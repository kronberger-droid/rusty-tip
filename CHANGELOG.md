# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Typed `Frame`** (`frame` module): a grabbed scan image as flat
  row-major `f32` pixels plus channel, dimensions, scan direction and
  physical geometry from the new `SpmController::scan_frame_get`.
  `rt.scan()?.grab_frame(channel, forward)` returns one, and its
  metadata (never the pixels) lands in the event log. `ToNpyPayload`
  encodes any frame-like payload as npy bytes for transport.
- **`Classifier` trait** (`classifier` module) with `HttpClassifier`
  (sync HTTP to a Python sidecar: `GET /info` handshake at connect,
  `POST /classify` with npy body + JSON metadata header) and
  `MockClassifier` (scripted verdicts for tests). `rt.classify` runs
  one and logs model, version, latency and the serialized verdict as
  events. A reference FastAPI server ships in
  `python/classifier_server.py`.
- `SpmError::ClassifierUnavailable`, matchable so routines can fall
  back to threshold logic when the sidecar is unreachable; contract
  violations (bad status, malformed JSON) stay `Protocol`.

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
- An action's `action_started` event now carries its parameters instead
  of an empty object, so the log records that a pulse was 4.0 V for
  50 ms rather than only that a pulse happened.
- Safe-tip guard for calibrated approaches, restoring the abort-on-trip
  checks v1 performed. Off unless a routine asks: `Routine::safe_tip_guard`
  sets it for a whole run and `Rt::set_safe_tip_guard` toggles it around a
  section. Scoped to the approach on purpose, since that is where the tip
  is driven at the surface; checking everywhere would mostly misfire.
  `TipPrep` turns it on, matching v1.
- Tip preparation is now `TipPrep`, the reference `Routine`
  implementation; `run_tip_prep` keeps its exact signature and behaviour
  as a thin wrapper over `run_routine`.

### Changed

- **Breaking (library):** `GrabScanFrame` returns a `Frame` instead of
  writing JSON to the `DataStore`, and `Analyzer::analyze` takes a
  `&Frame` instead of the deleted `AnalyzerInput`. `RunAnalyzer` is gone:
  analyzers are plain function calls on grabbed frames.
- **Breaking (library):** the action outputs that were still
  `serde_json::Value` are typed: `ReadScanStatus` and `ReadSafeTipStatus`
  return `bool`, `ReadSignalNames` returns `Vec<String>`,
  `ReadDataStreamStatus` returns `TCPLogStatus`, `OsciRead` returns
  `OsciData`, `ReadZControllerStatus` returns `ZControllerStatus` (the
  last two got their `Serialize` derives upstream in nanonis-rs 0.5.0,
  which is now the minimum version).
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

### Fixed

- **Regression against 0.2.3, the last version run on hardware, present
  in the published 0.3.0 and 0.4.0:** the Nanonis z-home mode defaulted
  to `Absolute`, where 0.2.3 set `Relative`. Neither affected release had
  met a tip, so no patch was cut for them; run 0.5.0 or later on
  hardware, or 0.2.3.
  The calibrated approach homes mid-sequence to back off the surface
  before centring the frequency shift, which only retracts under
  `Relative`; `Absolute` sends Z to a fixed coordinate instead.
- **Regression against 0.2.3:** the max-voltage pulse fired after a
  failed stability check withdrew the tip first. That pulse exists to
  reshape the tip hard enough to start over, which needs the tip
  engaged, as it was in v1.
- **Regression against 0.2.3:** the first approach of a run shared the
  300 s action timeout. It starts from an unknown coarse position and
  gets its own `initial_approach_timeout_ms`, defaulting to the 600 s
  v1 used. Approaches during a run keep the shorter timeout.

### Changed (continued)

- **Breaking (library):** `SpmError::Workflow` is now `SpmError::Routine`,
  since the module it was named for no longer exists. The `Display` output
  is unchanged.
- `NanonisSetupConfig::from_app_config` replaces the setup struct each
  binary built by hand. Naming only the fields that come from config is how
  the z-home mode silently differed from what the approach assumes.

### Removed

- **Breaking (library):** the `workflow` module. Its declarative
  `Step`/`Condition` executor is replaced by the routine harness, which
  puts control flow in Rust where the compiler can see it.
- **Breaking (library):** `DataStore` and `ActionContext.store`. The
  scan-frame handoff was the store's last use; data now flows between
  steps as typed values through the routine that runs them.
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
- **Breaking (library):** `ActionOutput`. `Action` now has an associated
  `Output` type, so an action that reads a voltage returns `f64` and the
  handle methods no longer carry a `Protocol` error for an output shape
  that could not occur. `Event::action_completed` takes the serialized
  `serde_json::Value` directly; the JSONL output field is unchanged.
- **Breaking (library):** the dead half of `types`: `TipShape`,
  `SignalStats`, `StableOsciData`, `SessionMetadata`, `DataToGet`,
  `AutoApproachResult` and `AutoApproachStatus`, none of which had a
  reference left. `TipShape` also collided by name with the live
  `action::tip_shaper::TipShape`.
- **Breaking (library):** `Deserialize` on the action structs, along
  with their `#[serde(default)]` attributes. Deserializing an action by
  name was the executor's job. `Serialize` stays, since it is what puts
  an action's parameters into the event log.

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
