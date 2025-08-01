# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

```bash
# Build the library
cargo build

# Run examples (replace with specific example name)
cargo run --example pulse_test
cargo run --example simple_policy_demo

# Run tests
cargo test

# Check for compilation errors without building
cargo check
```

## Architecture Overview

This is a Rust library for interfacing with Nanonis SPM (Scanning Probe Microscopy) systems via TCP protocol. The architecture follows a layered approach:

### Core Components

- **Protocol Layer** (`src/protocol.rs`): Low-level TCP protocol implementation for Nanonis communication
  - Handles message headers, serialization/deserialization of `NanonisValue` types
  - Supports various data types: integers, floats, arrays, strings with big-endian encoding
  
- **Client Layer** (`src/client.rs`): High-level API wrapper around the protocol
  - `NanonisClient` provides type-safe methods like `signals_val_get()`, `set_bias()`, `signal_names_get()`
  - Manages TCP connections, timeouts, and retry logic
  
- **Policy Engine** (`src/policy.rs`): Decision-making system for automated control
  - `PolicyEngine` trait for implementing different control strategies
  - `RuleBasedPolicy` for boundary-based signal monitoring with buffering
  - Supports signal buffering with configurable drop-front and max-value analysis

- **AFM Controller** (`src/afm_controller.rs`): High-level orchestration layer
  - Integrates `NanonisClient` with `PolicyEngine` for automated monitoring
  - Supports different loop modes and sample rates

### Key Data Types

- `NanonisValue`: Enum representing all possible data types in Nanonis protocol
- `NanonisError`: Comprehensive error handling for network, protocol, and type errors
- `PolicyDecision`: Simple enum for Continue/OutOfBounds decisions

### Nanonis Protocol Integration

The library implements the Nanonis TCP protocol for commands like:
- `Signals.ValGet` / `Signals.ValsGet` - Read signal values
- `Signals.NamesGet` - Get available signal names  
- `Bias.Set` / `Bias.Get` - Control bias voltage
- `FolMe.XYPosSet` - Set XY position

### Policy Engine Pattern

The policy engine uses a simple pattern:
1. Read signal values via `signals_val_get(vec![signal_index], true)`
2. Buffer values and apply drop-front + max-value analysis
3. Check boundaries and return `Continue` or `OutOfBounds`
4. Controller can halt or continue based on policy decision

### Example Usage Pattern

Most examples follow this structure:
```rust
let mut client = NanonisClient::new("127.0.0.1:6501")?;
let policy = RuleBasedPolicy::new("Monitor".to_string(), signal_index, min, max);
let controller = AFMController::new("127.0.0.1:6501", Box::new(policy))?;
controller.run_monitoring_loop(signal_index, loop_mode, sample_rate)?;
```

## Development Notes

- The protocol implementation expects big-endian byte order for all data types
- Signal indices are typically 0-127, with common signals like bias voltage at index 24
- Policy engines should be designed to work with single signal values for boundary checking
- The `signals_val_get()` method always takes `Vec<i32>` for signal indices, even for single signals