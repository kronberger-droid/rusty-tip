<p align="center">
  <img src="assets/branding/logo/rusty-tip-lockup-4x.png" alt="rusty tip" width="320">
</p>

[![Crates.io](https://img.shields.io/crates/v/rusty-tip)](https://crates.io/crates/rusty-tip)
[![docs.rs](https://img.shields.io/docsrs/rusty-tip)](https://docs.rs/rusty-tip)
[![License: MIT](https://img.shields.io/crates/l/rusty-tip)](https://github.com/kronberger-droid/rusty-tip/blob/main/LICENSE)
[![GitHub release](https://img.shields.io/github/v/release/kronberger-droid/rusty-tip)](https://github.com/kronberger-droid/rusty-tip/releases)
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.20325230.svg)](https://doi.org/10.5281/zenodo.20325230)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/kronberger-droid/rusty-tip)

A Rust library for building automated scanning probe microscopy routines,
shipping with automated STM/AFM tip preparation as its first end-to-end
routine.

> [!NOTE]
> 0.5 is the first release of the rewrite (the control path rebuilt around
> the `SpmController` trait, the routine harness and the event log) that has
> run on a real microscope: it was validated over a two-week machine session.
> The fixes that session's logs led to, in the stable read and in drift
> compensation's settling, came after it and are tested against a simulated
> instrument, not yet on hardware. Before 1.0 the API and the config format can still change between minor
> versions; the [CHANGELOG](CHANGELOG.md) says what moved.
>
> If you need a stable interface to program against today, use
> [nanonis-rs](https://crates.io/crates/nanonis-rs), the Nanonis TCP client
> this library is built on, or the official Python package
> [nanonis_spm](https://github.com/SPECS-Zurich-GmbH/nanonis_spm) from SPECS.

The core of rusty-tip is a library for writing SPM automation: a
hardware-abstraction trait (`SpmController`), a vocabulary of composable
actions with capability checking, an event stream for observability, and a
scriptable mock controller so routines can be developed and tested without
instrument time. Routines written against it are controller-agnostic; today
Nanonis (over TCP) is the only instrument backend, and the trait is the seam
for adding others.

The first routine built on this foundation, and the proof of concept for the
design, is automated tip preparation: pulse, reposition, measure the
frequency shift, repeat until the tip is sharp and provably stable. It ships
as a CLI (`tip-prep`) and as a tool in the workbench (`rusty-tip-gui`).

## The library

- **One controller interface.** `SpmController` is everything a routine
  asks of an instrument, with the capabilities each backend reports. The
  Nanonis backend talks TCP through [nanonis-rs](https://crates.io/crates/nanonis-rs);
  `MockController` simulates a tip, a Z loop, drift and faults, so routines
  are written and tested without instrument time.
- **Routines.** A `Routine` runs against `Rt`, which hands out
  capability-checked handles (bias, Z-controller, motor, scan, signals,
  drift, multi-pass, presets), interruptible waits and cycle or time
  budgets. `run_routine` cleans up however a routine ends, a panic
  included: withdraw, retract, safe-tip put back.
- **Sessions.** `Session` holds one connection across many runs, on its own
  thread for a window or a server.
- **Experiment logs.** Every run writes a self-describing JSONL log: each
  action with its parameters and timing, every measurement, typed events
  and the controller's facts. `experiment_log::reader` reads it back.
- **Tip preparation.** `TipPrep` pulses, repositions and reads the frequency
  shift until the tip is sharp, then sweeps the bias to check it stays so.
  Pulse strategies, a landing gate on the current, and a config that can
  change between cycles of a running prep.
- **Drift.** Measure the Z drift with an error bar and set the piezo
  compensation that cancels it.
- **Feedback controllers.** The Z-controller and the PLL's loops as
  parameter sets to read, edit and write, presets by name, and operating
  points: every loop, the bias and the scan under one name.
- **Multi-pass.** Read and write Nanonis `.mpas` files byte for byte and
  load them on the controller.
- **Analysis and export.** Constant-distance trajectories from a topograph
  (rolling ellipsoid), a CuOx row detector, Gwyddion `.gsf` and XYZ export.
- **An interface for agents.** `control` serves JSON requests on a local
  socket, behind a read-only gate and limits.

## The toolbox

| Tool | What it does |
| --- | --- |
| `rusty-tip-gui` | The workbench. Connect once, then run a tool and watch it live: **Tip prep**, **Drift**, **Controllers** (operating points and loops). The Connection page shows the controller's state, signals and live readouts, History opens any past run from its log, and a `Mock` backend runs everything without hardware. |
| `rusty-tip` | A JSON command line for scripts and agents: `describe`, `status`, `read`, `controllers`, `scan`, `frame`, `psd`, and `serve` for a headless connection. Read-only for now. |
| `tip-prep` | Tip preparation from a TOML config, without a window. |
| `rt-log` | Experiment logs in the terminal: `ls`, `summary`, `timeline`, `plot`, and `export` to CSV. |
| `const-distance` | `plan` a constant-distance trajectory from a topograph; `baseline` sets up the constant-lift multi-pass scan it is compared against. |
| `cuox-finder` | CuOx reconstruction rows in STM images of Cu(110), as JSON (`--features cuox`). |

## Installation

Pre-built binaries from [GitHub Releases](https://github.com/kronberger-droid/rusty-tip/releases):
`rusty-tip-x86_64-unknown-linux-gnu.tar.xz` (Linux) or
`rusty-tip-x86_64-pc-windows-msvc.zip` (Windows).

From source:

```bash
cargo build --release                 # rusty-tip, tip-prep, const-distance, rt-log
cargo build --release --features cuox # + cuox-finder
cargo build --release --features gui  # + rusty-tip-gui
```

With Nix, `nix run github:kronberger-droid/rusty-tip` starts the workbench.

As a library: `cargo add rusty-tip`.

## Documentation

- **[Library guide](docs/library.md)** — the action system, implementing
  `SpmController` for your hardware, writing routines of your own
- **[Configuration reference](docs/tip-prep/config.md)** — every section and
  field of the tip-prep config, pulse strategies, stability checking, TCP
  channel mapping, with working configs in `configs/`
- **[The command line for agents](docs/agent-cli.md)** — `rusty-tip`, its
  commands, replies, exit codes and limits
- **[Experiment logs](docs/experiment-log.md)** — the JSONL format every run
  writes
- **[How tip preparation works](docs/tip-prep/algorithm.md)** — the routine,
  cycle by cycle
- **[Constant-distance scanning](docs/const-distance-scan.md)** — planning a tip
  trajectory from a topograph with the rolling-ellipsoid construction, and
  exporting maps for Gwyddion
- **[CHANGELOG](CHANGELOG.md)** — release notes

## Requirements (Nanonis backend)

- Nanonis SPM controller with TCP interface enabled
- Configured TCP data logging (typically port 6590)
- Control ports accessible (typically 6501-6504)

## License

MIT

## AI use

[![AI:R READ](https://img.shields.io/badge/AI-READ-grey?style=flat&logoSize=auto&labelColor=13171f)](https://0rwx.com)
[![AI:W WRITE](https://img.shields.io/badge/AI-WRITE-grey?style=flat&logoSize=auto&labelColor=13171f)](https://0rwx.com)
[![AI:X EXECUTE](https://img.shields.io/badge/AI-EXECUTE-grey?style=flat&logoSize=auto&labelColor=13171f)](https://0rwx.com)

rusty-tip is a mix of all three on the [0rwx](https://0rwx.com) scale:
parts I wrote myself with an LLM only reading along (R), parts written with
Claude Code that I read and fixed (W), and tooling I checked by running it
rather than line by line (X).
