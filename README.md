# rusty-tip

[![Crates.io](https://img.shields.io/crates/v/rusty-tip)](https://crates.io/crates/rusty-tip)
[![docs.rs](https://img.shields.io/docsrs/rusty-tip)](https://docs.rs/rusty-tip)
[![License: MIT](https://img.shields.io/crates/l/rusty-tip)](https://github.com/kronberger-droid/rusty-tip/blob/main/LICENSE)
[![GitHub release](https://img.shields.io/github/v/release/kronberger-droid/rusty-tip)](https://github.com/kronberger-droid/rusty-tip/releases)

Rust library and tools for automated STM/AFM tip preparation on Nanonis SPM systems.

## Overview

rusty-tip provides automated tip conditioning for Scanning Probe Microscopy (SPM) systems. It connects to Nanonis controllers via TCP and implements tip preparation algorithms with configurable pulse strategies and stability verification.

The library is built around a hardware abstraction trait (`SpmController`) and a composable action system, making it possible to script arbitrary SPM workflows or integrate with external tooling.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Binaries                                               │
│  tip-prep-v2 (CLI)  ·  tip-prep-gui (eframe/egui)      │
└────────────┬────────────────────────────┬───────────────┘
             │                            │
┌────────────▼────────────────────────────▼───────────────┐
│  Library (rusty-tip)                                    │
│                                                         │
│  ┌──────────────┐  ┌──────────┐  ┌──────────────────┐  │
│  │ tip_prep     │  │ workflow │  │ analyzer         │  │
│  │  runner      │  │  executor│  │  CuOx detector   │  │
│  │  pulse_state │  │  steps   │  │  adapter         │  │
│  └──────┬───────┘  └────┬─────┘  └────────┬─────────┘  │
│         │               │                 │             │
│  ┌──────▼───────────────▼─────────────────▼─────────┐  │
│  │  Action System (28 built-in actions)              │  │
│  │  ActionRegistry · ActionContext · DataStore       │  │
│  └──────────────────────┬────────────────────────────┘  │
│                         │                               │
│  ┌──────────────────────▼────────────────────────────┐  │
│  │  SpmController trait                              │  │
│  │  signals · bias · z-controller · motor · scan     │  │
│  │  oscilloscope · PLL · safe-tip · data stream      │  │
│  └──────────────────────┬────────────────────────────┘  │
│                         │                               │
│  ┌──────────────────────▼────────────────────────────┐  │
│  │  EventBus (observer pattern)                      │  │
│  │  ChannelForwarder · FileLogger · Accumulator      │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
             │
┌────────────▼────────────────────────────────────────────┐
│  NanonisController (SpmController impl)                 │
│  nanonis-rs TCP client · BufferedTCPReader              │
└─────────────────────────────────────────────────────────┘
```

## Features

- **Hardware Abstraction** - `SpmController` trait decouples logic from hardware; implement it for any SPM system
- **Composable Actions** - 28 built-in actions (bias, signals, motor, scan, PLL, etc.) registered in an `ActionRegistry` with serde-based parameterization
- **Automated Tip Preparation** - Pulse-and-check algorithm with configurable strategies and stability verification
- **Multiple Pulse Strategies** - Fixed voltage, adaptive stepping, or linear mapping based on signal response
- **Stability Verification** - Bias sweep testing with statistical signal analysis (std deviation + linear regression slope)
- **Event System** - Observer-based event bus for logging, GUI updates, and external monitoring
- **Workflow Engine** - Declarative workflow DSL with loops, conditions, and variable store
- **Image Analysis** - CuOx row detector with projection-based band detection
- **Real-time Monitoring** - TCP data stream with background buffering and timestamp-based sample collection
- **CLI and GUI** - Command-line and graphical (eframe/egui) interfaces
- **Configurable** - TOML configuration with all parameters exposed

## Installation

### Pre-built Binaries

Download from [GitHub Releases](https://github.com/kronberger-droid/rusty-tip/releases):

- **Linux**: `rusty-tip-x86_64-unknown-linux-gnu.tar.xz`
- **Windows**: `rusty-tip-x86_64-pc-windows-msvc.zip`

### From Source

```bash
# CLI only
cargo build --release

# With GUI
cargo build --release --features gui
```

### As a Library

```bash
cargo add rusty-tip
```

## Usage

### CLI (tip-prep-v2)

```bash
tip-prep-v2 --config path/to/config.toml
tip-prep-v2 --config config.toml --log-level debug
```

### GUI (tip-prep-gui)

Launch the application and load a configuration file. The interface provides:

- **Control Tab** - Start/stop preparation, live freq-shift and voltage plots, event stream
- **Configuration Tab** - Edit all parameters before running

## Configuration

Configuration uses TOML format. See `configs/` for examples.

### Minimal Example

```toml
[nanonis]
host_ip = "127.0.0.1"
control_ports = [6501, 6502, 6503, 6504]

[data_acquisition]
data_port = 6590
sample_rate = 2000

[tip_prep]
sharp_tip_bounds = [-1.5, 0.0]  # Frequency shift range for "sharp" (Hz)
max_cycles = 10000
max_duration_secs = 12000
initial_bias_v = -0.5
initial_z_setpoint_a = 100e-12

[pulse_method]
type = "fixed"
voltage = 4.0
polarity = "positive"
```

### Pulse Methods

**Fixed** - Constant voltage pulses:
```toml
[pulse_method]
type = "fixed"
voltage = 4.0
polarity = "positive"  # or "negative"
```

**Stepping** - Increases voltage after repeated failures:
```toml
[pulse_method]
type = "stepping"
voltage_bounds = [2.0, 6.0]
voltage_steps = 4
cycles_before_step = 2
threshold_value = 0.1
```

**Linear** - Voltage scales with current signal:
```toml
[pulse_method]
type = "linear"
voltage_bounds = [2.0, 7.0]
linear_clamp = [-20.0, 0.0]  # Map freq_shift range to voltage range
```

### Stability Checking

Optional verification that the tip remains stable under bias sweeps:

```toml
[tip_prep.stability]
check_stability = true
stable_tip_allowed_change = 0.4  # Max allowed freq_shift change (Hz)
bias_range = [0.2, 2.0]          # Sweep range (V)
bias_steps = 1000
step_period_ms = 200
polarity_mode = "both"           # "positive", "negative", or "both"
```

### Data Acquisition

Configure the TCP data stream and stable signal reading:

```toml
[data_acquisition]
data_port = 6590
sample_rate = 2000
stable_signal_samples = 100   # Samples per stability read
max_std_dev = 1.0              # Max std deviation for "stable"
max_slope = 0.01               # Max linear regression slope for "stable"
stable_read_retries = 3        # Retries with exponential backoff
```

### Full Configuration Reference

```toml
[nanonis]
host_ip = "127.0.0.1"
control_ports = [6501, 6502, 6503, 6504]
layout_file = "./layout.lyt"      # Optional
settings_file = "./settings.ini"  # Optional

[data_acquisition]
data_port = 6590
sample_rate = 2000
stable_signal_samples = 100
max_std_dev = 1.0
max_slope = 0.01
stable_read_retries = 3

[experiment_logging]
enabled = true
output_path = "./experiments"

[console]
verbosity = "info"

[tip_prep]
sharp_tip_bounds = [-1.5, 0.0]
max_cycles = 10000
max_duration_secs = 12000
initial_bias_v = -0.5
initial_z_setpoint_a = 100e-12

[tip_prep.timing]
pulse_width_ms = 50
post_approach_settle_ms = 2000
post_reposition_settle_ms = 1000
post_pulse_settle_ms = 1000
buffer_clear_wait_ms = 500
reposition_steps = [3, 3]
status_interval = 10

[tip_prep.stability]
check_stability = true
stable_tip_allowed_change = 0.4
bias_range = [0.2, 2.0]
bias_steps = 1000
step_period_ms = 200
max_duration_secs = 100
polarity_mode = "both"

[pulse_method]
# See pulse method examples above
```

## Library Usage

### Action System

The V2 API uses trait-based actions executed against an `SpmController`:

```rust
use rusty_tip::action::{builtin_registry, ActionContext, DataStore};
use rusty_tip::action::bias::{SetBias, BiasPulse};
use rusty_tip::action::signals::ReadStableSignal;
use rusty_tip::action::z_controller::Withdraw;
use rusty_tip::event::EventBus;

// Actions are structs with serde parameters
let set_bias = SetBias { voltage: -0.5 };
let pulse = BiasPulse {
    voltage: 4.0,
    duration_ms: 50,
    z_hold: true,
    absolute: true,
};
let stable_read = ReadStableSignal {
    index: 0,
    num_samples: 100,
    max_std_dev: 1.0,
    max_slope: 0.01,
    max_retries: 3,
};

// Execute against a controller
let mut store = DataStore::new();
let events = EventBus::new();
let mut ctx = ActionContext {
    controller: &mut *controller,
    store: &mut store,
    events: &events,
};
set_bias.execute(&mut ctx)?;
pulse.execute(&mut ctx)?;
```

### SpmController Trait

Implement `SpmController` for your hardware:

```rust
use rusty_tip::spm_controller::{SpmController, Capability};
use std::collections::HashSet;

struct MyController { /* ... */ }

impl SpmController for MyController {
    fn capabilities(&self) -> HashSet<Capability> {
        [Capability::Bias, Capability::Signals, Capability::ZController]
            .into_iter().collect()
    }

    fn get_bias(&mut self) -> Result<f64> { /* ... */ }
    fn set_bias(&mut self, voltage: f64) -> Result<()> { /* ... */ }
    // ... implement methods for your hardware
}
```

### Built-in Actions

| Category | Actions |
|----------|---------|
| **Bias** | ReadBias, SetBias, SafeSetBias, BiasPulse |
| **Signals** | ReadSignal, ReadSignals, ReadSignalNames, ReadStableSignal |
| **Z-Controller** | Withdraw, AutoApproach, CalibratedApproach, SetZSetpoint |
| **Position** | ReadPosition, SetPosition |
| **Motor** | MoveMotor, MoveMotor3D, MoveMotorClosedLoop, StopMotor, Reposition |
| **Scanning** | ScanControl, ReadScanStatus, GrabScanFrame |
| **Oscilloscope** | OsciRead |
| **Tip Shaper** | TipShape |
| **PLL** | CenterFreqShift |
| **Data Stream** | ConfigureDataStream, StartDataStream, StopDataStream, ReadDataStreamStatus |
| **Utility** | Wait |

### Workflow Engine

Define workflows as JSON/TOML-serializable step trees:

```rust
use rusty_tip::workflow::{Workflow, Step, Condition, CompareOp};

let workflow = Workflow::new("pulse_and_check", "Apply pulse then verify")
    .step(Step::sequence(vec![
        Step::action("set_bias", serde_json::json!({ "voltage": -0.5 })),
        Step::action("bias_pulse", serde_json::json!({
            "voltage": 4.0,
            "duration_ms": 50,
        })),
        Step::action("read_stable_signal", serde_json::json!({
            "index": 0,
            "num_samples": 100,
        })),
    ]));
```

### Tip Preparation

Run the full tip-prep algorithm from the library:

```rust
use rusty_tip::tip_prep::run_tip_prep;
use rusty_tip::config::load_config;
use rusty_tip::event::EventBus;
use rusty_tip::workflow::ShutdownFlag;

let config = load_config("config.toml")?;
let events = EventBus::new();
let shutdown = ShutdownFlag::new();

let outcome = run_tip_prep(
    controller,       // Box<dyn SpmController>
    &events,
    &shutdown,
    &config,
    freq_shift_index, // signal index for frequency shift
)?;
```

## How It Works

The tip preparation algorithm:

1. **Initialize** - Set bias, setpoint, calibrated approach
2. **Pulse Loop** (per cycle):
   - Apply voltage pulse (absolute, z-hold)
   - Settle briefly
   - Reposition (withdraw, motor move, calibrated approach)
   - Measure frequency shift at new position (stable read with retries)
   - If sharp: run confirmation + stability check
   - Update voltage strategy for next cycle
3. **Confirmation** - 3x reposition-and-measure to verify sharpness persists
4. **Stability Check** (optional):
   - Start continuous scan
   - Sweep bias through configured range
   - Withdraw and restore bias after sweep
   - Measure final frequency shift
   - Compare to baseline; if drift exceeds threshold, fire max pulse and restart
5. **Cleanup** - Withdraw tip, teardown controller

## Requirements

- Nanonis SPM controller with TCP interface enabled
- Configured TCP data logging (typically port 6590)
- Control ports accessible (typically 6501-6504)

## License

MIT
