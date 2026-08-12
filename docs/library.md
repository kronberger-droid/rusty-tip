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

`run_tip_prep` owns the controller life cycle (prepare/teardown, withdraw on
exit) and returns an `Outcome` — `Completed`, `StoppedByUser`, `CycleLimit`,
or `TimedOut`. A shutdown request is an expected ending, never an error.

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

Run it with `run_routine(controller, &events, &shutdown, &mut routine)`,
which owns the controller life cycle: `prepare()` before, tip withdrawal and
`teardown()` after, whatever the outcome. A stop request (Ctrl+C, GUI
button) surfaces as `Outcome::StoppedByUser`, never as an error. "Whatever
the outcome" covers panics too: a panicking routine is withdrawn and torn
down first, then the panic is re-raised unchanged.

`run_routine` takes the controller by value, so for now a controller runs
exactly one routine: there is no way to prepare a tip with one routine and
measure with the next on the same connection. Sequence such work inside a
single `Routine` until that changes.

The pieces, in the order you meet them:

- **Subsystem handles** — `rt.bias()?`, `rt.z()?`, `rt.signals()?`,
  `rt.motor()?`, `rt.scan()?`. Each accessor checks the controller's
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
- **`rt.set_safe_tip_guard(bool)`** — when armed, a calibrated approach
  aborts if the controller reports safe-tip protection has tripped. Off
  unless asked for: implement `Routine::safe_tip_guard` to arm it for a
  whole run, or call this to arm it around one section. It covers the
  approach only, which is where the tip is driven at the surface; checking
  after every action would mostly catch misfires.
- **`rt.controller()`** — the escape hatch to the bare `SpmController` for
  anything the handles don't cover; calls through it bypass event logging.

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
use rusty_tip::action::{Action, ActionContext};
use rusty_tip::event::EventBus;

let events = EventBus::new();
let mut ctx = ActionContext {
    controller: &mut *controller,
    events: &events,
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
`ChannelForwarder` for GUIs) to consume them.

An action's started event carries its parameters and its completed event
carries its result, both serialized from the action's own types, so the log
says a pulse was 4.0 V for 50 ms rather than only that a pulse happened:

```json
{"type":"action_started","action":"bias_pulse","params":{"voltage":4.0,"duration_ms":50,"z_hold":true,"absolute":true}}
{"type":"action_completed","action":"bias_pulse","output":null,"duration":51.2}
```
