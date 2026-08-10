# Library guide

`rusty-tip` is usable as a library: bring your own binary, compose the
built-in actions, or implement `SpmController` for non-Nanonis hardware.
Current as of 0.4.0.

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

## The action system

Every SPM operation is an action: a serializable struct that executes against
an `ActionContext`. Actions declare the hardware capabilities they need, and
execution fails with `Unsupported` before touching hardware if the controller
lacks one.

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

`action::builtin_registry()` returns an `ActionRegistry` with all of these,
constructible by name from JSON parameters:

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

## Workflow engine (experimental)

The `workflow` module holds a declarative `Step`/`Condition` tree executor.
It has no production consumer yet and is expected to be redesigned around a
routine-harness interface; expect breaking changes or removal in 0.5.x. New
automations should follow the shape of `tip_prep::run_tip_prep` instead.
