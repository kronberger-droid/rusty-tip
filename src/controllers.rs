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

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::{Event, EventEmitter};
use crate::experiment_log::{LogEvent, ToolSchema};
use crate::routine::Outcome;
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

impl ControllerId {
    /// A stable key for maps and ids: `z`, `pll1_amplitude`.
    pub fn key(&self) -> String {
        match self {
            ControllerId::Z => "z".into(),
            ControllerId::PllAmplitude { modulator } => format!("pll{modulator}_amplitude"),
            ControllerId::PllPhase { modulator } => format!("pll{modulator}_phase"),
        }
    }
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

/// The Z-controller's settable parameters. The setpoint is in the unit of
/// the active controller's input signal, amperes for a current loop and
/// hertz for a frequency-shift loop, which is why it carries no unit here.
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
    #[schemars(extend("x-unit" = "s", "x-display-unit" = "µs"))]
    pub time_constant_s: f64,
    /// Retract by this much when the loop is switched off.
    #[schemars(extend("x-unit" = "m", "x-display-unit" = "nm"))]
    pub tip_lift_m: f64,
    /// Z is averaged over this before the loop switches off, so the
    /// parked position is reproducible.
    #[schemars(extend("x-unit" = "s", "x-display-unit" = "ms"))]
    pub switch_off_delay_s: f64,
    /// Slew rate of a withdraw.
    #[schemars(extend("x-unit" = "m/s", "x-display-unit" = "nm/s"))]
    pub withdraw_rate_m_s: f64,
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
            setpoint: 100e-12,
            p_gain_m: 10e-12,
            time_constant_s: 100e-6,
            tip_lift_m: 0.0,
            switch_off_delay_s: 0.0,
            withdraw_rate_m_s: 1e-6,
            limits_enabled: false,
            limits_m: (0.0, 0.0),
        }
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

/// A settings file loaded as part of a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SettingsLoadedEvent {
    pub path: String,
}

impl LogEvent for SettingsLoadedEvent {
    const KIND: &'static str = "controller/settings_loaded";
}

/// The events these jobs write.
pub fn log_schema() -> ToolSchema {
    ToolSchema::new("controllers")
        .with::<ControllerReading>()
        .with::<ControllerAppliedEvent>()
        .with::<SettingsLoadedEvent>()
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileEntry {
    pub id: ControllerId,
    pub params: ControllerParams,
}

impl ControllerProfile {
    /// The profile's parameters by controller.
    pub fn by_id(&self) -> BTreeMap<ControllerId, &ControllerParams> {
        self.controllers.iter().map(|e| (e.id, &e.params)).collect()
    }

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

/// Load the profile's settings file, then write each controller and read it
/// back, writing a `controller/applied` event per controller and a final
/// `controller/read` of everything. Loops are not switched on or off.
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
        for entry in &self.profile.controllers {
            if cx.shutdown.is_requested() {
                return Ok(Outcome::StoppedByUser);
            }
            let before = cx.controller.read_controller(entry.id)?.params;
            cx.controller.write_controller(entry.id, &entry.params)?;
            let after = cx.controller.read_controller(entry.id)?.params;
            cx.events.emit(Event::typed(&ControllerAppliedEvent {
                id: entry.id,
                before,
                after,
            }));
        }
        read_all(cx.controller, cx.events)?;
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

fn require(controller: &mut dyn SpmController, cap: Capability) -> Result<(), SpmError> {
    if controller.capabilities().contains(&cap) {
        Ok(())
    } else {
        Err(SpmError::Unsupported(format!(
            "this controller has no {cap:?} capability"
        )))
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

    #[test]
    fn ids_have_stable_keys_and_names() {
        assert_eq!(ControllerId::Z.key(), "z");
        assert_eq!(
            ControllerId::PllAmplitude { modulator: 1 }.key(),
            "pll1_amplitude"
        );
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
    fn the_schema_carries_units_and_the_limits_gate() {
        let schema = ControllerParams::schema_for(ControllerId::Z);
        let props = &schema["properties"];
        assert_eq!(props["p_gain_m"]["x-display-unit"], "pm");
        assert_eq!(props["limits_m"]["x-enabled-by"], "limits_enabled");
        let pll = ControllerParams::schema_for(ControllerId::PllPhase { modulator: 1 });
        assert_eq!(pll["properties"]["p_gain_hz_deg"]["x-unit"], "Hz/°");
    }
}
