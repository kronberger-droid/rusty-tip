//! Feedback controllers as parameter sets you can read, edit and write
//! back: the Z-controller and the PLL's amplitude and phase loops.
//!
//! Each kind is one struct of the fields the protocol lets you set, in SI,
//! with `JsonSchema` and unit annotations so a form draws it and a
//! [`ControllerProfile`] is plain TOML. Reading gives a [`ControllerReading`],
//! the parameters plus what cannot be written: whether the loop is on, its
//! status word, and for the Z-controller the list of controllers Nanonis
//! has defined. Which signal a controller reads, and linear or log input,
//! is a definition, not a parameter: it comes from a settings file, which a
//! profile can name and [`ApplyProfile`] loads first.
//!
//! Switching a loop on or off is deliberately not part of applying a
//! profile: [`crate::spm_controller::SpmController::set_controller_enabled`]
//! is its own call, so a profile cannot drop the Z feedback by accident.

use std::fmt;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::{Event, EventEmitter};
use crate::experiment_log::{LogEvent, ToolSchema};
use crate::routine::{Outcome, SettingsLoadedEvent, require};
use crate::session::{Job, JobCx};
use crate::spm_controller::{Capability, SpmController};
use crate::spm_error::SpmError;

/// One controller on the machine.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControllerId {
    /// The Z feedback loop. Nanonis defines several and runs one; which
    /// one is the `active` parameter.
    Z,
    /// The PLL's oscillation amplitude loop, per modulator (1 or 2).
    PllAmplitude { modulator: u8 },
    /// The PLL's phase loop, which tracks the resonance, per modulator.
    PllPhase { modulator: u8 },
}

impl fmt::Display for ControllerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ControllerId::Z => write!(f, "Z-controller"),
            ControllerId::PllAmplitude { modulator } => write!(f, "PLL {modulator} amplitude"),
            ControllerId::PllPhase { modulator } => write!(f, "PLL {modulator} phase"),
        }
    }
}

/// What a defined Z-controller reads, as far as its name says. Nanonis
/// names them by law and signal (`log Current`, `abs Conductance`,
/// `Frequency (neg)`), and over TCP the name is the only place the
/// definition shows, so this is parsed from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ZLoopInput {
    pub quantity: ZQuantity,
    /// `(neg)` in the name: the loop acts on the negative of its input,
    /// as a frequency loop on an attractive shift does.
    pub negative: bool,
    pub law: ZLaw,
}

/// The physical quantity a Z-controller's input signal is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZQuantity {
    Current,
    Conductance,
    Frequency,
    Phase,
    Excitation,
    Amplitude,
    /// A name this code does not know; the setpoint's unit is unknown too.
    Unknown,
}

/// How the error is formed from the input: the log of the ratio to the
/// setpoint, or the plain difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZLaw {
    Log,
    Linear,
}

impl ZLoopInput {
    /// From a defined controller's name, case-insensitively: a leading
    /// `log` is the log law, anything else (`abs`, a bare `Frequency`) is
    /// linear; the quantity is the first word this code knows.
    pub fn from_name(name: &str) -> Self {
        let lower = name.to_lowercase();
        let law = if lower.trim_start().starts_with("log") {
            ZLaw::Log
        } else {
            ZLaw::Linear
        };
        let negative = lower.contains("(neg)");
        let quantity = if lower.contains("current") {
            ZQuantity::Current
        } else if lower.contains("conductance") {
            ZQuantity::Conductance
        } else if lower.contains("freq") {
            ZQuantity::Frequency
        } else if lower.contains("phase") {
            ZQuantity::Phase
        } else if lower.contains("excitation") {
            ZQuantity::Excitation
        } else if lower.contains("amplitude") {
            ZQuantity::Amplitude
        } else {
            ZQuantity::Unknown
        };
        Self {
            quantity,
            law,
            negative,
        }
    }

    /// The SI unit the setpoint is in, if the quantity is known.
    pub fn unit(&self) -> Option<&'static str> {
        match self.quantity {
            ZQuantity::Current => Some("A"),
            ZQuantity::Conductance => Some("S"),
            ZQuantity::Frequency => Some("Hz"),
            ZQuantity::Phase => Some("°"),
            ZQuantity::Excitation => Some("V"),
            ZQuantity::Amplitude => Some("m"),
            ZQuantity::Unknown => None,
        }
    }
}

/// The Z-controller's settable parameters. The setpoint is in the unit of
/// the active controller's input signal, amperes for a current loop and
/// hertz for a frequency-shift loop, which is why it carries no unit here;
/// [`input`](Self::input) says which from the active controller's name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ZControllerParams {
    /// Which of the defined controllers runs, by name from the list the
    /// reading carries. Empty leaves the active one as it is.
    pub active: String,
    /// Setpoint, in the input signal's own unit.
    pub setpoint: f64,
    /// Proportional gain: how far Z moves per unit of error.
    #[schemars(extend("x-unit" = "m", "x-display-unit" = "pm"))]
    pub p_gain_m: f64,
    /// Time constant; the integral gain is P over this.
    #[schemars(extend("x-unit" = "s", "x-display-unit" = "ms"))]
    pub time_constant_s: f64,
    /// Retract by this much when the loop is switched off.
    #[schemars(extend("x-unit" = "m", "x-display-unit" = "nm"))]
    pub tip_lift_m: f64,
    /// Z is averaged over this before the loop switches off, so the
    /// parked position is reproducible.
    #[schemars(extend("x-unit" = "s", "x-display-unit" = "ms"))]
    pub switch_off_delay_s: f64,
    /// Slew rate of a withdraw. Unset is unlimited, which the module
    /// shows as `Inf`; a JSON number cannot be infinite, so it is an
    /// option here.
    #[schemars(extend("x-unit" = "m/s", "x-display-unit" = "nm/s"))]
    pub withdraw_rate_m_s: Option<f64>,
    /// Whether the Z position limits below apply.
    pub limits_enabled: bool,
    /// Z position limits, high then low. Without `limits_enabled` these
    /// read as the piezo range and a write is ignored.
    #[schemars(extend("x-unit" = "m", "x-display-unit" = "nm", "x-enabled-by" = "limits_enabled"))]
    pub limits_m: (f64, f64),
}

impl Default for ZControllerParams {
    fn default() -> Self {
        Self {
            active: String::new(),
            setpoint: 50e-12,
            p_gain_m: 200e-12,
            time_constant_s: 1e-3,
            tip_lift_m: 0.0,
            switch_off_delay_s: 0.0,
            withdraw_rate_m_s: None,
            limits_enabled: false,
            limits_m: (0.0, 0.0),
        }
    }
}

impl ZControllerParams {
    /// What the active controller reads, from its name.
    pub fn input(&self) -> ZLoopInput {
        ZLoopInput::from_name(&self.active)
    }
}

/// The PLL amplitude loop: keeps the oscillation amplitude at a setpoint
/// by driving the excitation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PllAmplitudeParams {
    /// Oscillation amplitude to hold.
    #[schemars(extend("x-unit" = "m", "x-display-unit" = "pm"))]
    pub setpoint_m: f64,
    /// Proportional gain, excitation volts per metre of amplitude error.
    #[schemars(extend("x-unit" = "V/m"))]
    pub p_gain_v_m: f64,
    /// Time constant; the integral gain is P over this.
    #[schemars(extend("x-unit" = "s", "x-display-unit" = "ms"))]
    pub time_constant_s: f64,
    /// Demodulator bandwidth of the amplitude signal.
    #[schemars(extend("x-unit" = "Hz"))]
    pub bandwidth_hz: f64,
}

impl Default for PllAmplitudeParams {
    fn default() -> Self {
        Self {
            setpoint_m: 100e-12,
            p_gain_v_m: 1e5,
            time_constant_s: 1e-3,
            bandwidth_hz: 100.0,
        }
    }
}

/// The PLL phase loop: keeps the phase at its reference by shifting the
/// excitation frequency, which is what makes the frequency shift a signal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PllPhaseParams {
    /// Proportional gain, hertz of frequency shift per degree of phase error.
    #[schemars(extend("x-unit" = "Hz/°"))]
    pub p_gain_hz_deg: f64,
    /// Time constant; the integral gain is P over this.
    #[schemars(extend("x-unit" = "s", "x-display-unit" = "ms"))]
    pub time_constant_s: f64,
    /// Demodulator bandwidth of the phase signal.
    #[schemars(extend("x-unit" = "Hz"))]
    pub bandwidth_hz: f64,
}

impl Default for PllPhaseParams {
    fn default() -> Self {
        Self {
            p_gain_hz_deg: 1.0,
            time_constant_s: 1e-3,
            bandwidth_hz: 100.0,
        }
    }
}

/// The parameters of one controller, whichever kind it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControllerParams {
    Z(ZControllerParams),
    PllAmplitude(PllAmplitudeParams),
    PllPhase(PllPhaseParams),
}

impl ControllerParams {
    /// The defaults for a controller of this id's kind.
    pub fn default_for(id: ControllerId) -> Self {
        match id {
            ControllerId::Z => ControllerParams::Z(ZControllerParams::default()),
            ControllerId::PllAmplitude { .. } => {
                ControllerParams::PllAmplitude(PllAmplitudeParams::default())
            }
            ControllerId::PllPhase { .. } => ControllerParams::PllPhase(PllPhaseParams::default()),
        }
    }

    /// Whether these parameters are of the kind `id` names.
    pub fn fits(&self, id: ControllerId) -> bool {
        matches!(
            (self, id),
            (ControllerParams::Z(_), ControllerId::Z)
                | (
                    ControllerParams::PllAmplitude(_),
                    ControllerId::PllAmplitude { .. }
                )
                | (ControllerParams::PllPhase(_), ControllerId::PllPhase { .. })
        )
    }

    /// The JSON Schema of this kind's parameter struct, for a form.
    pub fn schema_for(id: ControllerId) -> serde_json::Value {
        let schema = match id {
            ControllerId::Z => schemars::schema_for!(ZControllerParams),
            ControllerId::PllAmplitude { .. } => schemars::schema_for!(PllAmplitudeParams),
            ControllerId::PllPhase { .. } => schemars::schema_for!(PllPhaseParams),
        };
        serde_json::to_value(schema).unwrap_or(serde_json::Value::Null)
    }

    /// The parameters as a flat JSON object without the `kind` tag, the
    /// shape a form of `schema_for` edits.
    pub fn to_fields(&self) -> serde_json::Value {
        let inner = match self {
            ControllerParams::Z(p) => serde_json::to_value(p),
            ControllerParams::PllAmplitude(p) => serde_json::to_value(p),
            ControllerParams::PllPhase(p) => serde_json::to_value(p),
        };
        inner.unwrap_or(serde_json::Value::Null)
    }

    /// The loop's setpoint, where the kind has one: the Z-controller's, in
    /// its input's unit, and the amplitude loop's in metres. The phase
    /// loop has none.
    pub fn setpoint(&self) -> Option<f64> {
        match self {
            ControllerParams::Z(p) => Some(p.setpoint),
            ControllerParams::PllAmplitude(p) => Some(p.setpoint_m),
            ControllerParams::PllPhase(_) => None,
        }
    }

    /// These parameters with `other`'s setpoint in place of their own,
    /// when both are of one kind. What applying a preset writes: its gains
    /// over the operating point whoever owns the run has set.
    pub fn with_setpoint_of(mut self, other: &ControllerParams) -> Self {
        match (&mut self, other) {
            (ControllerParams::Z(p), ControllerParams::Z(o)) => p.setpoint = o.setpoint,
            (ControllerParams::PllAmplitude(p), ControllerParams::PllAmplitude(o)) => {
                p.setpoint_m = o.setpoint_m;
            }
            _ => {}
        }
        self
    }

    /// The reverse of [`to_fields`](Self::to_fields).
    pub fn from_fields(id: ControllerId, fields: serde_json::Value) -> Result<Self, String> {
        let params = match id {
            ControllerId::Z => {
                ControllerParams::Z(serde_json::from_value(fields).map_err(|e| e.to_string())?)
            }
            ControllerId::PllAmplitude { .. } => ControllerParams::PllAmplitude(
                serde_json::from_value(fields).map_err(|e| e.to_string())?,
            ),
            ControllerId::PllPhase { .. } => ControllerParams::PllPhase(
                serde_json::from_value(fields).map_err(|e| e.to_string())?,
            ),
        };
        Ok(params)
    }
}

/// What a read gives back: the parameters and what cannot be written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ControllerReading {
    pub id: ControllerId,
    pub params: ControllerParams,
    /// Whether the loop is on.
    pub enabled: bool,
    /// The module's status word where it has one (`on`, `off`, `hold`,
    /// `safe tip`, `withdrawing`); empty otherwise.
    #[serde(default)]
    pub status: String,
    /// For the Z-controller, the controllers Nanonis has defined, in the
    /// order the module lists them.
    #[serde(default)]
    pub available: Vec<String>,
}

impl LogEvent for ControllerReading {
    const KIND: &'static str = "controller/read";
}

/// One controller written: what it held before and what it holds now,
/// read back after the write, so a clamped or refused value shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ControllerAppliedEvent {
    pub id: ControllerId,
    pub before: ControllerParams,
    pub after: ControllerParams,
}

impl LogEvent for ControllerAppliedEvent {
    const KIND: &'static str = "controller/applied";
}

/// The events these jobs write.
pub fn log_schema() -> ToolSchema {
    ToolSchema::new("controllers")
        .with::<ControllerReading>()
        .with::<ControllerAppliedEvent>()
        .including(crate::routine::log_schema())
}

/// A set of controller parameters to apply together, as a TOML file: the
/// settings file that defines the controllers, loaded first, then each
/// controller's parameters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ControllerProfile {
    /// A Nanonis settings file to load before anything is written. Where
    /// a controller's input signal or log input is set; optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_file: Option<PathBuf>,
    /// The controllers to write, in this order.
    #[serde(default)]
    pub controllers: Vec<ProfileEntry>,
}

/// One controller's parameters in a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileEntry {
    pub id: ControllerId,
    pub params: ControllerParams,
}

impl ControllerProfile {
    /// Every entry's parameters are of its id's kind.
    pub fn validate(&self) -> Result<(), String> {
        for entry in &self.controllers {
            if !entry.params.fits(entry.id) {
                return Err(format!(
                    "the parameters given for {} are of another kind",
                    entry.id
                ));
            }
        }
        Ok(())
    }
}

/// The operating point a preset's gains were tuned at. A preset never
/// writes any of these; the run that applies it owns them. They are here
/// so a reader can tell whether the gains transfer: a log current loop's
/// do not depend on the setpoint or the bias, a linear or a
/// frequency-shift loop's do.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TunedAt {
    /// The loop's setpoint, in its input's unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setpoint: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bias_v: Option<f64>,
    /// The oscillation amplitude, for a loop on the frequency shift.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amplitude_m: Option<f64>,
    /// Anything else worth knowing: the sample, the tip's state.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// One controller's parameters under a name, with where they were tuned:
/// what a lab keeps and recalls, and what a routine names in its config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Preset {
    pub name: String,
    pub id: ControllerId,
    pub params: ControllerParams,
    #[serde(default)]
    pub tuned_at: TunedAt,
}

impl Preset {
    /// The name is not empty and the parameters are of the id's kind.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("a preset needs a name".into());
        }
        if !self.params.fits(self.id) {
            return Err(format!(
                "preset {:?}: the parameters given for {} are of another kind",
                self.name, self.id
            ));
        }
        Ok(())
    }

    /// Whether this is the preset `name` means: names match ignoring case
    /// and surrounding blanks, as they do everywhere presets are looked up.
    pub fn is_named(&self, name: &str) -> bool {
        name_key(&self.name) == name_key(name)
    }

    /// What applying this preset writes over `current`: its parameters
    /// with `current`'s setpoint kept.
    pub fn params_over(&self, current: &ControllerParams) -> ControllerParams {
        self.params.clone().with_setpoint_of(current)
    }

    /// Whether the gains depend on the operating point they were tuned
    /// at. A log loop on the current is the one case where they do not:
    /// its plant is decades per ångström, set by the barrier and not by
    /// the setpoint or the bias. Everything else, a linear loop whose
    /// gain scales with the setpoint or a frequency loop whose slope
    /// changes with distance, bias and amplitude, does.
    pub fn depends_on_operating_point(&self) -> bool {
        match &self.params {
            ControllerParams::Z(p) => {
                let input = p.input();
                !(input.law == ZLaw::Log && input.quantity == ZQuantity::Current)
            }
            _ => true,
        }
    }
}

fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// A preset file: `[[presets]]` tables, names unique within it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PresetFile {
    #[serde(default)]
    pub presets: Vec<Preset>,
}

impl PresetFile {
    /// Every preset valid and no name twice.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for preset in &self.presets {
            preset.validate()?;
            if !seen.insert(name_key(&preset.name)) {
                return Err(format!("preset {:?} is defined twice", preset.name));
            }
        }
        Ok(())
    }

    /// By name, case-insensitively.
    pub fn find(&self, name: &str) -> Option<&Preset> {
        self.presets.iter().find(|p| p.is_named(name))
    }
}

/// Where presets are kept. One implementation reads and writes a TOML
/// file; a database can stand behind the same calls later without the
/// pane or a routine noticing.
pub trait PresetStore {
    /// Every preset, in the store's order.
    fn list(&self) -> Result<Vec<Preset>, String>;

    /// One preset by name, if there is one.
    fn get(&self, name: &str) -> Result<Option<Preset>, String> {
        Ok(self.list()?.into_iter().find(|p| p.is_named(name)))
    }

    /// Add a preset, or replace the one with its name.
    fn put(&mut self, preset: Preset) -> Result<(), String>;

    /// Remove the preset with this name; nothing happens when there is none.
    fn remove(&mut self, name: &str) -> Result<(), String>;
}

/// Presets in one TOML file. A missing file is an empty store, so the
/// first save creates it; every call reads the file, since the pane and
/// a routine may share it.
#[derive(Debug, Clone)]
pub struct TomlPresetStore {
    pub path: PathBuf,
}

impl TomlPresetStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The file as it stands, validated; empty when there is no file.
    pub fn read(&self) -> Result<PresetFile, String> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(PresetFile::default()),
            Err(e) => return Err(format!("Cannot read {}: {e}", self.path.display())),
        };
        let file: PresetFile =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", self.path.display()))?;
        file.validate()
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(file)
    }

    fn write(&self, file: &PresetFile) -> Result<(), String> {
        file.validate()?;
        let text = toml::to_string_pretty(file).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, text)
            .map_err(|e| format!("Cannot write {}: {e}", self.path.display()))
    }
}

impl PresetStore for TomlPresetStore {
    fn list(&self) -> Result<Vec<Preset>, String> {
        Ok(self.read()?.presets)
    }

    fn put(&mut self, preset: Preset) -> Result<(), String> {
        preset.validate()?;
        let mut file = self.read()?;
        match file.presets.iter_mut().find(|p| p.is_named(&preset.name)) {
            Some(slot) => *slot = preset,
            None => file.presets.push(preset),
        }
        self.write(&file)
    }

    fn remove(&mut self, name: &str) -> Result<(), String> {
        let mut file = self.read()?;
        file.presets.retain(|p| !p.is_named(name));
        self.write(&file)
    }
}

/// Write one preset's parameters to its controller, keeping the setpoint
/// the controller holds, and read it back: a `controller/applied` event
/// and a `controller/read` of the result. The loop is not switched.
///
/// For the Z-controller the preset's `active` has to be one of the loops
/// the module has defined; a name it does not know is refused before
/// anything is written.
#[derive(Debug, Clone)]
pub struct ApplyPreset {
    pub preset: Preset,
}

impl ApplyPreset {
    /// The shared body: also what a routine calls to apply a preset it was
    /// configured with, since a routine is not a job of its own.
    pub fn apply(
        preset: &Preset,
        controller: &mut dyn SpmController,
        events: &dyn EventEmitter,
    ) -> Result<ControllerReading, SpmError> {
        preset.validate().map_err(SpmError::Workflow)?;
        require(controller, Capability::Controllers)?;
        let current = controller.read_controller(preset.id)?;
        if let ControllerParams::Z(p) = &preset.params
            && !p.active.is_empty()
            && !current.available.is_empty()
            && !current.available.iter().any(|a| a == &p.active)
        {
            return Err(SpmError::Workflow(format!(
                "preset {:?} names a Z-controller called {:?}; the module has {}",
                preset.name,
                p.active,
                current.available.join(", ")
            )));
        }
        let params = preset.params_over(&current.params);
        write_and_report(controller, events, preset.id, current.params, &params)
    }
}

/// Write `params`, read the loop back, and report both: the
/// `controller/applied` event against `before`, then the reading.
fn write_and_report(
    controller: &mut dyn SpmController,
    events: &dyn EventEmitter,
    id: ControllerId,
    before: ControllerParams,
    params: &ControllerParams,
) -> Result<ControllerReading, SpmError> {
    controller.write_controller(id, params)?;
    let after = controller.read_controller(id)?;
    events.emit(Event::typed(&ControllerAppliedEvent {
        id,
        before,
        after: after.params.clone(),
    }));
    events.emit(Event::typed(&after));
    Ok(after)
}

impl Job for ApplyPreset {
    fn name(&self) -> &str {
        "controllers_apply_preset"
    }

    fn log_schema(&self) -> ToolSchema {
        log_schema()
    }

    fn header_config(&self) -> serde_json::Value {
        serde_json::to_value(&self.preset).unwrap_or(serde_json::Value::Null)
    }

    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        Self::apply(&self.preset, cx.controller, cx.events)?;
        Ok(Outcome::Completed)
    }
}

/// Read every controller the connection has and write each as a
/// `controller/read` event.
#[derive(Debug, Default)]
pub struct ReadControllers;

impl Job for ReadControllers {
    fn name(&self) -> &str {
        "controllers_read"
    }

    fn log_schema(&self) -> ToolSchema {
        log_schema()
    }

    fn header_config(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        read_all(cx.controller, cx.events)?;
        Ok(Outcome::Completed)
    }
}

/// Load the profile's settings file (a `routine/settings_loaded` event,
/// as a routine's load writes), then write each controller and read it
/// back, writing a `controller/applied` event per written controller and
/// a `controller/read` of every controller, the written ones as they read
/// back. Loops are not switched on or off.
#[derive(Debug, Clone)]
pub struct ApplyProfile {
    pub profile: ControllerProfile,
}

impl Job for ApplyProfile {
    fn name(&self) -> &str {
        "controllers_apply"
    }

    fn log_schema(&self) -> ToolSchema {
        log_schema()
    }

    fn header_config(&self) -> serde_json::Value {
        serde_json::to_value(&self.profile).unwrap_or(serde_json::Value::Null)
    }

    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        self.profile.validate().map_err(SpmError::Workflow)?;
        if let Some(path) = &self.profile.settings_file {
            require(cx.controller, Capability::Presets)?;
            cx.controller.load_settings(path)?;
            cx.events.emit(Event::typed(&SettingsLoadedEvent {
                path: path.display().to_string(),
            }));
        }
        require(cx.controller, Capability::Controllers)?;
        let mut written = Vec::new();
        for entry in &self.profile.controllers {
            if cx.shutdown.is_requested() {
                return Ok(Outcome::StoppedByUser);
            }
            let before = cx.controller.read_controller(entry.id)?.params;
            write_and_report(cx.controller, cx.events, entry.id, before, &entry.params)?;
            written.push(entry.id);
        }
        for id in cx.controller.controllers()? {
            if !written.contains(&id) {
                cx.events
                    .emit(Event::typed(&cx.controller.read_controller(id)?));
            }
        }
        Ok(Outcome::Completed)
    }
}

/// Switch one loop on or off, then read it back.
#[derive(Debug, Clone, Copy)]
pub struct SetControllerEnabled {
    pub id: ControllerId,
    pub on: bool,
}

impl Job for SetControllerEnabled {
    fn name(&self) -> &str {
        "controllers_switch"
    }

    fn log_schema(&self) -> ToolSchema {
        log_schema()
    }

    fn header_config(&self) -> serde_json::Value {
        serde_json::json!({ "id": self.id, "on": self.on })
    }

    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        require(cx.controller, Capability::Controllers)?;
        cx.controller.set_controller_enabled(self.id, self.on)?;
        let reading = cx.controller.read_controller(self.id)?;
        cx.events.emit(Event::typed(&reading));
        Ok(Outcome::Completed)
    }
}

fn read_all(controller: &mut dyn SpmController, events: &dyn EventEmitter) -> Result<(), SpmError> {
    require(controller, Capability::Controllers)?;
    for id in controller.controllers()? {
        let reading = controller.read_controller(id)?;
        events.emit(Event::typed(&reading));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip_prep_preset() -> Preset {
        Preset {
            name: "tip prep".into(),
            id: ControllerId::Z,
            params: ControllerParams::Z(ZControllerParams {
                active: "log Current".into(),
                setpoint: 100e-12,
                p_gain_m: 1.5e-12,
                time_constant_s: 50e-6,
                ..Default::default()
            }),
            tuned_at: TunedAt {
                setpoint: Some(100e-12),
                bias_v: Some(1.0),
                ..Default::default()
            },
        }
    }

    #[test]
    fn a_preset_file_is_toml_with_names_unique() {
        let mut file = PresetFile {
            presets: vec![tip_prep_preset()],
        };
        let text = toml::to_string_pretty(&file).unwrap();
        assert!(text.contains("[[presets]]"), "{text}");
        assert!(text.contains("bias_v = 1.0"), "{text}");
        assert!(
            !text.contains("amplitude_m"),
            "unset points are left out: {text}"
        );
        let back: PresetFile = toml::from_str(&text).unwrap();
        assert_eq!(back, file);
        assert!(
            file.find("Tip Prep").is_some(),
            "names match case-insensitively"
        );

        file.presets.push(tip_prep_preset());
        assert!(file.validate().unwrap_err().contains("twice"));
    }

    #[test]
    fn applying_a_preset_keeps_the_setpoint_the_loop_holds() {
        let preset = tip_prep_preset();
        let current = ControllerParams::Z(ZControllerParams {
            setpoint: 50e-12,
            p_gain_m: 200e-12,
            ..Default::default()
        });
        match preset.params_over(&current) {
            ControllerParams::Z(p) => {
                assert_eq!(
                    p.setpoint, 50e-12,
                    "the setpoint is the run's, not the preset's"
                );
                assert_eq!(p.p_gain_m, 1.5e-12, "the gains are the preset's");
                assert_eq!(p.active, "log Current");
            }
            other => panic!("not Z parameters: {other:?}"),
        }
    }

    #[test]
    fn only_a_log_current_loop_transfers_across_operating_points() {
        assert!(!tip_prep_preset().depends_on_operating_point());
        let mut imaging = tip_prep_preset();
        imaging.params = ControllerParams::Z(ZControllerParams {
            active: "Frequency (neg)".into(),
            ..Default::default()
        });
        assert!(imaging.depends_on_operating_point());
        let mut amplitude = tip_prep_preset();
        amplitude.id = ControllerId::PllAmplitude { modulator: 1 };
        amplitude.params = ControllerParams::PllAmplitude(PllAmplitudeParams::default());
        assert!(amplitude.depends_on_operating_point());
    }

    #[test]
    fn the_toml_store_creates_replaces_and_removes() {
        let dir = std::env::temp_dir().join(format!(
            "rusty-tip-presets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = TomlPresetStore::new(dir.join("controllers.toml"));
        assert!(
            store.list().unwrap().is_empty(),
            "no file is an empty store"
        );

        store.put(tip_prep_preset()).unwrap();
        let mut faster = tip_prep_preset();
        if let ControllerParams::Z(p) = &mut faster.params {
            p.time_constant_s = 25e-6;
        }
        store.put(faster.clone()).unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1, "a put by an existing name replaces");
        assert_eq!(listed[0], faster);
        assert_eq!(store.get("TIP PREP").unwrap(), Some(faster));

        store.remove("tip prep").unwrap();
        assert!(store.list().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ids_have_names() {
        assert_eq!(ControllerId::Z.to_string(), "Z-controller");
        assert_eq!(
            ControllerId::PllPhase { modulator: 2 }.to_string(),
            "PLL 2 phase"
        );
        let json = serde_json::to_string(&ControllerId::PllPhase { modulator: 1 }).unwrap();
        assert_eq!(json, r#"{"kind":"pll_phase","modulator":1}"#);
    }

    #[test]
    fn params_round_trip_through_fields() {
        let id = ControllerId::PllAmplitude { modulator: 1 };
        let params = ControllerParams::PllAmplitude(PllAmplitudeParams {
            setpoint_m: 50e-12,
            ..Default::default()
        });
        let fields = params.to_fields();
        assert!(fields.get("kind").is_none(), "fields carry no tag");
        assert_eq!(fields["setpoint_m"], 50e-12);
        assert_eq!(ControllerParams::from_fields(id, fields).unwrap(), params);
        assert!(params.fits(id));
        assert!(!params.fits(ControllerId::Z));
    }

    #[test]
    fn a_profile_is_toml_and_checks_its_kinds() {
        let profile = ControllerProfile {
            settings_file: Some(PathBuf::from("afm.ini")),
            controllers: vec![
                ProfileEntry {
                    id: ControllerId::Z,
                    params: ControllerParams::Z(ZControllerParams {
                        active: "Current log".into(),
                        ..Default::default()
                    }),
                },
                ProfileEntry {
                    id: ControllerId::PllPhase { modulator: 1 },
                    params: ControllerParams::PllPhase(PllPhaseParams::default()),
                },
            ],
        };
        let text = toml::to_string_pretty(&profile).unwrap();
        let back: ControllerProfile = toml::from_str(&text).unwrap();
        assert_eq!(back, profile);
        assert!(profile.validate().is_ok());

        let wrong = ControllerProfile {
            settings_file: None,
            controllers: vec![ProfileEntry {
                id: ControllerId::Z,
                params: ControllerParams::PllPhase(PllPhaseParams::default()),
            }],
        };
        assert!(wrong.validate().unwrap_err().contains("Z-controller"));
    }

    #[test]
    fn a_loop_name_says_its_law_and_quantity() {
        let log_current = ZLoopInput::from_name("log Current");
        assert_eq!(log_current.law, ZLaw::Log);
        assert_eq!(log_current.quantity, ZQuantity::Current);
        assert_eq!(log_current.unit(), Some("A"));
        let df = ZLoopInput::from_name("Frequency (neg)");
        assert_eq!(df.law, ZLaw::Linear);
        assert_eq!(df.unit(), Some("Hz"));
        assert_eq!(ZLoopInput::from_name("abs Conductance").unit(), Some("S"));
        assert_eq!(ZLoopInput::from_name("Amplitude").unit(), Some("m"));
        assert_eq!(ZLoopInput::from_name("").unit(), None);
        assert_eq!(ZLoopInput::from_name("").law, ZLaw::Linear);
    }

    #[test]
    fn an_unlimited_withdraw_rate_survives_json() {
        let params = ControllerParams::Z(ZControllerParams::default());
        let fields = params.to_fields();
        assert!(fields["withdraw_rate_m_s"].is_null());
        assert_eq!(
            ControllerParams::from_fields(ControllerId::Z, fields).unwrap(),
            params
        );
    }

    #[test]
    fn the_schema_carries_units_and_the_limits_gate() {
        let schema = ControllerParams::schema_for(ControllerId::Z);
        let props = &schema["properties"];
        assert_eq!(props["p_gain_m"]["x-display-unit"], "pm");
        assert_eq!(props["limits_m"]["x-enabled-by"], "limits_enabled");
        let pll = ControllerParams::schema_for(ControllerId::PllPhase { modulator: 1 });
        assert_eq!(pll["properties"]["p_gain_hz_deg"]["x-unit"], "Hz/°");
    }
}
