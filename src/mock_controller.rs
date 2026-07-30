//! In-memory [`SpmController`] for exercising the tip-prep routine without a
//! real Nanonis system.
//!
//! The mock lets you drive the full [`run_tip_prep`](crate::tip_prep::run_tip_prep)
//! algorithm — and the edge cases that are dangerous or impossible to provoke
//! on real hardware — entirely in software:
//!
//! * **Tip model** — a closure that decides what `freq_shift` the (virtual)
//!   tip currently exhibits. Because the routine's branching is driven almost
//!   entirely by this one signal, the model is what selects the outcome. See
//!   [`models`] for ready-made models and [`MockControllerBuilder::freq_shift`]
//!   to supply your own.
//! * **Fault injection** — make any controller method fail on its Nth call (or
//!   every call) with a chosen [`FaultKind`], including a connection drop. This
//!   is how you test that the routine cleans up (withdraws, tears down) when an
//!   I/O error strikes mid-run.
//! * **Observations** — every method call, plus running counters (pulses,
//!   approaches, withdraws, last bias, …), are recorded behind a shared handle
//!   you can read *after* the routine finishes (it consumes the controller).
//!
//! ```no_run
//! use rusty_tip::mock_controller::{MockController, models};
//! use rusty_tip::event::EventBus;
//! use rusty_tip::workflow::ShutdownFlag;
//! use rusty_tip::config::AppConfig;
//! use rusty_tip::tip_prep::run_tip_prep;
//!
//! let freq_shift_index = 0;
//! // Tip is blunt (+40 Hz) until 3 pulses land, then sharp (-1 Hz).
//! let mock = MockController::builder()
//!     .freq_shift_index(freq_shift_index)
//!     .freq_shift(models::sharpens_after(3, 40.0, -1.0))
//!     .build();
//! let obs = mock.observations(); // clone the handle BEFORE moving the mock
//!
//! let outcome = run_tip_prep(
//!     Box::new(mock),
//!     &EventBus::new(),
//!     &ShutdownFlag::new(),
//!     &AppConfig::default(),
//!     freq_shift_index,
//! );
//!
//! println!("pulses fired: {}", obs.lock().pulses.len());
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use nanonis_rs::Position;
use nanonis_rs::motor::{
    MotorDirection, MotorDisplacement, MovementMode, Position3D,
};
use nanonis_rs::oscilloscope::OsciData;
use nanonis_rs::scan::{
    AutopasteMode, AutosaveMode, ScanAction, ScanConfig, ScanDirection,
    ScanProps, ScanPropsBuilder,
};
use nanonis_rs::tcplog::TCPLogStatus;
use nanonis_rs::tip_recovery::TipShaperConfig;

use crate::spm_controller::{
    AcquisitionMode, Capability, DataStreamStatus, Result, SpmController,
    TriggerSetup, ZControllerStatus, ZHomeMode,
};
use crate::spm_error::SpmError;

/// A scriptable freq-shift model: given the current [`MockObservations`],
/// return the frequency shift (Hz) the virtual tip exhibits on this read.
///
/// `FnMut` (not `Fn`) so models may carry their own evolving state.
pub type FreqShiftModel = Box<dyn FnMut(&MockObservations) -> f64 + Send>;

/// The kind of error a [scheduled fault](MockControllerBuilder::fail_on_call)
/// produces when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// Network/IO failure (`SpmError::Io`). Considered a connection error.
    Io,
    /// Operation timed out (`SpmError::Timeout`). Considered a connection error.
    Timeout,
    /// Malformed response / type mismatch (`SpmError::Protocol`).
    Protocol,
    /// Hardware/server-reported error with the given code (`SpmError::Hardware`).
    Hardware(i32),
    /// Like [`Io`](FaultKind::Io), but also flips the mock to "disconnected"
    /// so [`is_connected`](SpmController::is_connected) returns `false` until
    /// [`reconnect`](SpmController::reconnect) is called.
    Disconnect,
}

impl FaultKind {
    fn to_error(self) -> SpmError {
        match self {
            FaultKind::Io | FaultKind::Disconnect => SpmError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "mock: injected I/O fault",
                ),
                context: "mock controller".into(),
            },
            FaultKind::Timeout => {
                SpmError::Timeout("mock: injected timeout".into())
            }
            FaultKind::Protocol => {
                SpmError::Protocol("mock: injected protocol error".into())
            }
            FaultKind::Hardware(code) => SpmError::Hardware {
                code,
                message: "mock: injected hardware error".into(),
            },
        }
    }
}

/// Everything observable about a [`MockController`] run, recorded live behind a
/// shared handle so it can be inspected after the routine consumes the mock.
#[derive(Debug, Clone)]
pub struct MockObservations {
    /// Ordered log of every controller method invoked, by name. Use this to
    /// assert on *sequence* — e.g. that `withdraw` ran during cleanup after a
    /// mid-run fault.
    pub calls: Vec<&'static str>,
    /// Per-method invocation counts (drives fault scheduling too).
    pub call_counts: HashMap<&'static str, usize>,
    /// Most recent voltage passed to `set_bias`.
    pub bias: f64,
    /// Every voltage passed to `bias_pulse`, in order. `len()` is the pulse count.
    pub pulses: Vec<f64>,
    /// Most recent z-controller setpoint.
    pub z_setpoint: f64,
    /// Current safe-tip enable state.
    pub safe_tip_enabled: bool,
    /// Whether a scan is currently running.
    pub scan_running: bool,
    /// `prepare()` has been called.
    pub prepared: bool,
    /// `teardown()` has been called.
    pub torn_down: bool,
    /// Number of `auto_approach` calls.
    pub approach_count: usize,
    /// Number of `withdraw` calls.
    pub withdraw_count: usize,
    /// Number of coarse-motor moves (`move_motor` + `move_motor_3d`).
    pub motor_moves: usize,
    /// Number of freq-shift reads served by the tip model.
    pub freq_reads: usize,
    /// Every value the tip model returned, in order. `len()` equals
    /// [`freq_reads`](Self::freq_reads); useful for asserting on (or plotting)
    /// the trajectory a model produced.
    pub freq_values: Vec<f64>,
    /// Connection health; flipped to `false` by a [`FaultKind::Disconnect`].
    pub connected: bool,
}

impl Default for MockObservations {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            call_counts: HashMap::new(),
            bias: 0.0,
            pulses: Vec::new(),
            z_setpoint: 0.0,
            safe_tip_enabled: false,
            scan_running: false,
            prepared: false,
            torn_down: false,
            approach_count: 0,
            withdraw_count: 0,
            motor_moves: 0,
            freq_reads: 0,
            freq_values: Vec::new(),
            connected: true,
        }
    }
}

impl MockObservations {
    /// How many times `method` was called.
    pub fn count(&self, method: &str) -> usize {
        self.call_counts.get(method).copied().unwrap_or(0)
    }

    /// `true` if `method` was ever called.
    pub fn called(&self, method: &str) -> bool {
        self.count(method) > 0
    }

    /// Index of the first call to `method` in [`calls`](Self::calls), if any.
    pub fn first_index(&self, method: &str) -> Option<usize> {
        self.calls.iter().position(|&m| m == method)
    }

    /// Index of the last call to `method` in [`calls`](Self::calls), if any.
    pub fn last_index(&self, method: &str) -> Option<usize> {
        self.calls.iter().rposition(|&m| m == method)
    }
}

/// Small deterministic PRNG (xorshift64\*) so mock runs are reproducible —
/// a figure generated from a mock run can be regenerated exactly.
///
/// Deliberately not `rand`: the crate isn't a dependency, and a fixed-seed
/// generator is the property we actually want here.
#[derive(Debug, Clone, Copy)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Guard the all-zero state, which xorshift can never escape.
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, 1)`.
    fn uniform(&mut self) -> f64 {
        // Top 53 bits land exactly in an f64 mantissa, so no rounding bias.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal, via Box-Muller.
    fn normal(&mut self) -> f64 {
        let u1 = self.uniform().max(f64::MIN_POSITIVE);
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

/// A fault scheduled to fire on a specific call number of a method.
#[derive(Debug, Clone, Copy)]
struct ScheduledFault {
    /// 1-based call ordinal at which to fire.
    on_call: usize,
    kind: FaultKind,
}

/// An in-memory [`SpmController`] for tests and dry-runs. Build via
/// [`MockController::builder`].
pub struct MockController {
    obs: Arc<Mutex<MockObservations>>,
    freq_shift_index: u32,
    model: FreqShiftModel,
    /// Value returned for any non-freq-shift signal index.
    default_signal: f64,
    /// Within-batch scatter (Hz std dev) added by `read_signal_samples`. `0.0`
    /// keeps the legacy constant-batch behavior.
    sample_noise_hz: f64,
    /// PRNG state for [`Self::sample_noise_hz`].
    noise_rng: Rng,
    /// Faults that fire on a specific call ordinal, keyed by method name.
    faults_once: HashMap<&'static str, Vec<ScheduledFault>>,
    /// Faults that fire on *every* call, keyed by method name.
    faults_always: HashMap<&'static str, FaultKind>,
    capabilities: HashSet<Capability>,
    position: Position,
    scan_config: ScanConfig,
}

impl MockController {
    /// Start building a mock controller.
    pub fn builder() -> MockControllerBuilder {
        MockControllerBuilder::new()
    }

    /// A clone of the shared observation handle. Call this *before* moving the
    /// mock into [`run_tip_prep`](crate::tip_prep::run_tip_prep) — the routine
    /// consumes the controller, so this handle is the only way to read state
    /// afterward.
    pub fn observations(&self) -> Arc<Mutex<MockObservations>> {
        Arc::clone(&self.obs)
    }

    /// Record a method call, bump its counter, and fire any scheduled fault.
    ///
    /// Every trait method funnels through here first, so the call log reflects
    /// the *attempt* even when a fault makes the call fail.
    fn enter(&mut self, method: &'static str) -> Result<()> {
        let n = {
            let mut obs = self.obs.lock();
            obs.calls.push(method);
            let c = obs.call_counts.entry(method).or_insert(0);
            *c += 1;
            *c
        };

        if let Some(kind) = self.faults_always.get(method).copied() {
            return Err(self.fire(kind));
        }
        let once = self.faults_once.get(method).and_then(|faults| {
            faults.iter().find(|f| f.on_call == n).map(|f| f.kind)
        });
        if let Some(kind) = once {
            return Err(self.fire(kind));
        }
        Ok(())
    }

    /// Turn a [`FaultKind`] into an error, applying its side effects.
    fn fire(&mut self, kind: FaultKind) -> SpmError {
        if matches!(kind, FaultKind::Disconnect) {
            self.obs.lock().connected = false;
        }
        kind.to_error()
    }

    /// Evaluate the tip model for the freq-shift channel; constant otherwise.
    fn signal_value(&mut self, index: u32) -> f64 {
        if index != self.freq_shift_index {
            return self.default_signal;
        }
        // Disjoint field borrow: the guard borrows `self.obs` while the model
        // call borrows `self.model` mutably — different fields, so this is OK.
        let value = {
            let obs = self.obs.lock();
            (self.model)(&obs)
        };
        let mut obs = self.obs.lock();
        obs.freq_reads += 1;
        obs.freq_values.push(value);
        drop(obs);
        value
    }
}

impl SpmController for MockController {
    fn capabilities(&self) -> HashSet<Capability> {
        self.capabilities.clone()
    }

    // -- Lifecycle --

    fn prepare(&mut self) -> Result<()> {
        self.enter("prepare")?;
        self.obs.lock().prepared = true;
        Ok(())
    }

    fn teardown(&mut self) {
        // teardown must not short-circuit; a fault here is recorded but ignored.
        let _ = self.enter("teardown");
        self.obs.lock().torn_down = true;
    }

    fn is_connected(&self) -> bool {
        self.obs.lock().connected
    }

    fn reconnect(&mut self) -> Result<()> {
        self.enter("reconnect")?;
        self.obs.lock().connected = true;
        Ok(())
    }

    // -- Signals --

    fn read_signal(&mut self, index: u32, _wait: bool) -> Result<f64> {
        self.enter("read_signal")?;
        Ok(self.signal_value(index))
    }

    fn read_signals(
        &mut self,
        indices: &[u32],
        _wait: bool,
    ) -> Result<Vec<f64>> {
        self.enter("read_signals")?;
        Ok(indices.iter().map(|&i| self.signal_value(i)).collect())
    }

    fn signal_names(&mut self) -> Result<Vec<String>> {
        self.enter("signal_names")?;
        Ok(vec![
            "Current (A)".into(),
            "Bias (V)".into(),
            "freq shift".into(),
            "Z (m)".into(),
        ])
    }

    fn read_signal_samples(
        &mut self,
        index: u32,
        num_samples: usize,
    ) -> Result<Vec<f64>> {
        self.enter("read_signal_samples")?;
        if num_samples == 0 {
            return Err(SpmError::Protocol(
                "read_signal_samples: num_samples must be > 0".into(),
            ));
        }
        // One model call per *batch*, not per sample: the tip model owns the
        // slow behavior (pulse response, drift), and scatter is layered on top.
        // Keeping drift out of the within-batch samples matters — it would show
        // up as a regression slope and trip `ReadStableSignal`'s `max_slope`
        // (default 0.01 Hz/sample), stalling the routine in retry backoff.
        let value = self.signal_value(index);

        if self.sample_noise_hz > 0.0 && index == self.freq_shift_index {
            let sigma = self.sample_noise_hz;
            let mut rng = self.noise_rng;
            let samples = (0..num_samples)
                .map(|_| value + rng.normal() * sigma)
                .collect();
            self.noise_rng = rng;
            return Ok(samples);
        }

        // Default: constant samples => std_dev = 0, slope = 0 => always
        // "stable", so ReadStableSignal returns on the first attempt with no
        // backoff sleeps. Keeps the test suite fast and deterministic.
        Ok(vec![value; num_samples])
    }

    // -- Bias --

    fn get_bias(&mut self) -> Result<f64> {
        self.enter("get_bias")?;
        Ok(self.obs.lock().bias)
    }

    fn set_bias(&mut self, voltage: f64) -> Result<()> {
        self.enter("set_bias")?;
        self.obs.lock().bias = voltage;
        Ok(())
    }

    fn bias_pulse(
        &mut self,
        voltage: f64,
        _width: Duration,
        _z_hold: bool,
        _absolute: bool,
    ) -> Result<()> {
        self.enter("bias_pulse")?;
        self.obs.lock().pulses.push(voltage);
        Ok(())
    }

    // -- Z-Controller --

    fn withdraw(&mut self, _wait: bool, _timeout: Duration) -> Result<()> {
        self.enter("withdraw")?;
        let mut obs = self.obs.lock();
        obs.withdraw_count += 1;
        obs.scan_running = false;
        Ok(())
    }

    fn auto_approach(&mut self, _wait: bool, _timeout: Duration) -> Result<()> {
        self.enter("auto_approach")?;
        self.obs.lock().approach_count += 1;
        Ok(())
    }

    fn set_z_setpoint(&mut self, setpoint: f64) -> Result<()> {
        self.enter("set_z_setpoint")?;
        self.obs.lock().z_setpoint = setpoint;
        Ok(())
    }

    fn set_z_home(&mut self, _mode: ZHomeMode, _position: f64) -> Result<()> {
        self.enter("set_z_home")?;
        Ok(())
    }

    fn go_z_home(&mut self) -> Result<()> {
        self.enter("go_z_home")?;
        Ok(())
    }

    fn z_controller_status(&mut self) -> Result<ZControllerStatus> {
        self.enter("z_controller_status")?;
        Ok(ZControllerStatus::On)
    }

    // -- Piezo Positioning --

    fn get_position(&mut self, _wait: bool) -> Result<Position> {
        self.enter("get_position")?;
        Ok(self.position)
    }

    fn set_position(&mut self, pos: Position, _wait: bool) -> Result<()> {
        self.enter("set_position")?;
        self.position = pos;
        Ok(())
    }

    // -- Motor --

    fn move_motor(
        &mut self,
        _direction: MotorDirection,
        _steps: u16,
        _wait: bool,
    ) -> Result<()> {
        self.enter("move_motor")?;
        self.obs.lock().motor_moves += 1;
        Ok(())
    }

    fn move_motor_3d(
        &mut self,
        _displacement: MotorDisplacement,
        _wait: bool,
    ) -> Result<()> {
        self.enter("move_motor_3d")?;
        self.obs.lock().motor_moves += 1;
        Ok(())
    }

    fn move_motor_closed_loop(
        &mut self,
        _target: Position3D,
        _mode: MovementMode,
    ) -> Result<()> {
        self.enter("move_motor_closed_loop")?;
        self.obs.lock().motor_moves += 1;
        Ok(())
    }

    fn stop_motor(&mut self) -> Result<()> {
        self.enter("stop_motor")?;
        Ok(())
    }

    // -- Scanning --

    fn scan_action(
        &mut self,
        action: ScanAction,
        _direction: ScanDirection,
    ) -> Result<()> {
        self.enter("scan_action")?;
        let mut obs = self.obs.lock();
        match action {
            ScanAction::Start | ScanAction::Resume => obs.scan_running = true,
            ScanAction::Stop | ScanAction::Pause => obs.scan_running = false,
            // Freeze/Unfreeze/GoToCenter don't change the running flag.
            _ => {}
        }
        Ok(())
    }

    fn scan_status(&mut self) -> Result<bool> {
        self.enter("scan_status")?;
        Ok(self.obs.lock().scan_running)
    }

    fn scan_props_get(&mut self) -> Result<ScanProps> {
        self.enter("scan_props_get")?;
        Ok(mock_scan_props())
    }

    fn scan_props_set(&mut self, _props: ScanPropsBuilder) -> Result<()> {
        self.enter("scan_props_set")?;
        Ok(())
    }

    fn scan_speed_get(&mut self) -> Result<ScanConfig> {
        self.enter("scan_speed_get")?;
        Ok(self.scan_config)
    }

    fn scan_speed_set(&mut self, config: ScanConfig) -> Result<()> {
        self.enter("scan_speed_set")?;
        self.scan_config = config;
        Ok(())
    }

    fn scan_frame_data_grab(
        &mut self,
        _channel_index: u32,
        forward: bool,
    ) -> Result<(String, Vec<Vec<f32>>, bool)> {
        self.enter("scan_frame_data_grab")?;
        // 2x2 flat frame is enough for routines that only check shape.
        Ok(("mock_channel".into(), vec![vec![0.0; 2]; 2], forward))
    }

    // -- Oscilloscope --

    fn osci_read(
        &mut self,
        _channel: i32,
        _trigger: Option<&TriggerSetup>,
        _mode: AcquisitionMode,
    ) -> Result<OsciData> {
        self.enter("osci_read")?;
        Ok(OsciData::new(0.0, 1e-3, 4, vec![0.0; 4]))
    }

    // -- Tip Shaper --

    fn tip_shaper(
        &mut self,
        _config: &TipShaperConfig,
        _wait: bool,
        _timeout: Duration,
    ) -> Result<()> {
        self.enter("tip_shaper")?;
        Ok(())
    }

    // -- PLL --

    fn pll_center_freq_shift(&mut self) -> Result<()> {
        self.enter("pll_center_freq_shift")?;
        Ok(())
    }

    // -- Safe Tip --

    fn safe_tip_configure(
        &mut self,
        _auto_recovery: bool,
        _auto_pause_scan: bool,
        _threshold: f64,
    ) -> Result<()> {
        self.enter("safe_tip_configure")?;
        Ok(())
    }

    fn safe_tip_status(&mut self) -> Result<(bool, bool, f64)> {
        self.enter("safe_tip_status")?;
        Ok((false, true, 1e-9))
    }

    fn safe_tip_set_enabled(&mut self, enabled: bool) -> Result<()> {
        self.enter("safe_tip_set_enabled")?;
        self.obs.lock().safe_tip_enabled = enabled;
        Ok(())
    }

    fn safe_tip_enabled(&mut self) -> Result<bool> {
        self.enter("safe_tip_enabled")?;
        Ok(self.obs.lock().safe_tip_enabled)
    }

    // -- Data Stream --

    fn data_stream_configure(
        &mut self,
        _channels: &[i32],
        _oversampling: i32,
    ) -> Result<()> {
        self.enter("data_stream_configure")?;
        Ok(())
    }

    fn data_stream_start(&mut self) -> Result<()> {
        self.enter("data_stream_start")?;
        Ok(())
    }

    fn data_stream_stop(&mut self) -> Result<()> {
        self.enter("data_stream_stop")?;
        Ok(())
    }

    fn data_stream_status(&mut self) -> Result<DataStreamStatus> {
        self.enter("data_stream_status")?;
        Ok(TCPLogStatus::Running)
    }

    fn clear_data_buffer(&mut self) {
        let _ = self.enter("clear_data_buffer");
    }
}

/// Builder for [`MockController`].
pub struct MockControllerBuilder {
    freq_shift_index: u32,
    model: FreqShiftModel,
    default_signal: f64,
    sample_noise_hz: f64,
    noise_seed: u64,
    faults_once: HashMap<&'static str, Vec<ScheduledFault>>,
    faults_always: HashMap<&'static str, FaultKind>,
    capabilities: HashSet<Capability>,
    start_connected: bool,
}

impl MockControllerBuilder {
    fn new() -> Self {
        Self {
            freq_shift_index: 0,
            // Default: an obstinately blunt tip (+100 Hz, far outside any sane
            // sharp-tip bound) so a forgotten model yields CycleLimit, not a
            // spurious success.
            model: models::always(100.0),
            default_signal: 0.0,
            sample_noise_hz: 0.0,
            noise_seed: 0x5EED_5EED,
            faults_once: HashMap::new(),
            faults_always: HashMap::new(),
            capabilities: all_capabilities(),
            start_connected: true,
        }
    }

    /// Set which signal index the tip model answers for. Must match the
    /// `freq_shift_index` you pass to `run_tip_prep`.
    pub fn freq_shift_index(mut self, index: u32) -> Self {
        self.freq_shift_index = index;
        self
    }

    /// Supply the tip model — the closure that decides the freq-shift returned
    /// on each read. See [`models`] for ready-made ones.
    pub fn freq_shift(mut self, model: FreqShiftModel) -> Self {
        self.model = model;
        self
    }

    /// Value returned for every non-freq-shift signal index (default `0.0`).
    pub fn default_signal(mut self, value: f64) -> Self {
        self.default_signal = value;
        self
    }

    /// Add per-sample scatter (Hz std dev) to `read_signal_samples` batches on
    /// the freq-shift channel. Default `0.0` = constant batches.
    ///
    /// This is *within-batch* noise, distinct from
    /// [`RealisticParams::noise_hz`], which jitters the batch mean read-to-read.
    /// Non-zero scatter is what makes `ReadStableSignal`'s std-dev check do real
    /// work — keep it comfortably under the configured `max_std_dev` or the
    /// routine will spend its time in retry backoff.
    pub fn sample_noise_hz(mut self, sigma: f64) -> Self {
        self.sample_noise_hz = sigma;
        self
    }

    /// Seed for the [`sample_noise_hz`](Self::sample_noise_hz) generator. Fixed
    /// by default, so a mock run is reproducible.
    pub fn noise_seed(mut self, seed: u64) -> Self {
        self.noise_seed = seed;
        self
    }

    /// Schedule `method` to fail on its `nth` call (1-based) with `kind`.
    ///
    /// `method` is the [`SpmController`] method name, e.g. `"auto_approach"`,
    /// `"withdraw"`, `"read_signal_samples"`, `"scan_action"`.
    pub fn fail_on_call(
        mut self,
        method: &'static str,
        nth: usize,
        kind: FaultKind,
    ) -> Self {
        self.faults_once
            .entry(method)
            .or_default()
            .push(ScheduledFault { on_call: nth, kind });
        self
    }

    /// Make `method` fail on *every* call with `kind`.
    pub fn fail_every(mut self, method: &'static str, kind: FaultKind) -> Self {
        self.faults_always.insert(method, kind);
        self
    }

    /// Restrict the capabilities the mock reports (default: all of them).
    pub fn capabilities(mut self, caps: HashSet<Capability>) -> Self {
        self.capabilities = caps;
        self
    }

    /// Start in the disconnected state (`is_connected()` returns `false` until
    /// `reconnect()` is called).
    pub fn start_disconnected(mut self) -> Self {
        self.start_connected = false;
        self
    }

    /// Finish building.
    pub fn build(self) -> MockController {
        let mut obs = MockObservations::default();
        obs.connected = self.start_connected;
        MockController {
            obs: Arc::new(Mutex::new(obs)),
            freq_shift_index: self.freq_shift_index,
            model: self.model,
            default_signal: self.default_signal,
            sample_noise_hz: self.sample_noise_hz,
            noise_rng: Rng::new(self.noise_seed),
            faults_once: self.faults_once,
            faults_always: self.faults_always,
            capabilities: self.capabilities,
            position: Position::new(0.0, 0.0),
            scan_config: mock_scan_config(),
        }
    }
}

/// Ready-made [`FreqShiftModel`]s for common scenarios.
///
/// Need behavior these don't cover? A `freq_shift` model is just
/// `Box::new(move |obs| ...)` — read whatever you need off [`MockObservations`]
/// (pulse count, last bias, read count) and return the Hz value.
pub mod models {
    use super::{FreqShiftModel, MockObservations};

    /// The tip always reads `value` Hz, no matter what.
    ///
    /// Pick `value` inside your `sharp_tip_bounds` to test the "already sharp"
    /// path, or far outside to drive `CycleLimit`.
    pub fn always(value: f64) -> FreqShiftModel {
        Box::new(move |_| value)
    }

    /// Blunt (`blunt` Hz) until `pulses` bias pulses have landed, then sharp
    /// (`sharp` Hz) forever after. The classic "conditioning works" trajectory.
    pub fn sharpens_after(
        pulses: usize,
        blunt: f64,
        sharp: f64,
    ) -> FreqShiftModel {
        Box::new(move |obs: &MockObservations| {
            if obs.pulses.len() >= pulses {
                sharp
            } else {
                blunt
            }
        })
    }

    /// Return a pre-scripted value per freq-shift read: `values[i]` on read `i`,
    /// clamping to the last entry once exhausted.
    ///
    /// Total control over the trajectory — handy for the unstable branch, where
    /// you want the confirmation reads in-bounds but the post-sweep "final"
    /// read to have drifted past the stability threshold.
    pub fn scripted(values: Vec<f64>) -> FreqShiftModel {
        Box::new(move |obs: &MockObservations| {
            if values.is_empty() {
                return 0.0;
            }
            let i = obs.freq_reads.min(values.len() - 1);
            values[i]
        })
    }

    /// Tunables for [`realistic`]. Start from `Default` and override what you
    /// care about:
    ///
    /// ```
    /// use rusty_tip::mock_controller::models::{RealisticParams, realistic};
    ///
    /// let model = realistic(RealisticParams {
    ///     overshoot_chance: 0.3, // a badly behaved tip
    ///     ..Default::default()
    /// });
    /// ```
    #[derive(Debug, Clone, Copy)]
    pub struct RealisticParams {
        /// Frequency shift (Hz) of the unconditioned tip — strongly negative.
        /// Conditioning drives the shift *up* from here toward `sharp`.
        pub blunt: f64,
        /// Asymptote (Hz) a well-conditioned tip approaches, just inside the
        /// sharp window (typically `[-2, 0]`).
        pub sharp: f64,
        /// Read-to-read jitter (Hz std dev) on the batch mean. Distinct from
        /// [`MockControllerBuilder::sample_noise_hz`], which scatters the
        /// samples *within* one batch.
        pub noise_hz: f64,
        /// Std dev of each random-walk drift step, applied once per read.
        pub drift_hz_per_read: f64,
        /// Fraction of the remaining gap to `sharp` that a pulse at
        /// `reference_voltage` closes.
        pub pulse_efficiency: f64,
        /// Pulse voltage at which `pulse_efficiency` applies; higher pulses
        /// move the tip further, lower ones less.
        pub reference_voltage: f64,
        /// Probability that a pulse makes the tip *worse* instead of better —
        /// usually blunting it back down, occasionally overshooting the window
        /// into positive shift (see [`insulating_chance`](Self::insulating_chance)).
        pub overshoot_chance: f64,
        /// Given a bad pulse, the probability it lands the tip on an insulating
        /// patch — the one case where the shift goes *positive*. Real but
        /// uncommon, so this is a small fraction of an already-uncommon event.
        pub insulating_chance: f64,
        /// PRNG seed — fixed, so the trajectory is reproducible.
        pub seed: u64,
    }

    impl Default for RealisticParams {
        /// Tuned so the default run exercises the whole algorithm rather than
        /// just the happy path: the tip climbs from `blunt` into the sharp
        /// window, holds, then loses it to a bad recovery pulse and climbs back
        /// before passing the stability check.
        ///
        /// Change `seed` to draw a different trajectory — `cargo run --example
        /// tip-prep-mock -- realistic <seed>` prints the one a given seed
        /// produces, so a scenario can be judged without a GUI.
        fn default() -> Self {
            Self {
                blunt: -40.0,
                sharp: -1.0,
                noise_hz: 0.08,
                drift_hz_per_read: 0.012,
                pulse_efficiency: 0.45,
                reference_voltage: 4.0,
                overshoot_chance: 0.15,
                insulating_chance: 0.15,
                seed: 2026,
            }
        }
    }

    /// A tip that responds to pulses the way a real one roughly does:
    /// stochastic sharpening that closes a *fraction* of the remaining gap per
    /// pulse (so it converges asymptotically rather than snapping to a value),
    /// occasional over-pulsing that undoes progress, plus measurement noise and
    /// slow thermal drift on every read.
    ///
    /// Follows the physical sign convention: a blunt tip sits at a strongly
    /// negative frequency shift and conditioning drives it *up* toward the sharp
    /// window near `[-2, 0]` Hz. Positive shift means the tip has found an
    /// insulating patch — real, but uncommon, so the model reaches it rarely.
    ///
    /// Unlike [`sharpens_after`] and [`scripted`], the trajectory here is
    /// *emergent* — it depends on how many pulses the routine fires and at what
    /// voltage, which is what makes the resulting trace look like data instead
    /// of a step function. Pair it with
    /// [`MockControllerBuilder::sample_noise_hz`] for within-batch scatter.
    ///
    /// Seeded, so the same params always produce the same run.
    pub fn realistic(params: RealisticParams) -> FreqShiftModel {
        let mut rng = super::Rng::new(params.seed);
        let mut state = params.blunt;
        let mut seen_pulses = 0usize;
        let mut drift = 0.0f64;

        Box::new(move |obs: &MockObservations| {
            // Apply every pulse that landed since the previous read. Reading
            // `obs.pulses` rather than a counter means the model sees the
            // actual voltages the routine chose.
            for &voltage in obs.pulses.iter().skip(seen_pulses) {
                seen_pulses += 1;
                let strength =
                    (voltage.abs() / params.reference_voltage).clamp(0.0, 2.0);

                if rng.uniform() < params.overshoot_chance {
                    // A bad pulse: the apex reorganized the wrong way.
                    state = if rng.uniform() < params.insulating_chance {
                        // Overshot the window entirely and landed on an
                        // insulating patch — the rare positive-shift case.
                        (1.0 + 4.0 * rng.uniform()) * strength.max(0.5)
                    } else {
                        // The common failure: blunted back down, losing a
                        // fraction of the progress made so far.
                        state
                            - (0.25 + 0.45 * rng.uniform())
                                * (state - params.blunt).abs().max(2.0)
                    };
                } else {
                    // Normal conditioning: close a fraction of the gap, scaled
                    // by pulse strength, with spread on the efficiency.
                    let eff = (params.pulse_efficiency
                        * strength
                        * (0.6 + 0.8 * rng.uniform()))
                    .clamp(0.0, 0.95);
                    state += (params.sharp - state) * eff;
                }
            }

            // Random walk, not white noise: consecutive reads stay correlated
            // the way real thermal drift does.
            drift += rng.normal() * params.drift_hz_per_read;
            state + drift + rng.normal() * params.noise_hz
        })
    }
}

/// The full capability set (matches `NanonisController`).
fn all_capabilities() -> HashSet<Capability> {
    HashSet::from([
        Capability::Signals,
        Capability::Bias,
        Capability::ZController,
        Capability::PiezoPosition,
        Capability::Motor,
        Capability::Scanning,
        Capability::Oscilloscope,
        Capability::TipShaper,
        Capability::Pll,
        Capability::DataStream,
        Capability::SafeTip,
    ])
}

fn mock_scan_config() -> ScanConfig {
    ScanConfig {
        forward_linear_speed_m_s: 5e-9,
        backward_linear_speed_m_s: 5e-9,
        forward_time_per_line_s: 1.0,
        backward_time_per_line_s: 1.0,
        keep_parameter_constant: 0,
        speed_ratio: 1.0,
    }
}

fn mock_scan_props() -> ScanProps {
    ScanProps {
        continuous_scan: false,
        bouncy_scan: false,
        autosave: AutosaveMode::Off,
        series_name: String::new(),
        comment: String::new(),
        modules_names: Vec::new(),
        num_params_per_module: Vec::new(),
        parameters: Vec::new(),
        autopaste: AutopasteMode::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_drives_freq_shift_channel_only() {
        let mut mock = MockController::builder()
            .freq_shift_index(7)
            .freq_shift(models::always(-1.5))
            .default_signal(42.0)
            .build();

        assert_eq!(mock.read_signal(7, true).unwrap(), -1.5);
        assert_eq!(mock.read_signal(3, true).unwrap(), 42.0);
    }

    #[test]
    fn sharpens_after_tracks_pulse_count() {
        let mut mock = MockController::builder()
            .freq_shift_index(0)
            .freq_shift(models::sharpens_after(2, 40.0, -1.0))
            .build();

        assert_eq!(mock.read_signal(0, true).unwrap(), 40.0); // 0 pulses
        mock.bias_pulse(5.0, Duration::ZERO, true, true).unwrap();
        assert_eq!(mock.read_signal(0, true).unwrap(), 40.0); // 1 pulse
        mock.bias_pulse(5.0, Duration::ZERO, true, true).unwrap();
        assert_eq!(mock.read_signal(0, true).unwrap(), -1.0); // 2 pulses
    }

    #[test]
    fn scripted_advances_per_read_and_clamps() {
        let mut mock = MockController::builder()
            .freq_shift(models::scripted(vec![10.0, -1.0, -5.0]))
            .build();
        assert_eq!(mock.read_signal(0, true).unwrap(), 10.0);
        assert_eq!(mock.read_signal(0, true).unwrap(), -1.0);
        assert_eq!(mock.read_signal(0, true).unwrap(), -5.0);
        assert_eq!(mock.read_signal(0, true).unwrap(), -5.0); // clamped
    }

    #[test]
    fn fail_on_call_fires_once_at_the_right_ordinal() {
        let mut mock = MockController::builder()
            .fail_on_call("withdraw", 2, FaultKind::Timeout)
            .build();

        assert!(mock.withdraw(true, Duration::ZERO).is_ok()); // 1st
        let err = mock.withdraw(true, Duration::ZERO).unwrap_err(); // 2nd
        assert!(matches!(err, SpmError::Timeout(_)));
        assert!(mock.withdraw(true, Duration::ZERO).is_ok()); // 3rd
    }

    #[test]
    fn read_signal_samples_are_constant_and_stable() {
        let mut mock = MockController::builder()
            .freq_shift(models::always(-1.0))
            .build();
        let samples = mock.read_signal_samples(0, 64).unwrap();
        assert_eq!(samples.len(), 64);
        assert!(samples.iter().all(|&v| v == -1.0));
    }

    #[test]
    fn sample_noise_scatters_within_a_batch() {
        let mut mock = MockController::builder()
            .freq_shift(models::always(-1.0))
            .sample_noise_hz(0.3)
            .build();

        let samples = mock.read_signal_samples(0, 512).unwrap();
        let (mean, std_dev, _) =
            crate::action::signals::compute_stability_metrics(&samples);

        // Scattered, but centered on the model value.
        assert!(std_dev > 0.2 && std_dev < 0.4, "std_dev was {std_dev}");
        assert!((mean - -1.0).abs() < 0.05, "mean was {mean}");
    }

    #[test]
    fn sample_noise_leaves_other_channels_constant() {
        let mut mock = MockController::builder()
            .freq_shift_index(2)
            .default_signal(7.0)
            .sample_noise_hz(0.5)
            .build();

        let samples = mock.read_signal_samples(0, 32).unwrap();
        assert!(samples.iter().all(|&v| v == 7.0));
    }

    #[test]
    fn realistic_is_reproducible_for_a_seed() {
        let params = models::RealisticParams::default();
        let run = || {
            let mut mock = MockController::builder()
                .freq_shift(models::realistic(params))
                .build();
            (0..5)
                .map(|_| {
                    mock.bias_pulse(4.0, Duration::ZERO, true, true).unwrap();
                    mock.read_signal(0, true).unwrap()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn realistic_trends_toward_sharp_as_pulses_land() {
        // Overshoot disabled so the trend is monotone and the assert is not
        // hostage to the seed.
        let mut mock = MockController::builder()
            .freq_shift(models::realistic(models::RealisticParams {
                overshoot_chance: 0.0,
                ..Default::default()
            }))
            .build();

        let first = mock.read_signal(0, true).unwrap();
        for _ in 0..12 {
            mock.bias_pulse(4.0, Duration::ZERO, true, true).unwrap();
        }
        let last = mock.read_signal(0, true).unwrap();

        // Physical convention: blunt is strongly negative, conditioning drives
        // the shift *up* into the sharp window.
        assert!(
            first < -30.0,
            "blunt tip should start strongly negative, got {first}"
        );
        assert!(
            last < 0.0 && last > -3.0,
            "conditioned tip should rise toward sharp, got {last}"
        );
        assert!(last > first, "conditioning should raise the shift");
    }

    #[test]
    fn realistic_stays_negative_without_insulating_events() {
        // With the insulating branch disabled, the shift must never go positive.
        let mut mock = MockController::builder()
            .freq_shift(models::realistic(models::RealisticParams {
                insulating_chance: 0.0,
                overshoot_chance: 0.5, // plenty of bad pulses
                ..Default::default()
            }))
            .build();

        for _ in 0..40 {
            mock.bias_pulse(5.0, Duration::ZERO, true, true).unwrap();
            let v = mock.read_signal(0, true).unwrap();
            assert!(v < 0.5, "shift should stay negative, got {v}");
        }
    }

    #[test]
    fn disconnect_fault_flips_connection_until_reconnect() {
        let mut mock = MockController::builder()
            .fail_on_call("auto_approach", 1, FaultKind::Disconnect)
            .build();

        assert!(mock.is_connected());
        let err = mock.auto_approach(true, Duration::ZERO).unwrap_err();
        assert!(err.is_connection_error());
        assert!(!mock.is_connected());
        mock.reconnect().unwrap();
        assert!(mock.is_connected());
    }

    #[test]
    fn observations_handle_records_calls_and_counters() {
        let mock = MockController::builder().build();
        let obs = mock.observations();
        let mut mock = mock;

        mock.set_bias(-0.5).unwrap();
        mock.bias_pulse(4.0, Duration::ZERO, true, true).unwrap();
        mock.withdraw(true, Duration::ZERO).unwrap();

        let obs = obs.lock();
        assert_eq!(obs.bias, -0.5);
        assert_eq!(obs.pulses, vec![4.0]);
        assert_eq!(obs.withdraw_count, 1);
        assert!(obs.called("set_bias"));
        assert_eq!(obs.count("bias_pulse"), 1);
    }
}
