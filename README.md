# rusty-tip

A Rust library for interfacing with Nanonis SPM (Scanning Probe Microscopy) systems via TCP protocol.

## Quick Start

```rust
use nanonis_rust::{NanonisClient, BoundaryClassifier, RuleBasedPolicy, Controller};
use std::time::Duration;

// Create client and connect to Nanonis
let client = NanonisClient::new("127.0.0.1:6501")?;

// Set up signal classification and policy
let classifier = BoundaryClassifier::new("Bias Monitor".to_string(), 24, 0.0, 2.0)
    .with_buffer_config(10, 2)
    .with_stability_config(3);
let policy = RuleBasedPolicy::new("Simple Rule Policy".to_string());

// Run automated control loop
let mut controller = Controller::with_client(client, Box::new(classifier), Box::new(policy));
controller.run_control_loop(2.0, Duration::from_secs(30))?;
```

## Examples

Run examples with different logging levels:

```bash
# Basic monitoring demo
cargo run --example boundary_monitor_demo

# Real-time boundary monitoring
RUST_LOG=debug cargo run --example real_time_boundary_monitor

# Signal reading examples
cargo run --example get_signals
cargo run --example signal_monitor_test
```

## Architecture

The library follows a modular pipeline architecture:

**Raw Signals** → **StateClassifier** → **MachineState** → **PolicyEngine** → **Actions**

- **Protocol Layer**: Low-level TCP communication with Nanonis
- **Client Layer**: High-level command interface with comprehensive Nanonis support
- **Classification**: Signal interpretation and state classification with buffering
- **Policy Engine**: Decision making based on machine state
- **Controller**: Orchestrates the complete control loop with fresh sampling

## Development

```bash
# Build and test
cargo build
cargo test

# Run specific tests
cargo test classifier
cargo test controller

# Check code
cargo check
```

See [CLAUDE.md](CLAUDE.md) for detailed development guidance and architecture documentation.
