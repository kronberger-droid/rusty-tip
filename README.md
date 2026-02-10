# rusty-tip

[![Crates.io](https://img.shields.io/crates/v/rusty-tip)](https://crates.io/crates/rusty-tip)
[![docs.rs](https://img.shields.io/docsrs/rusty-tip)](https://docs.rs/rusty-tip)
[![License: MIT](https://img.shields.io/crates/l/rusty-tip)](https://github.com/kronberger-droid/rusty-tip/blob/main/LICENSE)
[![GitHub release](https://img.shields.io/github/v/release/kronberger-droid/rusty-tip)](https://github.com/kronberger-droid/rusty-tip/releases)

Rust library and tools for automated STM/AFM tip preparation on Nanonis SPM systems.

## Overview

rusty-tip provides automated tip conditioning for Scanning Probe Microscopy (SPM) systems. It connects to Nanonis controllers via TCP and implements tip preparation algorithms with configurable pulse strategies and stability verification.

## Features

- **Automated Tip Preparation** - State machine that detects tip quality and applies conditioning pulses
- **Multiple Pulse Strategies** - Fixed voltage, adaptive stepping, or linear mapping based on signal response
- **Stability Verification** - Optional bias sweep testing to confirm tip stability
- **Real-time Monitoring** - TCP data logging with signal history tracking
- **CLI and GUI Applications** - Both command-line and graphical interfaces
- **Configurable** - TOML configuration with environment variable overrides

## Installation

### Pre-built Binaries

Download from [GitHub Releases](https://github.com/kronberger-droid/rusty-tip/releases):

- **Linux**: `rusty-tip-x86_64-unknown-linux-gnu.tar.xz`
- **Windows**: `rusty-tip-x86_64-pc-windows-msvc.zip`

Each archive contains:
- `tip-prep` / `tip-prep.exe` - Command-line tool
- `tip-prep-gui` / `tip-prep-gui.exe` - Graphical interface

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

### CLI (tip-prep)

```bash
tip-prep --config path/to/config.toml
tip-prep --config config.toml --log-level debug
```

Options:
- `--config <FILE>` - Path to TOML configuration file (required)
- `--log-level <LEVEL>` - Override log level: trace, debug, info, warn, error

### GUI (tip-prep-gui)

Launch the application and load a configuration file via the file dialog. The interface provides:

- **Control Tab** - Start/stop preparation, live status display
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

```rust
use rusty_tip::{ActionDriver, Action, NanonisClient};

// Connect to Nanonis
let client = NanonisClient::new("127.0.0.1", 6501)?;
let mut driver = ActionDriver::new(client);

// Execute actions
driver.execute(Action::ReadBias)?;
driver.execute(Action::SetBias { voltage: -0.5 })?;
driver.execute(Action::AutoApproach { center_freq_shift: true })?;
driver.execute(Action::BiasPulse {
    voltage: 4.0,
    duration_ms: 10,
    z_hold: true,
})?;
```

### Available Actions

| Category | Actions |
|----------|---------|
| Signals | ReadSignal, ReadSignals, ReadSignalNames, ReadBias, SetBias |
| Positioning | ReadPiezoPosition, SetPiezoPosition, MovePiezoRelative, MoveMotor3D |
| High-level | AutoApproach, Withdraw, SafeReposition, BiasPulse, TipShaper |
| Analysis | CheckTipState, CheckTipStability, GetStableSignal |
| Scanning | ScanControl, ReadScanStatus |
| Oscilloscope | ReadOsci |

## How It Works

The tip preparation follows a state machine:

1. **Blunt** - Tip quality below threshold
   - Apply voltage pulse
   - Reposition (withdraw, move, approach)
   - Check if now sharp

2. **Sharp** - Tip quality within bounds
   - Verify with multiple repositions
   - If stability checking enabled: perform bias sweep
   - If stable, mark complete; otherwise pulse and return to Blunt

3. **Stable** - Preparation complete

## Requirements

- Nanonis SPM controller with TCP interface enabled
- Configured TCP data logging (typically port 6590)
- Control ports accessible (typically 6501-6504)

## License

MIT
