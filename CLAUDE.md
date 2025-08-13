# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with the `rusty-tip` library.

## Build and Development Commands

```bash
# Build the library
cargo build

# Run examples
cargo run --example boundary_monitor_demo
cargo run --example real_time_boundary_monitor
cargo run --example machine_test
cargo run --example get_signals
cargo run --example signal_monitor_test

# Run tests
cargo test

# Run specific test modules
cargo test classifier
cargo test controller
cargo test policy

# Check for compilation errors without building
cargo check
```

## Logging Configuration

The library uses the `log` crate with configurable logging levels. Set the `RUST_LOG` environment variable to control verbosity:

```bash
# Maximum verbosity - shows all internal operations
RUST_LOG=trace cargo run --example boundary_monitor_demo

# Debug level - shows detailed operational info
RUST_LOG=debug cargo run --example boundary_monitor_demo

# Info level (default) - shows normal operation messages
RUST_LOG=info cargo run --example boundary_monitor_demo

# Warning level - shows only warnings and errors
RUST_LOG=warn cargo run --example boundary_monitor_demo

# Error level - shows only errors
RUST_LOG=error cargo run --example boundary_monitor_demo
```

**Log Levels Used:**
- **`trace`**: Very detailed internal state (buffer contents, signal values)
- **`debug`**: Detailed operational info (good signals, stability tracking, pulse operations)
- **`info`**: Normal operation messages (control loop start/stop, major actions, stable signals)
- **`warn`**: Warnings (bad signals, recovery actions)
- **`error`**: Errors (connection failures, protocol errors)

## Architecture Overview

This is a Rust library for interfacing with Nanonis SPM (Scanning Probe Microscopy) systems via TCP protocol. The architecture has evolved to use a **separated, modular design** with clear data flow and responsibilities.

### Key Architectural Changes

The library now uses **`MachineState`** as the central data structure, replacing the previous `TipState`. This represents a significant architectural shift toward a more structured approach:

- **Raw Signals** → **StateClassifier** → **Enriched MachineState** → **PolicyEngine** → **Actions**
- State classification is now **in-place** on `MachineState` objects
- Fresh signal sampling with configurable buffer sizes
- Enhanced debugging and monitoring capabilities

### Core Components

#### **Types Layer** (`src/types.rs`)
- **`NanonisValue`**: Enum for all Nanonis protocol data types with comprehensive conversion traits
- **`BiasVoltage`**, **`Position`**: Type-safe wrappers for common values
- **`MachineState`**: **Central state representation** containing:
  - Current signal readings (`primary_signal`, `all_signals`)
  - Spatial context (`position`, `z_position`) 
  - Temporal context (`timestamp`, `signal_history`)
  - System state (`approach_count`, `last_action`, `system_parameters`)
  - **Classification result** (`classification: TipState`)
  - ML expansion fields (commented for future use)

#### **Error Handling** (`src/error.rs`)
- **`NanonisError`**: Comprehensive error types with detailed context
- Covers IO, protocol, type, and command errors using `thiserror`

#### **Protocol Layer** (`src/protocol.rs`)
- Low-level TCP protocol implementation for Nanonis communication
- Big-endian serialization/deserialization of `NanonisValue` types
- Message headers, validation, and protocol constants
- Supports integers, floats, arrays, strings

#### **Client Layer** (`src/client.rs`)
- **`NanonisClient`** with builder pattern for flexible configuration
- **Comprehensive Nanonis command support**:
  - **Signals**: `ValsGet`, `NamesGet`, `CalibrGet`, `RangeGet`
  - **Control**: `Bias.Set/Get`, `FolMe.XYPosSet/Get`, `ZCtrl.Withdraw`
  - **Automation**: `AutoApproach.*`, `Motor.*` commands
- Connection management with configurable timeouts and retry logic
- Type-safe method interfaces

#### **State Classification** (`src/classifier.rs`)
- **`StateClassifier`** trait: Converts raw signals into interpreted machine states
- **`BoundaryClassifier`**: Advanced boundary-based classification with:
  - **Fresh sampling integration**: Uses `signal_history` from `MachineState`
  - **Drop-front buffering**: Configurable buffer size and drop count
  - **Stability tracking**: Consecutive good readings for stable classification  
  - **In-place classification**: Updates `MachineState.classification` directly
- **`TipState`**: Simple enum (`Bad`, `Good`, `Stable`) with `Default` trait

#### **Policy Engine** (`src/policy.rs`)
- **`PolicyEngine`** trait: Makes decisions based on `MachineState`
- **`RuleBasedPolicy`**: Simple mapping from classification to decision
- **`PolicyDecision`**: Good/Bad/Stable decision types
- **Extensible design** for future ML/transformer-based policies with learning traits

#### **Controller** (`src/controller.rs`)
- **High-level orchestration** integrating all components
- **Fresh sampling strategy**: Collects multiple samples per monitoring cycle
- **State-driven actions**: Complex action sequences based on policy decisions
- **Rich context tracking**: Position history, action history, approach counts
- **ML-ready architecture**: Placeholder methods for future expansion

### Data Flow Architecture

```
1. Controller collects fresh samples → MachineState.signal_history
2. StateClassifier.classify(machine_state) → Updates classification in-place  
3. Controller enriches MachineState with position, signal names, etc.
4. PolicyEngine.decide(machine_state) → PolicyDecision
5. Controller executes actions based on decision
```

### Nanonis Protocol Integration

**Signal Operations:**
- `Signals.ValsGet` - Read multiple signal values with wait-for-newest option
- `Signals.NamesGet` - Get available signal names  
- `Signals.CalibrGet` - Get signal calibration and offset
- `Signals.RangeGet` - Get signal range limits

**Control Operations:**
- `Bias.Set` / `Bias.Get` - Control bias voltage
- `FolMe.XYPosSet` / `FolMe.XYPosGet` - Position control with type-safe Position struct
- `ZCtrl.Withdraw` - Tip withdrawal with timeout control

**Automation:**
- `AutoApproach.Open` / `AutoApproach.OnOffSet` / `AutoApproach.OnOffGet` - Auto-approach control
- `Motor.StartMove` / `Motor.StartClosedLoop` / `Motor.StopMove` - Coarse positioning
- `Motor.PosGet` / `Motor.StepCounterGet` / `Motor.FreqAmpGet/Set` - Motor status and control

### Current Usage Pattern

The separated architecture is demonstrated in `boundary_monitor_demo.rs`:

```rust
// Create client
let client = NanonisClient::new("127.0.0.1:6501")?;

// Create classifier for signal interpretation
let classifier = BoundaryClassifier::new(
    "Bias Boundary Classifier".to_string(),
    24,  // Signal index (bias voltage)
    0.0, // min bound (V)
    2.0, // max bound (V)
)
.with_buffer_config(10, 2)    // 10 samples, drop first 2
.with_stability_config(3);    // 3 consecutive good for stable

// Create policy for decision making
let policy = RuleBasedPolicy::new("Simple Rule Policy".to_string());

// Integrate with controller
let mut controller = Controller::with_client(
    client, 
    Box::new(classifier), 
    Box::new(policy)
);

// Run automated control loop
controller.run_control_loop(2.0, Duration::from_secs(30))?;
```

## Development Notes

### Architecture Principles
- **Separated concerns**: Raw signals → classification → policy → actions
- **Type safety**: Extensive use of type-safe wrappers and conversion traits
- **Fresh sampling**: Controller actively collects fresh samples for classification
- **In-place updates**: `MachineState` is modified in-place by classifiers
- **ML readiness**: Architecture designed for future transformer/ML policy engines

### Technical Details
- **Protocol**: Big-endian byte order for all data types
- **Signal indices**: Typically 0-127, with bias voltage commonly at index 24
- **Buffering**: Classifiers use configurable buffering with drop-front analysis
- **Error handling**: Comprehensive error types with detailed context
- **Testing**: Unit tests for all core components with mocking support

### Key Behavioral Changes
- `StateClassifier.classify()` now takes `&mut MachineState` instead of returning `TipState`
- `PolicyEngine.decide()` now takes `&MachineState` instead of `&TipState`  
- Controller performs fresh sampling (10 samples per cycle) instead of single reads
- `MachineState` replaces `TipState` as the central data structure
- Classification and enrichment happen in-place on the state object

### Future Expansion Points
- **Advanced Classifiers**: Statistical, multi-signal, frequency-domain analysis
- **ML Policy Engines**: Neural networks, transformers, reinforcement learning
- **Data Management**: Logging, real-time plotting, configuration management
- **Robustness**: Enhanced error recovery, simulation modes, benchmarking