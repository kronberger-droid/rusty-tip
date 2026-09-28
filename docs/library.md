# Library guide

The library is the product: routines like tip-prep are applications built on
it. This guide covers the pieces you compose your own automation from —
running the shipped routine, writing a routine of your own against the
harness, the action system underneath, implementing `SpmController` for
other hardware — current as of the 0.5.0 development line.

## Running the tip-prep routine

```rust
use rusty_tip::config::load_config;
use rusty_tip::event::EventBus;
use rusty_tip::tip_prep::{TipPrepParams, run_tip_prep};
use rusty_tip::{ShutdownFlag, SignalIndex, SignalRegistry};
use std::path::Path;

let config = load_config(Path::new("config.toml"))?;
let events = EventBus::new();
let shutdown = ShutdownFlag::new(); // wire to Ctrl+C / a stop button

// Resolve the frequency-shift signal through the registry, so the index is
// backed by a name the controller actually reported.
let registry = SignalRegistry::from_controller(&mut *controller)?;
let freq_shift = registry
    .get_by_name("freq shift")
    .expect("controller must expose a frequency-shift signal")
    .signal_index();

let outcome = run_tip_prep(
    controller, // Box<dyn SpmController>
    TipPrepParams {
        events: &events,
        shutdown: &shutdown,
        config: &config,
        freq_shift,
    },
)?;
```

`run_tip_prep` owns the run (prepare/teardown, Z home and safe-tip, withdraw
and coarse retract on exit) and returns an `Outcome` — `Completed`,
`StoppedByUser`, `CycleLimit`, or `TimedOut`. A shutdown request is an
expected ending, never an error. It consumes the controller, and dropping
it is what stops the data stream; to run tip prep as one of several
routines on one connection, build a `TipPrep` and call `run_routine` with a
borrowed controller, or use a `Session` (below).

For a complete, runnable example against the mock controller, see
`examples/tip-prep-mock.rs` (`cargo run --example tip-prep-mock`).

## Writing a routine

A routine is a struct implementing `Routine`. Its `run` method receives an
`Rt` (the routine runtime), which hands out the controller's subsystems and
absorbs the scaffolding every routine otherwise reimplements: capability
checks, event logging, interruptible waits, cycle/time budgets, and cleanup
that must run no matter how a step ends.

```rust
use rusty_tip::routine::{Outcome, Routine, Rt};
use rusty_tip::spm_error::SpmError;

struct PulseUntilSharp {
    target_hz: f64,
}

impl Routine for PulseUntilSharp {
    fn name(&self) -> &str {
        "pulse_until_sharp"
    }

    fn run(&mut self, rt: &mut Rt) -> Result<Outcome, SpmError> {
        rt.bias()?.set(-0.5)?;
        rt.z()?.calibrated_approach()?;

        let mut cycles = rt.cycles(Some(100), None);
        while let Some(cycle) = cycles.next() {
            rt.bias()?.pulse(4.0, 50)?;
            rt.settle(1000)?;
            let fs = rt.signals()?.read(rusty_tip::SignalIndex(76))?;
            log::info!("cycle {cycle}: freq shift {fs:.2} Hz");
            if fs >= self.target_hz {
                return Ok(Outcome::Completed);
            }
        }
        Ok(cycles.outcome())
    }
}
```

Run it with `run_routine(&mut controller, &events, &shutdown, &mut routine)`,
which owns the run: `prepare()` before, the routine's `RunSetup` applied,
the tip left as its `ExitPolicy` says and `teardown()` after, whatever the
outcome. A stop request (Ctrl+C, GUI button) surfaces as
`Outcome::StoppedByUser`, never as an error. "Whatever the outcome" covers
panics too: a panicking routine is cleaned up first, then the panic is
re-raised unchanged.

The controller is borrowed, so one connection runs routine after routine:
prepare a tip with one, measure with the next, without reconnecting or
restarting the data stream. Ending the connection is the owner's job
(`SpmController::disconnect`, or dropping the controller).

Two optional methods on `Routine` say what the harness should do around
`run`:

- **`run_setup()`** returns a `RunSetup`: the Z home mode and position to
  set, and the safe-tip threshold to apply, optionally with safe-tip
  switched off for the run. Whatever safe-tip was before is put back on
  exit, however the run ends, and every step is in the event log. The
  default sets a relative 50 nm home, since every calibrated approach homes
  the tip and absolute mode would drive Z to a coordinate instead of
  backing off, and leaves safe-tip as the operator set it. `RunSetup::NONE`
  touches nothing, for a routine that must not move Z home either. Tip
  prep switches safe-tip off on top, since a pulse is a current spike by
  design.
- **`exit_policy()`** returns an `ExitPolicy`: `Withdraw { retract_steps }`
  (the default, with zero steps) withdraws and backs the coarse motor off;
  `LeaveInPlace` leaves Z and the motor alone, for a routine that only
  reconfigures the controller or measures drift with the loop closed.

The pieces, in the order you meet them:

- **Subsystem handles** — `rt.bias()?`, `rt.z()?`, `rt.signals()?`,
  `rt.motor()?`, `rt.scan()?`, `rt.drift()?`, `rt.multi_pass()?`. Each accessor checks the controller's
  capabilities (a controller without a motor makes `rt.motor()` fail with
  `Unsupported` at the call site), and events are emitted for you.
  The rule for what gets logged: every operation that *changes* the
  instrument's state emits started/completed/failed, and so does every
  read whose value is scientific data (`signals().read()`). Reads used
  for control flow stay silent — `scan().status()` is polled in a loop,
  and logging that buries the run. Fetch a handle per statement rather
  than storing it; that keeps borrows from ever overlapping.
- **`rt.settle(ms)`** — an interruptible wait: a stop request wakes it
  immediately and surfaces as `ShutdownRequested`. Use it instead of
  `thread::sleep`, always.
- **`rt.cycles(max_cycles, max_duration)`** — drives the main loop and
  turns exhausted budgets and stop requests into the right `Outcome`, so
  the loop body contains only the science.
- **`rt.guarded(body, cleanup)`** — runs `cleanup` however `body` ends.
  For hardware that must be restored (a running scan, a modified scan
  speed, an engaged tip) even when the work in between fails. The body's
  error wins; a cleanup error on top of it is logged and emitted as a
  `cleanup_failed` event, so it never disappears silently. A cleanup
  that should never fail the run handles its own errors and returns
  `Ok(())`, which is what the stability sweep does.
- **`rt.presets()`** — `load_settings(path)` and `load_layout(path)`, for a
  routine that depends on particular controller settings. Loading a file is
  sometimes the only way to set a module over TCP, so this is a first-class
  step, not a workaround. A load persists for the rest of the connection
  and cannot be undone; it goes to the log as an action and as a typed
  `routine/settings_loaded` or `routine/layout_loaded` event, so whoever
  runs next can see the controller's state moved.
- **`rt.controller()`** — the escape hatch to the bare `SpmController` for
  anything the handles don't cover; calls through it bypass event logging.

## Sessions and jobs

`rusty_tip::session::Session` is one connection hosting many runs: connect
once (layout and settings files loaded, signal registry built, data stream
started), run `Job`s against it one at a time, disconnect at the end. Each
job gets its own JSONL log with a `run_started` header built from the job's
schema, config and the controller facts, and `run_finished` if the job did
not write one itself. A job that is a routine is a few lines:

```rust
use rusty_tip::session::{Job, JobCx};

impl Job for TipPrepJob {
    fn name(&self) -> &str { "tip_prep" }
    fn log_schema(&self) -> ToolSchema { rusty_tip::tip_prep::log_schema() }
    fn header_config(&self) -> serde_json::Value { serde_json::to_value(&self.config).unwrap() }
    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        let fs = cx.registry.get_by_name("freq shift").unwrap().signal_index();
        let mut routine = TipPrep::new(&self.config, fs);
        run_routine(cx.controller, cx.events, cx.shutdown, &mut routine)
    }
}
```

A job error or panic is reported and the session stays connected; a
connection error marks it poisoned until `reconnect()`. `Session` is
synchronous, so a test drives it against the mock; `session::spawn` puts
one on its own thread behind `SessionCmd`/`SessionUpdate` channels, polling
a few live readouts (bias, Z, current, frequency shift) while idle. That is
how a GUI uses it: exactly one thread touches the controller.

The shipped `TipPrep` routine (`src/tip_prep/runner.rs`) is the reference:
a full state machine with confirmation reads, a guarded stability sweep,
and pulse-voltage strategies, written entirely in these verbs.

## The action system

Underneath the subsystem handles, every SPM operation is an action: a struct
that executes against an `ActionContext`. The handles construct and execute
these for you, so reach for actions directly only when you want an operation
the handles do not expose, or one without the harness around it. Actions
declare the hardware capabilities they need, and execution fails with
`Unsupported` before touching hardware if the controller lacks one.

```rust
use rusty_tip::SignalIndex;
use rusty_tip::action::bias::{BiasPulse, SetBias};
use rusty_tip::action::signals::ReadStableSignal;
use rusty_tip::action::{Action, ActionContext, DataStore};
use rusty_tip::event::EventBus;

let mut store = DataStore::new();
let events = EventBus::new();
let mut ctx = ActionContext {
    controller: &mut *controller,
    store: &mut store,
    events: &events,
    shutdown: &shutdown,
};

SetBias { voltage: -0.5 }.execute(&mut ctx)?;
BiasPulse {
    voltage: 4.0,
    duration_ms: 50,
    z_hold: true,
    absolute: true,
}
.execute(&mut ctx)?;

let output = ReadStableSignal {
    index: SignalIndex(76),
    num_samples: 100,
    max_std_dev: 1.5,      // Hz
    max_slope: 0.5,        // Hz/s
    max_retries: 3,
    sample_rate_hz: 2000.0,
}
.execute(&mut ctx)?;
```

### Built-in actions

The action layer implements these operations. Routines reach the common ones
through the subsystem handles rather than constructing actions directly:

| Category | Actions |
|----------|---------|
| **Bias** | `ReadBias`, `SetBias`, `SafeSetBias`, `BiasPulse` |
| **Signals** | `ReadSignal`, `ReadSignals`, `ReadSignalNames`, `ReadStableSignal` |
| **Z-Controller** | `Withdraw`, `AutoApproach`, `CalibratedApproach`, `SetZSetpoint`, `ZHome`, `SafeTipSet`, `ReadZControllerStatus`, `ReadSafeTipStatus` |
| **Position** | `ReadPosition`, `SetPosition` |
| **Motor** | `MoveMotor`, `MoveMotor3D`, `MoveMotorClosedLoop`, `StopMotor`, `Reposition` |
| **Scanning** | `ScanControl`, `ReadScanStatus`, `GrabScanFrame` |
| **Multi-pass** | `LoadMultiPass`, `SaveMultiPass`, `ActivateMultiPass`, `ApplyMultiPass` |
| **Drift** | `MeasureZDrift`, `CompensateDrift` |
| **Oscilloscope** | `OsciRead` |
| **Tip Shaper** | `TipShape` |
| **PLL** | `CenterFreqShift` |
| **Data Stream** | `ConfigureDataStream`, `StartDataStream`, `StopDataStream`, `ReadDataStreamStatus` |
| **Utility** | `Wait` |

## Implementing `SpmController`

The trait is the hardware seam. `NanonisController` implements it over the
Nanonis TCP protocol; `MockController` implements it in memory with a
scriptable tip model and fault injection. Yours would look like:

```rust
use rusty_tip::spm_controller::{Capability, Result, SpmController};
use std::collections::HashSet;

struct MyController { /* ... */ }

impl SpmController for MyController {
    fn capabilities(&self) -> HashSet<Capability> {
        [Capability::Bias, Capability::Signals, Capability::ZController]
            .into_iter()
            .collect()
    }

    fn get_bias(&mut self) -> Result<f64> { /* ... */ }
    fn set_bias(&mut self, voltage: f64) -> Result<()> { /* ... */ }
    // ... the trait is wide; unsupported subsystems can return
    // SpmError::Unsupported, and capabilities() tells the execution layer
    // to refuse those actions up front.
}
```

`MockController` is the reference for testing routines: it records every call
(ordering, counters, pulse voltages), lets a closure decide the frequency
shift per read, and can inject faults on any method's Nth call. See the
module docs of `rusty_tip::mock_controller`.

## Events

Everything observable flows through the `EventBus`: action started/completed/
failed, measurements with their batch statistics, and routine state
snapshots. Attach observers (`ConsoleLogger`, `FileLogger` for JSONL,
`ChannelForwarder` for GUIs) to consume them. The JSONL form, including the
self-describing header every run starts with and how a tool declares its
own event kinds, is documented in [experiment-log.md](experiment-log.md).
