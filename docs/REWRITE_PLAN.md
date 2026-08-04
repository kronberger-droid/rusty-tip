# Rusty-Tip Rewrite Plan

A comprehensive plan for rewriting `rusty-tip` into a composable, LLM-friendly SPM
workflow automation platform. This document covers both the foundation work needed in
`nanonis-rs` and the full architectural redesign of `rusty-tip`.

---

## Table of Contents

1. [Vision and Goals](#vision-and-goals)
2. [Phase 0: Stabilize nanonis-rs](#phase-0-stabilize-nanonis-rs)
3. [Phase 1: Hardware Abstraction Layer](#phase-1-hardware-abstraction-layer)
4. [Phase 2: Trait-Based Action System](#phase-2-trait-based-action-system)
5. [Phase 3: Machine State Tracking](#phase-3-machine-state-tracking)
6. [Phase 4: Event and Observation System](#phase-4-event-and-observation-system)
7. [Phase 5: Workflow Engine](#phase-5-workflow-engine)
8. [Phase 6: LLM Integration Layer](#phase-6-llm-integration-layer)
9. [Phase 7: Data Pipeline](#phase-7-data-pipeline)
10. [Phase 8: Procedure Trait and Reimplement tip-prep](#phase-8-procedure-trait-and-reimplement-tip-prep)
11. [Design Decisions](#design-decisions)
12. [Migration Strategy](#migration-strategy)

---

## Vision and Goals

**Current state**: `rusty-tip` is a well-structured but tightly coupled library. The
tip-prep workflow is hardcoded as imperative Rust in `TipController`. Adding new
workflows requires writing Rust code. The action vocabulary (~30 actions) is solid but
locked behind an enum that can only be extended by modifying the library.

**Target state**: A platform where SPM workflows are:
- **Composable** -- built from small, independent actions combined with control flow
- **Declarative** -- defined as data (JSON/TOML), not imperative code
- **Observable** -- every action emits structured events for logging, GUIs, and analysis
- **LLM-accessible** -- actions expose JSON Schema descriptions; an LLM can discover,
  plan, and execute workflows through a standard tool-calling interface
- **Testable** -- hardware is abstracted behind a trait; workflows run against mocks
- **Synchronous** -- the entire control path stays blocking; no async runtime

---

## Phase 0: Stabilize nanonis-rs

Before touching `rusty-tip`, fix the foundation. These are bugs and maintenance hazards
in `nanonis-rs` that will bite during the rewrite.

### 0.1 Fix the duplicated parsing logic in protocol.rs

**Problem**: `parse_response` (line ~457) and `calculate_cursor_position` (line ~200)
contain ~200 lines of nearly identical match arms. One parses values, the other just
tracks byte position. If a new type descriptor is added to one and not the other, the
protocol silently breaks.

**Fix**: Unify into a single `parse_response_inner` that both parses values and tracks
cursor position. The cursor position is always computable from the parsed values since
every `NanonisValue` variant has a known wire size.

```rust
// Instead of two functions, one function that always returns both
fn parse_response_full(
    body: &[u8],
    type_descriptors: &[&str],
) -> Result<(Vec<NanonisValue>, usize), NanonisError> {
    // single parsing loop that tracks position as it goes
}
```

**Design insight**: The reason these two functions exist separately is that
`parse_response_with_error_check` needs to know where the data ends and the error
section begins. But since each parsed value has a deterministic wire size, the cursor
position is just the sum of all parsed value sizes. Computing it from the values
themselves eliminates the duplication entirely.

### 0.2 Fix runtime bugs

1. **`scan_frame_get` type mismatch** (`scan/mod.rs:54-55`): Calls `as_f64()` on values
   parsed as `"f"` (f32). The protocol returns `NanonisValue::F32` but the code tries to
   extract f64. Fix: use `as_f32()` or change return type descriptors to `"d"`.

2. **`scan_frame_data_grab` 2D array handling** (`scan/mod.rs:436-443`): Calls
   `as_f32_array()` on a `NanonisValue::Array2DF32` value. This will return a type
   mismatch error. Fix: add `as_f32_2d_array()` accessor or match the correct variant.

3. **`tip_shaper_config_get` panics** (`tip_recovery.rs:431-439`): Uses `panic!()` for
   invalid return values. Fix: return `NanonisError::Protocol(...)`.

4. **`tcplog_status_get` debug println** (`tcplog/mod.rs:195`): Remove the stray
   `println!("{result:?}")` or replace with `log::debug!`.

### 0.3 Fix type safety inconsistencies

- **`MotorAxis::from(u16)`** silently falls back to `MotorAxis::All` for unknown values.
  Change to `TryFrom<u16>` returning an error, consistent with `MotorDirection`.
- **`ChannelIndex::from(u8)`** silently clamps to 23 with a log warning. Change to
  `TryFrom<u8>` returning an error.
- **Dead enum variants**: `ArrayU16`, `ArrayI16` in `NanonisValue` have no serialization
  or deserialization support. Either implement them or remove them to avoid confusion.

### 0.4 Clean up structural issues

- **Remove `src/mod.rs`**: This file re-exports things already exported by `lib.rs`. It
  is a leftover from a refactoring and adds confusion. All re-exports should live in
  `lib.rs` only.
- **Update CLAUDE.md**: The Drop impl documentation says `ZPlus` but the code correctly
  uses `ZMinus`. The error enum documentation lists 6 variants but only 4 exist. Fix the
  docs to match reality.

### 0.5 Consider: Make `quick_send` more ergonomic for rusty-tip

Currently every command method manually indexes into `Vec<NanonisValue>` results:
```rust
let center_x = result[0].as_f32()?;
let center_y = result[1].as_f32()?;
```

This is fragile -- off-by-one in the index silently reads the wrong field. Consider a
helper that destructures with better errors:

```rust
// Something like this, exact API to be designed
let (center_x, center_y, width, height, angle) =
    result.extract::<(f32, f32, f32, f32, f32)>()?;
```

**Design insight**: This is not strictly necessary for the rewrite but would make
`nanonis-rs` significantly more pleasant to use and harder to misuse. The current
index-based extraction is error-prone because the compiler cannot verify that `result[3]`
corresponds to the 4th return type descriptor. A tuple-extraction API uses the type
system to enforce correctness. This could be implemented as a trait with a macro for
common tuple sizes.

---

## Phase 1: Hardware Abstraction Layer

### Goal

Decouple `rusty-tip` from `NanonisClient` so that:
- Actions can run against mock hardware (testing)
- Actions can run against a simulated tip model (LLM training, workflow validation)
- The same workflow can target different SPM controllers in the future

### Design

```rust
/// The minimal hardware interface that rusty-tip needs.
/// Each method maps to a physical capability, not a Nanonis command.
pub trait SpmController: Send {
    // -- Signals --
    fn read_signal(&mut self, signal: &str) -> Result<f64>;
    fn read_signals(&mut self, signals: &[&str]) -> Result<Vec<f64>>;
    fn signal_names(&mut self) -> Result<Vec<String>>;

    // -- Bias --
    fn get_bias(&mut self) -> Result<f64>;
    fn set_bias(&mut self, voltage: f64) -> Result<()>;
    fn bias_pulse(&mut self, voltage: f64, duration: Duration, z_hold: bool) -> Result<()>;

    // -- Z-Controller --
    fn withdraw(&mut self, wait: bool) -> Result<()>;
    fn auto_approach(&mut self) -> Result<()>;
    fn set_z_setpoint(&mut self, setpoint: f64) -> Result<()>;
    fn get_z_position(&mut self) -> Result<f64>;

    // -- Positioning --
    fn get_position(&mut self) -> Result<Position>;
    fn set_position(&mut self, pos: Position) -> Result<()>;
    fn move_motor(&mut self, direction: MotorDirection, steps: u16) -> Result<()>;

    // -- Scanning --
    fn scan_start(&mut self, direction: ScanDirection) -> Result<()>;
    fn scan_stop(&mut self) -> Result<()>;
    fn scan_status(&mut self) -> Result<bool>;

    // -- Data stream --
    fn start_data_stream(&mut self, channels: &[u32], sample_rate: f64) -> Result<()>;
    fn stop_data_stream(&mut self) -> Result<()>;
    fn read_stream_data(&self, duration: Duration) -> Result<Vec<SignalFrame>>;
}
```

**Design insight**: The trait uses `&mut self` (not `&self`) because the underlying TCP
connection is inherently stateful and non-shareable. This is intentional -- it makes the
ownership model explicit and prevents accidental concurrent access. If you later need
shared access, the caller wraps in `Mutex<Box<dyn SpmController>>`, which makes the
synchronization point visible.

The trait uses `f64` everywhere even though Nanonis uses `f32` on the wire. This is
deliberate -- `f64` is the natural precision for scientific computing and avoids
precision loss when values flow into analysis pipelines. The `NanonisController`
implementation handles the f32<->f64 conversion internally.

### Implementations

```rust
/// Real hardware via nanonis-rs
pub struct NanonisController {
    client: NanonisClient,
    signal_registry: SignalRegistry,
    tcp_reader: Option<BufferedTCPReader>,
}

/// For unit testing -- records calls, replays responses
pub struct MockController {
    call_log: Vec<RecordedCall>,
    responses: VecDeque<MockResponse>,
}

/// For workflow validation and LLM training
pub struct SimulatedController {
    tip_state: SimulatedTipState,
    rng: StdRng,
}
```

**Design insight**: `SimulatedController` is where things get interesting for LLM
integration. It models a simplified tip physics: the probability of transitioning from
Blunt to Sharp depends on pulse voltage and count (higher voltage = higher probability
but also higher risk of destroying the tip). An LLM agent can practice workflow design
against this simulation without access to a real multi-million-dollar SPM system. The
simulation parameters can be tuned to match empirical data from real tip-prep sessions
logged by the event system (Phase 3).

### What stays the same

- `SignalRegistry` stays as-is, it's already well-designed
- `BufferedTCPReader` stays as-is, moves inside `NanonisController`

---

## Phase 2: Trait-Based Action System

### Goal

Replace the monolithic `Action` enum with an open, extensible action system where:
- Each action is a self-contained struct implementing a trait
- Actions are discoverable at runtime (for LLMs)
- Actions can be defined outside the library (user-defined actions)
- Actions carry their own parameter schemas

### Core types

```rust
/// Every action implements this trait
pub trait Action: Send + Sync {
    /// Unique identifier, e.g. "read_signal", "bias_pulse"
    fn name(&self) -> &str;

    /// Human-readable description for documentation and LLM context
    fn description(&self) -> &str;

    /// Execute against any SpmController
    fn execute(&self, ctx: &mut ActionContext) -> Result<ActionOutput>;
}

/// Context passed to every action execution
pub struct ActionContext<'a> {
    /// The hardware (or mock/simulation)
    pub controller: &'a mut dyn SpmController,
    /// Shared key-value store for passing data between actions
    pub store: &'a mut DataStore,
    /// Event emitter for observability
    pub events: &'a dyn EventEmitter,
    /// Cancellation check
    pub shutdown: &'a ShutdownFlag,
}

/// What an action returns
pub enum ActionOutput {
    /// Single numeric value
    Value(f64),
    /// Multiple values with labels
    Values(Vec<(String, f64)>),
    /// Structured data (for complex returns)
    Data(serde_json::Value),
    /// No meaningful return
    Unit,
}
```

**Design insight**: `ActionContext` uses `&mut dyn SpmController` (dynamic dispatch)
rather than a generic `C: SpmController`. This is a deliberate trade-off: we lose ~5ns
per call from vtable indirection but gain the ability to store heterogeneous actions in
collections, pass them across API boundaries, and serialize/deserialize them. For
hardware control where each call takes milliseconds over TCP, the vtable overhead is
unmeasurable. The generic approach would infect every type with `<C: SpmController>`,
making the API harder to use and preventing type-erased action collections.

### The DataStore

```rust
/// Typed key-value store for inter-action communication
pub struct DataStore {
    values: HashMap<String, serde_json::Value>,
}

impl DataStore {
    pub fn set<T: Serialize>(&mut self, key: &str, value: &T) { ... }
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> { ... }
    pub fn remove(&mut self, key: &str) -> Option<serde_json::Value> { ... }
}
```

**Design insight**: This replaces the current `Action::Store` and `Action::Retrieve`
enum variants. Using `serde_json::Value` as the internal representation means any
serializable type can be stored and retrieved, including complex structs. The serialization
boundary also means the store contents can be logged, inspected by LLMs, and
persisted to disk. The trade-off is a small serialization overhead per store/retrieve,
but this is negligible compared to TCP command latency.

### Action Registry

```rust
pub struct ActionRegistry {
    actions: HashMap<String, ActionFactory>,
}

/// Factory that creates action instances from parameters
type ActionFactory = Box<dyn Fn(serde_json::Value) -> Result<Box<dyn Action>>>;

impl ActionRegistry {
    pub fn new() -> Self { ... }

    /// Register a built-in or user-defined action
    pub fn register<A: Action + DeserializeOwned + 'static>(&mut self) { ... }

    /// List all registered action names and descriptions
    pub fn list(&self) -> Vec<ActionInfo> { ... }

    /// Create an action instance from a name and JSON parameters
    pub fn create(&self, name: &str, params: serde_json::Value) -> Result<Box<dyn Action>> { ... }

    /// Get JSON Schema for an action's parameters
    pub fn schema(&self, name: &str) -> Option<serde_json::Value> { ... }
}
```

### Example: Converting a current Action variant to the new system

Current (enum-based):
```rust
// In actions.rs -- part of the ~30-variant enum
Action::BiasPulse { voltage, duration, z_hold }

// In action_driver.rs -- part of the ~500-line match
Action::BiasPulse { voltage, duration, z_hold } => {
    self.client.bias_pulse(true, duration, voltage, ZControllerHold::Hold, PulseMode::Normal)?;
    ActionResult::Success
}
```

New (trait-based):
```rust
/// Each action is its own struct with named fields
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BiasPulse {
    pub voltage: f64,
    pub duration_ms: u64,
    #[serde(default = "default_true")]
    pub z_hold: bool,
}

impl Action for BiasPulse {
    fn name(&self) -> &str { "bias_pulse" }
    fn description(&self) -> &str {
        "Apply a voltage pulse to the bias. Used for tip conditioning."
    }

    fn execute(&self, ctx: &mut ActionContext) -> Result<ActionOutput> {
        ctx.events.emit(Event::ActionStarted {
            action: self.name().into(),
            params: serde_json::to_value(self)?,
        });
        ctx.controller.bias_pulse(
            self.voltage,
            Duration::from_millis(self.duration_ms),
            self.z_hold,
        )?;
        Ok(ActionOutput::Unit)
    }
}
```

**Design insight**: The struct-per-action approach has a higher line count than the enum
approach for individual actions. This is the correct trade-off because: (1) each action
is independently testable, (2) `#[derive(JsonSchema)]` automatically generates the LLM
tool schema, (3) new actions can be defined in downstream crates, (4) the match
statement in `execute_internal` that currently spans hundreds of lines disappears
entirely. The total codebase size may actually decrease because the pattern-matching
dispatch code is replaced by trait dispatch.

### Built-in action categories

These map directly to the current ~30 Action enum variants:

| Category | Actions |
|---|---|
| Signal | `ReadSignal`, `ReadSignals`, `ReadSignalNames` |
| Bias | `ReadBias`, `SetBias`, `BiasPulse` |
| Position | `ReadPosition`, `SetPosition`, `MoveMotor`, `MoveMotorClosedLoop` |
| Z-Control | `Withdraw`, `AutoApproach`, `SetZSetpoint` |
| Scan | `StartScan`, `StopScan`, `ReadScanStatus` |
| Analysis | `CheckTipState`, `CheckTipStability`, `ReadStableSignal` |
| Composite | `SafeReposition`, `PulseRetract` |
| Data | `Wait`, `Store`, `Retrieve` |

---

## Phase 3: Machine State Tracking

### Goal

Ensure that the machine state is always known. Every action, every procedure, and every
error path must leave the system in a state that is either fully known or explicitly
marked as unknown. No action or runner should ever return with the machine in an
ambiguous, untracked state.

This is the foundation for action-chain safety validation: before executing an action,
the framework checks that the current machine state satisfies the action's preconditions.
If it does not, the framework either refuses to proceed (on real hardware) or logs a
warning (on simulated controllers).

### The state model

```rust
/// A value whose tracking status is explicit.
#[derive(Debug, Clone, PartialEq)]
enum Tracked<T> {
    /// Known value, set by a successful action or verified by a query.
    Known(T),
    /// Was known, but a failed action made it uncertain.
    /// Requires a Query action to re-establish.
    Unknown,
    /// Never set -- procedure has not initialized this field yet.
    Uninitialized,
}

/// Software model of the physical machine state.
///
/// This is a best-effort model that is updated by actions on success and
/// degraded to Unknown on error. It is NOT a live mirror of the hardware --
/// it reflects what the software believes to be true based on the actions
/// it has executed.
#[derive(Debug, Clone)]
struct MachineState {
    tip: Tracked<TipEngagement>,
    bias_v: Tracked<f64>,
    z_setpoint_a: Tracked<f64>,
    z_controller: Tracked<ZControllerStatus>,
    scan: Tracked<ScanActivity>,
    safe_tip_enabled: Tracked<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TipEngagement {
    Approached,
    Withdrawn,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ScanActivity {
    Running,
    Stopped,
}
```

**Design insight**: `Uninitialized` vs `Unknown` matters for lifecycle clarity. At
procedure startup, everything is `Uninitialized` -- the system knows nothing about what
the previous user or procedure left behind. The `prepare()` step runs initialization
and query actions that promote fields to `Known`. During execution, only failed actions
demote `Known` to `Unknown`. This distinction lets the framework give different error
messages: "bias has not been initialized" vs "bias is unknown due to a previous timeout"
-- both block execution, but for different reasons.

Motor position is intentionally excluded. Moving the coarse motor is never a safety
hazard (the tip is withdrawn during motor moves). Scan properties (speed, continuous,
bouncy) are also excluded for now -- they can be added later if needed.

### Action classification

Actions are classified by their side-effect profile, which determines the recovery
policy the framework applies:

```rust
/// How an action interacts with the physical hardware state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionKind {
    /// Pure read -- no physical side effects. Safe to auto-insert
    /// for state recovery when a field is Unknown.
    Query,
    /// Modifies physical hardware state. Must be explicitly placed
    /// by the programmer or workflow. Never auto-inserted.
    Mutate,
}
```

| Kind | Physical side effects | Auto-insert for recovery | Examples |
|---|---|---|---|
| Query | None | Yes | `ReadBias`, `ReadSignal`, `ZControllerStatus`, `ReadScanStatus` |
| Mutate | Yes | Never | `SetBias`, `Withdraw`, `CalibratedApproach`, `BiasPulse`, `ScanControl` |

### Extending the Action trait

The Action trait gains four new methods, all with backwards-compatible defaults so that
existing actions compile without changes:

```rust
pub trait Action: Send + Sync {
    // --- existing ---
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn requires(&self) -> Vec<Capability> { vec![] }
    fn execute(&self, ctx: &mut ActionContext) -> Result<ActionOutput>;

    // --- new ---

    /// Whether this action is a pure read or mutates hardware.
    /// Default: Mutate (conservative -- forces explicit opt-in for Query).
    fn kind(&self) -> ActionKind { ActionKind::Mutate }

    /// What machine state this action needs to proceed.
    /// Default: no requirements (always valid).
    fn expects(&self) -> StateRequirements { StateRequirements::none() }

    /// How this action changes the machine state model on success.
    /// Used for pre-execution chain validation (simulating a chain
    /// without running it) and for the static "what will happen" view.
    /// Default: no state changes.
    fn effects(&self) -> StateEffects { StateEffects::none() }

    /// Update machine state from the actual execution result.
    ///
    /// Called by the framework after a successful execute(). For simple
    /// actions, this applies the same changes as effects(). For actions
    /// with conditional outcomes (e.g., AutoApproach might fail to reach
    /// the surface), this can inspect the ActionOutput to determine the
    /// actual state change.
    ///
    /// For Query actions, this is where the queried value is written into
    /// the MachineState, promoting an Unknown field to Known.
    ///
    /// Default: applies effects().
    fn apply_to_state(&self, output: &ActionOutput, state: &mut MachineState) {
        self.effects().apply(state);
    }
}
```

**Design insight**: `effects()` and `apply_to_state()` serve different purposes.
`effects()` describes what *will* happen and is used for static chain validation before
any hardware call. `apply_to_state()` describes what *did* happen and can inspect the
actual `ActionOutput` to make the state update conditional on the result. For most
actions these are equivalent. For actions like `AutoApproach` where the outcome is
uncertain (it might time out), `apply_to_state()` can check whether the approach
actually succeeded before setting `tip = Approached`.

### Annotating existing actions

```rust
impl Action for ReadBias {
    fn kind(&self) -> ActionKind { ActionKind::Query }
    // expects: none (can always read bias)
    // effects: none (doesn't change hardware)
    fn apply_to_state(&self, output: &ActionOutput, state: &mut MachineState) {
        if let ActionOutput::Value(v) = output {
            state.bias_v = Tracked::Known(*v);
        }
    }
}

impl Action for SetBias {
    fn kind(&self) -> ActionKind { ActionKind::Mutate }
    // expects: none (Nanonis accepts SetBias in any state)
    fn effects(&self) -> StateEffects {
        StateEffects::new().set_bias(self.voltage)
    }
}

impl Action for BiasPulse {
    fn kind(&self) -> ActionKind { ActionKind::Mutate }
    fn expects(&self) -> StateRequirements {
        StateRequirements::new().tip(TipEngagement::Approached)
    }
    // effects: none (bias returns to previous value after pulse)
}

impl Action for Withdraw {
    fn kind(&self) -> ActionKind { ActionKind::Mutate }
    // expects: none (withdrawing when already withdrawn is a no-op)
    fn effects(&self) -> StateEffects {
        StateEffects::new().set_tip(TipEngagement::Withdrawn)
    }
}

impl Action for CalibratedApproach {
    fn kind(&self) -> ActionKind { ActionKind::Mutate }
    // expects: none (handles both approached and withdrawn initial states)
    fn effects(&self) -> StateEffects {
        StateEffects::new()
            .set_tip(TipEngagement::Approached)
            .set_z_controller(ZControllerStatus::On)
    }
}

impl Action for ScanControl {
    fn kind(&self) -> ActionKind { ActionKind::Mutate }
    fn expects(&self) -> StateRequirements {
        match self.action {
            ScanActionParam::Start => {
                StateRequirements::new().tip(TipEngagement::Approached)
            }
            ScanActionParam::Stop => StateRequirements::none(),
        }
    }
    fn effects(&self) -> StateEffects {
        match self.action {
            ScanActionParam::Start => {
                StateEffects::new().set_scan(ScanActivity::Running)
            }
            ScanActionParam::Stop => {
                StateEffects::new().set_scan(ScanActivity::Stopped)
            }
        }
    }
}
```

### Validation policy

The enforcement strictness is a property of the executor, not the actions. Actions
always declare the same preconditions and effects -- the executor decides what to do
when preconditions are not met:

```rust
/// How strictly the executor enforces action preconditions.
enum ValidationPolicy {
    /// Log warnings but execute anyway. For simulation and testing.
    /// Useful for LLM training: the LLM can see what violations it
    /// caused without the run aborting.
    Advisory,
    /// Hard gate with automatic Query-based recovery for Unknown fields.
    /// For real hardware. See recovery logic below.
    Strict,
}
```

### Automatic state recovery (Query actions only)

When the executor encounters an `Unknown` field that an action requires, it can
automatically resolve it by running a side-effect-free Query action:

```
for each unmet requirement:
    if field is Unknown AND a Query action exists that resolves it:
        → auto-execute the Query action
        → update MachineState from the query result
        → re-check the requirement with the now-Known value
    if field is still unmet (wrong value, Uninitialized, or no recovery action):
        → hard error, refuse to proceed
```

The key safety invariant: **automatic recovery never changes the physical state of the
machine.** It only queries hardware to update the software model. This means auto-recovery
is always safe -- the worst case is a redundant read that takes a few milliseconds.

Mutate actions are never auto-inserted. If the framework determines that `tip = Withdrawn`
but the next action needs `tip = Approached`, it does not auto-insert an approach. That
would be an implicit state change with parameters the framework cannot know (which bias?
which timeout? center freq shift?). The caller must explicitly place the approach action.

### Error state degradation

When an action fails, the framework degrades the fields that action was supposed to
modify to `Unknown`:

```rust
fn execute_tracked(
    action: &dyn Action,
    ctx: &mut ActionContext,
    state: &mut MachineState,
) -> Result<ActionOutput> {
    // 1. Check preconditions (with auto-recovery for Query actions)
    resolve_and_check(action, ctx, state)?;

    // 2. Execute
    match action.execute(ctx) {
        Ok(output) => {
            // 3a. Success: update model from actual result
            action.apply_to_state(&output, state);
            Ok(output)
        }
        Err(e) => {
            // 3b. Failure: degrade affected fields to Unknown
            action.effects().degrade(state);
            Err(e)
        }
    }
}
```

`degrade()` sets every field that `effects()` would have modified to `Unknown`. Fields
not mentioned in `effects()` are untouched -- the framework assumes the failed action
did not modify them. This is conservative: a `SetBias` failure leaves `bias_v = Unknown`
but `tip`, `scan`, etc. remain at their previous known values.

### Chain validation (pre-execution)

With `effects()` declared on every action, the framework can validate an entire action
chain before executing it:

```rust
fn validate_chain(
    actions: &[&dyn Action],
    initial_state: &MachineState,
) -> Result<(), Vec<ValidationError>> {
    let mut simulated = initial_state.clone();
    let mut errors = vec![];

    for (i, action) in actions.iter().enumerate() {
        if let Err(e) = action.expects().check(&simulated) {
            errors.push(ValidationError {
                step: i,
                action: action.name().to_string(),
                violation: e,
            });
        }
        // Advance simulated state regardless (to catch later errors too)
        action.effects().apply(&mut simulated);
    }

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}
```

This is critical for LLM-generated workflows: the LLM proposes a chain, the framework
validates it before any hardware call, and returns specific error messages if the chain
is invalid. The LLM can fix the chain and resubmit. No tip is harmed in the process.

### Trust model

The machine state model is a software-side best-effort tracker. It can drift from
hardware reality if an external actor changes the hardware state (e.g., the user
manually adjusts bias in the Nanonis GUI while a procedure is running).

The design assumes the software is the **sole writer** to the hardware during procedure
execution. To enforce this socially, the GUI should display a persistent overlay window
("rusty-tip is controlling the instrument") that discourages manual interaction. True
technical enforcement (headless Nanonis, input blocking) may be investigated later but
is not required for the model to be useful.

At procedure boundaries (startup, after long pauses, after errors), the framework should
run verification queries to re-sync the model with hardware reality.

---

## Phase 4: Event and Observation System

### Goal

Replace the current mix of `log::info!`, `crossbeam_channel<ControllerState>`, and
`Logger<ActionLogEntry>` with a unified event system that all consumers (CLI, GUI, LLM,
data export) can plug into.

### Design

```rust
/// Structured event emitted during execution
#[derive(Debug, Clone, Serialize)]
pub enum Event {
    ActionStarted {
        action: String,
        params: serde_json::Value,
        timestamp: SystemTime,
    },
    ActionCompleted {
        action: String,
        result: ActionOutput,
        duration: Duration,
    },
    ActionFailed {
        action: String,
        error: String,
    },
    StateChanged {
        key: String,
        value: serde_json::Value,
    },
    SignalReading {
        signal: String,
        value: f64,
        timestamp: SystemTime,
    },
    WorkflowProgress {
        step_index: usize,
        step_name: String,
        total_steps: usize,
    },
    WorkflowCompleted {
        outcome: String,
        total_duration: Duration,
    },
    DataCollected {
        channel: String,
        samples: usize,
        duration: Duration,
    },
    Custom {
        kind: String,
        data: serde_json::Value,
    },
}

/// Trait for consuming events
pub trait Observer: Send + Sync {
    fn on_event(&self, event: &Event);
}

/// Broadcasts events to multiple observers
pub struct EventBus {
    observers: Vec<Box<dyn Observer>>,
}

pub trait EventEmitter: Send + Sync {
    fn emit(&self, event: Event);
}
```

**Design insight**: Events are `Clone + Serialize` so they can be: (1) sent to multiple
observers without allocation, (2) written directly to JSONL log files, (3) sent over a
channel to a GUI thread, (4) accumulated in memory for LLM context windows. The
`Custom` variant allows actions to emit domain-specific events without modifying the
enum -- important for user-defined actions.

### Built-in observers

```rust
/// Writes events to a JSONL file
pub struct FileLogger { writer: BufWriter<File> }

/// Sends events over a channel (for GUI)
pub struct ChannelForwarder { sender: crossbeam_channel::Sender<Event> }

/// Accumulates events in memory (for LLM context)
pub struct EventAccumulator { events: Mutex<Vec<Event>>, max_events: usize }

/// Prints human-readable summaries to terminal
pub struct ConsoleLogger { verbosity: Verbosity }

/// Collects signal readings for analysis
pub struct SignalCollector { data: Mutex<HashMap<String, Vec<(SystemTime, f64)>>> }
```

**Design insight**: The `EventAccumulator` is specifically designed for LLM integration.
When an LLM agent is driving the system, it needs to "see" what has happened recently to
make decisions. The accumulator maintains a sliding window of recent events that can be
serialized into the LLM's context. The `max_events` cap prevents unbounded memory
growth during long experiments.

---

## Phase 5: Workflow Engine

### Goal

Replace `TipController`'s hardcoded imperative logic with a declarative workflow
definition and a general-purpose executor.

### Workflow definition

```rust
/// A complete workflow definition -- serializable to/from JSON/TOML
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub description: String,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// A single step in a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Step {
    /// Execute a single action
    Do {
        action: String,
        params: serde_json::Value,
        #[serde(default)]
        label: Option<String>,
    },

    /// Execute steps in order, stop on first error
    Sequence {
        steps: Vec<Step>,
        #[serde(default)]
        label: Option<String>,
    },

    /// Execute steps independently (where hardware permits)
    Parallel {
        steps: Vec<Step>,
    },

    /// Conditional execution
    If {
        condition: Condition,
        then: Box<Step>,
        #[serde(default)]
        otherwise: Option<Box<Step>>,
    },

    /// Repeat until condition is met or max iterations reached
    Loop {
        body: Box<Step>,
        until: Option<Condition>,
        #[serde(default = "default_max_iterations")]
        max_iterations: u32,
        #[serde(default)]
        label: Option<String>,
    },

    /// Store a computed value for later use
    SetVar {
        key: String,
        value: ValueExpr,
    },

    /// Wait for a duration
    Wait {
        duration_ms: u64,
    },
}

/// Conditions for If and Loop steps
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Condition {
    /// Compare a stored value to a threshold
    Compare {
        variable: String,
        operator: CompareOp,
        threshold: f64,
    },
    /// Check if a value is within bounds
    InRange {
        variable: String,
        min: f64,
        max: f64,
    },
    /// Combine conditions
    And { conditions: Vec<Condition> },
    Or { conditions: Vec<Condition> },
    Not { condition: Box<Condition> },
    /// Check cycle/iteration count
    MaxCycles { count: u32 },
    /// Check elapsed time
    Timeout { duration_secs: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompareOp { Lt, Le, Eq, Ge, Gt, Ne }

/// Expressions for computing values
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ValueExpr {
    Literal { value: f64 },
    Variable { key: String },
    LastResult,
    ActionResult { action: String, params: serde_json::Value },
}
```

**Design insight**: The `Step` enum is intentionally simple. It does not try to be a
general-purpose programming language. There are no variables with scoping, no function
definitions, no complex expressions. This is deliberate: workflows should be simple
enough that an LLM can reliably generate them, and simple enough that a human can read a
JSON workflow and understand what it does. If a workflow needs complex logic, it should
be implemented as a custom `Action` in Rust and invoked via `Do`. The workflow engine
handles orchestration; actions handle domain logic.

The `Parallel` step exists for future extensibility (e.g., reading signals on one
connection while controlling motors on another) but in the initial implementation it
would execute sequentially, since the Nanonis controller serializes commands anyway.

### The executor

```rust
pub struct WorkflowExecutor {
    registry: ActionRegistry,
    controller: Box<dyn SpmController>,
    events: EventBus,
    store: DataStore,
    shutdown: ShutdownFlag,
}

impl WorkflowExecutor {
    pub fn new(
        registry: ActionRegistry,
        controller: Box<dyn SpmController>,
    ) -> Self { ... }

    pub fn add_observer(&mut self, observer: Box<dyn Observer>) { ... }

    /// Execute a workflow, returning the final outcome
    pub fn run(&mut self, workflow: &Workflow) -> Result<WorkflowOutcome> { ... }

    /// Execute a single step (recursive)
    fn execute_step(&mut self, step: &Step) -> Result<StepOutcome> {
        match step {
            Step::Do { action, params, .. } => {
                let action = self.registry.create(action, params.clone())?;
                let mut ctx = ActionContext {
                    controller: &mut *self.controller,
                    store: &mut self.store,
                    events: &self.events,
                    shutdown: &self.shutdown,
                };
                let output = action.execute(&mut ctx)?;
                Ok(StepOutcome::Completed(output))
            }
            Step::Sequence { steps, .. } => {
                for step in steps {
                    self.execute_step(step)?;
                }
                Ok(StepOutcome::Completed(ActionOutput::Unit))
            }
            Step::Loop { body, until, max_iterations, .. } => {
                for i in 0..*max_iterations {
                    self.execute_step(body)?;
                    if let Some(condition) = until {
                        if self.evaluate_condition(condition)? {
                            return Ok(StepOutcome::Completed(ActionOutput::Unit));
                        }
                    }
                }
                Err(Error::CycleLimit(*max_iterations))
            }
            Step::If { condition, then, otherwise } => {
                if self.evaluate_condition(condition)? {
                    self.execute_step(then)
                } else if let Some(alt) = otherwise {
                    self.execute_step(alt)
                } else {
                    Ok(StepOutcome::Completed(ActionOutput::Unit))
                }
            }
            // ... Wait, SetVar, Parallel
        }
    }
}
```

**Design insight**: The executor is deliberately single-threaded and recursive. Each
`execute_step` call either completes or returns an error. There is no scheduling, no
task queue, no concurrency. This makes the execution model trivially predictable: if you
read a workflow definition top to bottom, that is exactly the order things will happen.
For instrument control, predictability is more valuable than performance. The recursive
approach also means the Rust call stack mirrors the workflow nesting, giving clear panic
backtraces if something goes wrong.

### Tip-prep as a workflow

The current `TipController::run_inner()` translates to roughly this workflow structure:

```json
{
  "name": "tip_prep",
  "description": "Automated tip preparation with optional stability checking",
  "steps": [
    {
      "type": "Loop",
      "label": "main_loop",
      "max_iterations": 100,
      "until": { "type": "Compare", "variable": "tip_shape", "operator": "Eq", "threshold": 2 },
      "body": {
        "type": "Sequence",
        "steps": [
          { "type": "Do", "action": "bias_pulse", "params": { "voltage": "${pulse_voltage}" } },
          { "type": "Wait", "duration_ms": 500 },
          { "type": "Do", "action": "safe_reposition", "params": {} },
          { "type": "Wait", "duration_ms": 1000 },
          { "type": "Do", "action": "check_tip_state", "params": { "method": "standard_deviation" } },
          {
            "type": "If",
            "condition": { "type": "Compare", "variable": "tip_shape", "operator": "Ge", "threshold": 1 },
            "then": {
              "type": "Sequence",
              "label": "stability_check",
              "steps": [
                { "type": "Do", "action": "check_tip_stability", "params": { "sweep_range": [0.1, 2.0] } },
                {
                  "type": "If",
                  "condition": { "type": "Compare", "variable": "stability_passed", "operator": "Eq", "threshold": 1 },
                  "then": { "type": "SetVar", "key": "tip_shape", "value": { "type": "Literal", "value": 2 } },
                  "otherwise": { "type": "Do", "action": "bias_pulse", "params": { "voltage": 8.0 } }
                }
              ]
            }
          }
        ]
      }
    }
  ]
}
```

**Design insight**: Notice how the workflow references `"${pulse_voltage}"` -- a
variable that needs to be computed dynamically based on the pulse method (Fixed,
Stepping, Linear). The current `update_pulse_voltage()` logic in `TipController`
requires state tracking across iterations. This is where the `SetVar` step and custom
actions come in: a `ComputePulseVoltage` action encapsulates that logic and stores the
result in the `DataStore`. The workflow engine doesn't need to understand pulse voltage
calculation; it just runs the action and uses the stored result.

This pattern -- pushing complex logic into actions and keeping workflows simple -- is
the key architectural principle. The workflow is the skeleton; actions are the muscles.

---

## Phase 6: LLM Integration Layer

### Goal

Enable LLMs to discover, plan, and execute SPM workflows through a standard tool-calling
interface.

### Tool definition export

```rust
impl ActionRegistry {
    /// Export all actions as tool definitions in Anthropic/OpenAI format
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.actions.values().map(|factory| {
            ToolDefinition {
                name: factory.name(),
                description: factory.description(),
                input_schema: factory.json_schema(),
            }
        }).collect()
    }
}
```

**Design insight**: The `schemars` crate derives JSON Schema from Rust structs via
`#[derive(JsonSchema)]`. By requiring all action parameter structs to derive
`JsonSchema`, the tool definitions are always in sync with the actual Rust types. There
is zero glue code, zero manual schema maintenance. When a developer adds a new action
with `#[derive(JsonSchema)]`, it automatically becomes available as an LLM tool.

### System state for LLM context

The `MachineState` from Phase 3 serves double duty: it is both the safety model for
action validation and the state description for LLM context. This avoids maintaining
two parallel state representations:

```rust
impl MachineState {
    /// Generate a natural-language summary for LLM context.
    /// Unknown/Uninitialized fields are reported explicitly so the LLM
    /// knows what it doesn't know.
    pub fn describe(&self) -> String {
        format!(
            "Tip: {}. Bias: {}. Z-setpoint: {}. Z-controller: {}. \
             Scan: {}. Safe-tip: {}.",
            self.tip.describe("tip engagement"),
            self.bias_v.describe_v("bias"),
            self.z_setpoint_a.describe_a("setpoint"),
            self.z_controller.describe("z-controller"),
            self.scan.describe("scan"),
            self.safe_tip_enabled.describe("safe-tip"),
        )
    }

    /// Structured version for programmatic access
    pub fn to_json(&self) -> serde_json::Value { ... }
}
```

### LLM agent loop (conceptual, not part of the library itself)

```rust
// This would live in a binary or integration, not in the library
loop {
    let state = executor.system_state();
    let tools = registry.tool_definitions();
    let recent_events = accumulator.recent(50);

    let llm_response = call_llm(system_prompt, state.describe(), tools, recent_events)?;

    match llm_response {
        ToolCall { name, params } => {
            let result = executor.execute_action(&name, params)?;
            // feed result back to LLM on next iteration
        }
        TextResponse(analysis) => {
            log::info!("LLM analysis: {}", analysis);
        }
        Done => break,
    }
}
```

**Design insight**: The library does not include an LLM client or any AI SDK dependency.
It only provides the interface that an LLM integration would need: tool definitions,
state descriptions, and JSON-based action execution. This keeps the library focused and
avoids coupling to a specific LLM provider. The actual LLM loop lives in the binary
(like `tip-prep`) or in a separate integration crate.

---

## Phase 7: Data Pipeline

### Goal

Replace ad-hoc `ExperimentData` and JSONL logging with a proper data pipeline that
integrates with scientific computing tools.

### Design

```rust
/// A collection of time-aligned signal channels
#[derive(Debug, Clone)]
pub struct DataStream {
    pub channels: Vec<SignalChannel>,
    pub sample_rate: f64,
    pub start_time: SystemTime,
}

#[derive(Debug, Clone)]
pub struct SignalChannel {
    pub name: String,
    pub unit: String,
    pub data: Vec<f64>,
}

impl DataStream {
    /// Basic statistics per channel
    pub fn statistics(&self) -> Vec<ChannelStats> { ... }

    /// Time-window extraction
    pub fn window(&self, start_sample: usize, end_sample: usize) -> DataStream { ... }

    /// Downsample by averaging
    pub fn resample(&self, factor: usize) -> DataStream { ... }

    /// Export to CSV (universal, zero dependencies)
    pub fn to_csv(&self, path: &Path) -> Result<()> { ... }

    /// Export to JSON (for LLM consumption)
    pub fn to_json(&self) -> serde_json::Value { ... }

    /// Export to numpy-compatible binary format (optional feature)
    #[cfg(feature = "npy")]
    pub fn to_npy(&self, path: &Path) -> Result<()> { ... }
}
```

**Design insight**: Start with CSV and JSON export. These are universally readable and
have zero dependencies. Arrow/Parquet export can be added later behind a feature flag --
the `arrow` and `parquet` crates are large and would bloat compile times for users who
don't need them. The `DataStream` struct is designed so that adding new export formats
is purely additive (new methods, no changes to existing code).

The `to_json()` method is specifically designed for LLM consumption. An LLM can receive
a JSON representation of recent signal data and perform analysis (trend detection,
anomaly identification) as part of its decision-making loop.

### Integration with events and actions

Actions that produce data (like `ReadStableSignal`, `CheckTipStability`) return
`ActionOutput::Data(json)` containing the measurements. The event system captures these
as `Event::DataCollected`. A `DataCollector` observer accumulates signal readings into
`DataStream` instances that can be exported at any point.

---

## Phase 8: Procedure Trait and Reimplement tip-prep

### Goal

Introduce the `Procedure` trait as the abstraction for high-level SPM procedures, then
rewrite tip-prep as its first implementation. The Procedure trait captures the lifecycle
pattern that all procedures share: precondition checking, guaranteed cleanup, state
tracking, and shutdown handling.

### The Procedure trait

```rust
/// A high-level SPM procedure with lifecycle guarantees.
///
/// The framework handles the lifecycle:
/// 1. Verify required_initial_state() against current MachineState
/// 2. Call run()
/// 3. Call cleanup() unconditionally (even if run() returned Err)
/// 4. Verify guaranteed_final_state() matches actual state
///
/// Procedure authors write run() and cleanup(). The framework handles
/// the rest.
pub trait Procedure: Send {
    /// Human-readable name for logging and GUI display.
    fn name(&self) -> &str;

    /// What machine state must be true before this procedure starts.
    /// The framework verifies this and refuses to start if unmet.
    /// Fields that are Uninitialized or Unknown trigger an error unless
    /// a Query action can resolve them.
    fn required_initial_state(&self) -> StateRequirements;

    /// What machine state this procedure guarantees on successful exit.
    /// Used for procedure chaining: the next procedure's
    /// required_initial_state() can rely on this.
    fn guaranteed_final_state(&self) -> StateEffects;

    /// The procedure's main logic.
    ///
    /// All action execution should go through ctx.execute(), which handles
    /// state tracking, validation, and event emission automatically.
    fn run(&mut self, ctx: &mut ProcedureContext) -> Result<Outcome>;

    /// Cleanup that runs unconditionally, even if run() failed or was
    /// interrupted by shutdown.
    ///
    /// Should bring the machine to a safe, known state. At minimum:
    /// withdraw the tip if it was approached during the procedure.
    ///
    /// Errors in cleanup are logged but do not replace the original error.
    fn cleanup(&self, ctx: &mut ProcedureContext);
}
```

### ProcedureContext

Extends ActionContext with MachineState and provides the `execute()` method that
performs automatic state tracking:

```rust
pub struct ProcedureContext<'a> {
    pub controller: &'a mut dyn SpmController,
    pub state: &'a mut MachineState,
    pub store: &'a mut DataStore,
    pub events: &'a dyn EventEmitter,
    pub shutdown: &'a ShutdownFlag,
    pub policy: ValidationPolicy,
}

impl<'a> ProcedureContext<'a> {
    /// Execute an action with full state tracking.
    ///
    /// 1. Check expects() against current MachineState
    /// 2. Auto-recover Unknown fields via Query actions (Strict mode)
    /// 3. Execute the action
    /// 4. On success: call apply_to_state()
    /// 5. On failure: degrade affected fields to Unknown
    pub fn execute(&mut self, action: &dyn Action) -> Result<ActionOutput> {
        execute_tracked(action, self)
    }
}
```

### Procedure runner (framework-side)

```rust
pub fn run_procedure(
    procedure: &mut dyn Procedure,
    controller: Box<dyn SpmController>,
    events: &EventBus,
    shutdown: &ShutdownFlag,
    policy: ValidationPolicy,
) -> Result<Outcome> {
    let mut state = MachineState::uninitialized();
    let mut store = DataStore::new();

    // Initialize controller
    controller.prepare()?;

    // Build context
    let mut ctx = ProcedureContext {
        controller: &mut *controller,
        state: &mut state,
        store: &mut store,
        events,
        shutdown,
        policy,
    };

    // Verify initial state (will auto-query to resolve Uninitialized fields)
    resolve_and_check_requirements(
        &procedure.required_initial_state(),
        &mut ctx,
    )?;

    // Run procedure, then always cleanup
    let result = procedure.run(&mut ctx);
    procedure.cleanup(&mut ctx);
    controller.teardown();

    result
}
```

### Tip-prep as a Procedure

The current `run_tip_prep()` free function becomes `struct TipPrep` implementing
`Procedure`:

```rust
pub struct TipPrep {
    config: AppConfig,
    freq_shift_index: u32,
}

impl Procedure for TipPrep {
    fn name(&self) -> &str { "tip_prep" }

    fn required_initial_state(&self) -> StateRequirements {
        // Tip-prep starts from scratch -- it initializes everything itself.
        // No requirements on entry state.
        StateRequirements::none()
    }

    fn guaranteed_final_state(&self) -> StateEffects {
        // On successful completion: tip is withdrawn, safe-tip enabled
        StateEffects::new()
            .set_tip(TipEngagement::Withdrawn)
            .set_safe_tip(true)
    }

    fn run(&mut self, ctx: &mut ProcedureContext) -> Result<Outcome> {
        // SetBias, SetZSetpoint, CalibratedApproach, pulse loop, etc.
        // All via ctx.execute(&action) -- state tracked automatically
        // ...
    }

    fn cleanup(&self, ctx: &mut ProcedureContext) {
        let _ = ctx.execute(&Withdraw::default());
    }
}
```

**Design insight**: `TipPrep` requires no initial state because it begins by setting bias,
setpoint, and approaching -- it establishes its own known state. A future
`SpectroscopyCampaign` procedure might require `tip = Approached` and
`bias_v = Known(_)`, relying on a previous procedure having set those up. The
`guaranteed_final_state()` contract is what enables safe procedure chaining.

### What changes from current implementation

- `TipController` is replaced by `TipPrep` implementing `Procedure`
- `ActionDriver` is replaced by `ProcedureContext` + `NanonisController`
- The `Action` enum is replaced by individual action structs in the registry
- `Logger<ActionLogEntry>` is replaced by a `FileLogger` observer
- The GUI's `crossbeam_channel<ControllerState>` is replaced by a `ChannelForwarder`
  observer
- Manual state management in the runner (save/restore scan props, bias, etc.) is
  replaced by the framework's `MachineState` tracking

### What stays

- `AppConfig` and the config loading system (minor adaptations)
- `SignalRegistry` (moved into `NanonisController`)
- `BufferedTCPReader` (moved into `NanonisController`)
- CLI argument parsing
- The GUI structure (adapted to consume `Event` instead of `ControllerState`)

### Binary structure

```rust
fn main() -> Result<()> {
    // 1. Load config (same as today)
    let config = AppConfig::load()?;

    // 2. Create hardware controller
    let controller = NanonisController::new(&config)?;

    // 3. Create procedure
    let mut procedure = TipPrep::new(config.clone(), freq_shift_index);

    // 4. Set up observers
    let events = EventBus::new();
    events.add_observer(Box::new(ConsoleLogger::new(config.verbosity)));
    events.add_observer(Box::new(FileLogger::new(&config.log_path)?));

    // 5. Run with strict validation on real hardware
    let outcome = run_procedure(
        &mut procedure,
        Box::new(controller),
        &events,
        &ShutdownFlag::new(),
        ValidationPolicy::Strict,
    )?;
    println!("Outcome: {:?}", outcome);
    Ok(())
}
```

---

## Design Decisions

### Why synchronous, not async

- The Nanonis controller serializes commands internally across all 4 ports. Sending
  commands concurrently provides no throughput benefit.
- `nanonis-rs` is fully synchronous with blocking `TcpStream`. Going async would
  require rewriting the entire protocol layer.
- The TCP logger data stream is the only genuinely concurrent I/O, and it is already
  handled correctly with a background `std::thread`.
- Async adds complexity (colored functions, runtime dependency, harder debugging) with
  no measurable benefit for this workload.
- Hardware control benefits from predictable, sequential execution. An async scheduler
  introduces non-deterministic task ordering that could cause subtle timing issues.
- If async is ever needed (e.g., for a web API frontend), it can be added as a thin
  async wrapper around the synchronous core using `tokio::task::spawn_blocking`.

### Why trait objects over generics for SpmController

- Actions need to be stored in heterogeneous collections (`Vec<Box<dyn Action>>`)
- The `ActionContext` struct would need a type parameter `<C: SpmController>` that
  would propagate through every type in the system
- The vtable overhead (~5ns per call) is irrelevant when each call does TCP I/O (~1ms)
- Trait objects enable runtime polymorphism: switching between real hardware and
  simulation without recompilation

### Why serde_json::Value for inter-action data

- Actions may produce different types of data (f64, Vec<f64>, complex structs)
- Using `Any` + downcasting loses serializability (can't log, can't send to LLM)
- Using a custom enum would require modifying the enum for every new data type
- `serde_json::Value` is the universal interchange format: serializable, inspectable,
  and directly consumable by LLMs
- The performance cost (serialization/deserialization) is negligible vs TCP latency

### Why trust-the-model for machine state

The MachineState is a software-side model that is not continuously synchronized with
hardware. This is a deliberate choice:

- **Querying hardware for every check would be slow.** Each TCP round-trip is ~1ms.
  Checking 6 fields before every action adds 6ms of overhead per action. In a tight
  pulse loop doing hundreds of cycles, this adds up.
- **The software is the sole writer.** During procedure execution, nothing else should be
  touching the hardware. The overlay window / headless approach enforces this socially.
- **Query-on-Unknown provides the safety net.** When something goes wrong (timeout,
  error), the affected field drops to Unknown. The next action that needs that field
  triggers an automatic query. This is verify-on-demand exactly where it matters: at
  error recovery boundaries, not on every single call.
- **Advisory mode enables safe exploration.** Simulated controllers use Advisory policy
  where violations are logged but not blocked. This lets LLMs learn from mistakes
  without risk. The same action annotations work in both modes -- only the enforcement
  changes.

The alternative -- continuous hardware polling -- would be correct but wasteful. The
state model is a cache with explicit invalidation (on error). Caches that know when
they're stale are safe caches.

### Why a simple workflow language instead of a full scripting engine

- LLMs generate simple JSON structures more reliably than complex programs
- Humans can read a workflow definition and understand it without learning a language
- The Condition/Step model covers 95% of SPM workflows
- Complex logic belongs in actions (Rust code), not in the workflow definition
- Adding a scripting language (Lua, Rhai) would add significant complexity and a large
  dependency; it can be considered later if the Step model proves insufficient

### Why JSON Schema for action parameters

- Both Anthropic and OpenAI tool-calling APIs use JSON Schema
- The `schemars` crate auto-generates schemas from `#[derive(JsonSchema)]`
- Zero manual maintenance: schema always matches the Rust type
- Enables: LLM tool discovery, dynamic GUI generation, config validation
- Also useful for non-LLM scenarios: generating documentation, validating TOML configs

---

## Migration Strategy

The rewrite is designed to be incremental. At no point should the existing `tip-prep`
binary stop working.

### Step 1: Fix nanonis-rs (Phase 0)
- Fix bugs, clean up inconsistencies
- No changes to rusty-tip needed
- nanonis-rs gets a patch release

### Step 2: Add SpmController trait (Phase 1)
- Define the trait in rusty-tip
- Implement `NanonisController` wrapping the existing `NanonisClient` usage
- The existing `ActionDriver` still works; `NanonisController` exists alongside it

### Step 3: Build the Action trait + Registry (Phase 2)
- Define the `Action` trait and `ActionContext`
- Convert actions one at a time from enum variants to trait structs
- Both systems coexist; nothing breaks

### Step 4: Add Machine State Tracking (Phase 3)
- Define `MachineState`, `Tracked<T>`, `ActionKind`, `StateRequirements`, `StateEffects`
- Add `kind()`, `expects()`, `effects()`, `apply_to_state()` to Action trait with defaults
- Annotate safety-critical actions (Withdraw, Approach, SetBias, BiasPulse, ScanControl)
- Implement `execute_tracked()` with auto-recovery logic
- Add `ValidationPolicy` (Advisory/Strict)
- This is purely additive -- existing code continues to work with the default annotations

### Step 5: Add the Event system (Phase 4)
- Define `Event`, `Observer`, `EventBus`
- Wire into `ActionContext`
- Add observers for console, file, channel
- The existing `Logger` and `ControllerState` channel still work

### Step 6: Build the Workflow engine (Phase 5)
- Define `Step`, `Condition`, `Workflow`, `WorkflowExecutor`
- Test with simple workflows against `MockController`
- This is pure new code; nothing existing changes

### Step 7: Port tip-prep to Procedure trait (Phase 8)
- Define the `Procedure` trait and `ProcedureContext`
- Implement `TipPrep` as the first Procedure
- Replace `TipController::run()` and `run_tip_prep()` with `run_procedure()`
- This is the switchover point -- the old implementations are retired

### Step 8: Add LLM integration (Phase 6) and Data pipeline (Phase 7)
- These are additive features on top of the working system
- Can be done in any order, at any pace
- No existing functionality is affected

---

## Estimated Complexity per Phase

| Phase | New code | Existing code changed | Risk |
|---|---|---|---|
| 0: Fix nanonis-rs | ~100 lines | ~300 lines modified | Low -- bug fixes |
| 1: Hardware trait | ~400 lines | ~50 lines | Low -- additive |
| 2: Action system | ~1500 lines | ~200 lines | Medium -- core redesign |
| 3: Machine state | ~600 lines | ~200 lines (action annotations) | Medium -- cross-cutting |
| 4: Event system | ~500 lines | ~100 lines | Low -- additive |
| 5: Workflow engine | ~800 lines | None | Medium -- new logic |
| 6: LLM integration | ~300 lines | None | Low -- additive |
| 7: Data pipeline | ~400 lines | ~100 lines | Low -- additive |
| 8: Procedure + tip-prep | ~400 lines | ~500 lines removed | Medium -- integration |

Phases 0 through 5 are required to reimplement `tip-prep`. Phases 6-7 are
enhancements that can come later. The total new code for the critical path is
approximately 3800 lines of Rust, offset by removing ~500 lines of existing code
(`TipController`, the `Action` enum match arms, the old logger integration).

Phase 3 (Machine State) is the new addition that enables action-chain safety validation.
It adds ~600 lines of infrastructure (MachineState, Tracked, validation logic, recovery
logic) plus ~200 lines of annotations spread across existing actions. The upfront cost
is moderate; the payoff is that every future action and procedure gets safety validation
for free.
