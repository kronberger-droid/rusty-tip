//! Operating points: every feedback loop with its setpoint, the bias, and
//! the scan's size, angle, resolution and speed, under one name.
//!
//! A preset is one loop's gains, which transfer between setpoints for a log
//! current loop and little else. What a lab actually tunes is the set: the
//! loops at a bias, scanned at a speed over a frame of a given size and
//! resolution. An operating point keeps that set together, captured from
//! the controller in one read and applied back in one job.
//!
//! The frame's centre is not part of it: applying a point never moves the
//! scan window across the sample. Points live in their own TOML file, apart
//! from the presets, so saving one never rewrites the other.

use std::path::PathBuf;

use nanonis_rs::scan::{ScanConfig, ScanFrame};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::controllers::{self, ControllerParams, ProfileEntry};
use crate::event::{Event, EventEmitter};
use crate::experiment_log::{LogEvent, ToolSchema};
use crate::routine::{Outcome, require};
use crate::session::{Job, JobCx};
use crate::spm_controller::{Capability, ScanBuffer, SpmController};
use crate::spm_error::SpmError;

/// Where operating points are kept unless configured otherwise: beside the
/// presets, in a file of their own.
pub const DEFAULT_OPERATING_POINTS_FILE: &str = "./operating_points.toml";

/// Which of a line's speed and time Nanonis holds when the other changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KeepConstant {
    LinearSpeed,
    TimePerLine,
}

/// The scan's speeds, as Nanonis keeps them: both ways, by speed and by time
/// per line, and which of the two it holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScanSpeed {
    pub forward_m_s: f64,
    pub backward_m_s: f64,
    pub forward_time_per_line_s: f64,
    pub backward_time_per_line_s: f64,
    pub keep: KeepConstant,
    /// Backward speed over forward speed.
    pub backward_ratio: f64,
}

/// What an operating point holds of the scan. Not the frame's centre:
/// applying a point keeps the window where it is on the sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScanSettings {
    pub width_m: f64,
    pub height_m: f64,
    pub angle_deg: f64,
    /// Pixels per line; Nanonis takes multiples of 16.
    pub pixels: i32,
    pub lines: i32,
    pub speed: ScanSpeed,
}

/// The bias and the scan: what a point holds besides the loops, and what an
/// apply reports before and after, as `controller/applied` does for each loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BiasAndScan {
    pub bias_v: f64,
    pub scan: ScanSettings,
}

/// Everything an operating point sets, as read from the controller: the
/// `operating_point/read` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OperatingState {
    pub bias_v: f64,
    pub scan: ScanSettings,
    /// Every loop, setpoint included, in the controller's order.
    pub controllers: Vec<ProfileEntry>,
}

impl LogEvent for OperatingState {
    const KIND: &'static str = "operating_point/read";
}

/// An operating point applied: the bias and scan before and after, read
/// back so a value the controller clamped or coerced shows. Each loop has
/// its own `controller/applied`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OperatingPointApplied {
    pub name: String,
    pub before: BiasAndScan,
    pub after: BiasAndScan,
}

impl LogEvent for OperatingPointApplied {
    const KIND: &'static str = "operating_point/applied";
}

/// An operating state under a name: what a lab keeps and recalls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OperatingPoint {
    pub name: String,
    /// Anything worth knowing: the sample, the tip's state.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// When it was saved, local time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<String>,
    pub bias_v: f64,
    pub scan: ScanSettings,
    pub controllers: Vec<ProfileEntry>,
}

impl OperatingPoint {
    /// A read state under `name`.
    pub fn from_state(
        name: &str,
        note: &str,
        saved_at: Option<String>,
        state: OperatingState,
    ) -> Self {
        Self {
            name: name.trim().to_string(),
            note: note.trim().to_string(),
            saved_at,
            bias_v: state.bias_v,
            scan: state.scan,
            controllers: state.controllers,
        }
    }

    /// A name, parameters of each loop's kind, and a frame and resolution
    /// that can be scanned.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("an operating point needs a name".into());
        }
        let what = format!("operating point {:?}", self.name);
        for entry in &self.controllers {
            if !entry.params.fits(entry.id) {
                return Err(format!(
                    "{what}: the parameters given for {} are of another kind",
                    entry.id
                ));
            }
        }
        let s = &self.scan;
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if !positive(s.width_m) || !positive(s.height_m) {
            return Err(format!(
                "{what}: the frame needs a positive width and height"
            ));
        }
        if s.pixels <= 0 || s.lines <= 0 {
            return Err(format!("{what}: the frame needs pixels and lines"));
        }
        if !self.bias_v.is_finite() {
            return Err(format!("{what}: the bias is not a number"));
        }
        Ok(())
    }

    /// Names match ignoring case and surrounding blanks, as presets do.
    pub fn is_named(&self, name: &str) -> bool {
        self.name.trim().to_lowercase() == name.trim().to_lowercase()
    }

    /// The setpoint of the Z loop, if the point has one.
    pub fn z_setpoint(&self) -> Option<f64> {
        self.controllers.iter().find_map(|e| match &e.params {
            ControllerParams::Z(p) => Some(p.setpoint),
            _ => None,
        })
    }
}

/// An operating points file: `[[points]]` tables, names unique within it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OperatingPointFile {
    #[serde(default)]
    pub points: Vec<OperatingPoint>,
}

impl OperatingPointFile {
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for point in &self.points {
            point.validate()?;
            if !seen.insert(point.name.trim().to_lowercase()) {
                return Err(format!("operating point {:?} is defined twice", point.name));
            }
        }
        Ok(())
    }
}

/// Operating points in one TOML file. A missing file is an empty store, so
/// the first save creates it; every call reads the file again, since the
/// workbench and a script may share it.
#[derive(Debug, Clone)]
pub struct OperatingPointStore {
    pub path: PathBuf,
}

impl OperatingPointStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The file as it stands, validated; empty when there is no file.
    pub fn read(&self) -> Result<OperatingPointFile, String> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(OperatingPointFile::default());
            }
            Err(e) => return Err(format!("Cannot read {}: {e}", self.path.display())),
        };
        let file: OperatingPointFile =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", self.path.display()))?;
        file.validate()
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(file)
    }

    fn write(&self, file: &OperatingPointFile) -> Result<(), String> {
        file.validate()?;
        let text = toml::to_string_pretty(file).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, text)
            .map_err(|e| format!("Cannot write {}: {e}", self.path.display()))
    }

    /// Every point, in the file's order.
    pub fn list(&self) -> Result<Vec<OperatingPoint>, String> {
        Ok(self.read()?.points)
    }

    /// Add a point, or replace the one with its name.
    pub fn put(&mut self, point: OperatingPoint) -> Result<(), String> {
        point.validate()?;
        let mut file = self.read()?;
        match file.points.iter_mut().find(|p| p.is_named(&point.name)) {
            Some(slot) => *slot = point,
            None => file.points.push(point),
        }
        self.write(&file)
    }

    /// Remove the point with this name; nothing happens when there is none.
    pub fn remove(&mut self, name: &str) -> Result<(), String> {
        let mut file = self.read()?;
        file.points.retain(|p| !p.is_named(name));
        self.write(&file)
    }
}

/// An f32 from the controller as the f64 its shortest decimal form names,
/// so a saved file reads `5e-8`, not `5.000000058430487e-8`.
fn short(value: f32) -> f64 {
    value.to_string().parse().unwrap_or(f64::from(value))
}

fn read_scan(controller: &mut dyn SpmController) -> Result<ScanSettings, SpmError> {
    let frame = controller.scan_frame_get()?;
    let buffer = controller.scan_buffer_get()?;
    let speed = controller.scan_speed_get()?;
    Ok(ScanSettings {
        width_m: short(frame.width_m),
        height_m: short(frame.height_m),
        angle_deg: short(frame.angle_deg),
        pixels: buffer.pixels,
        lines: buffer.lines,
        speed: ScanSpeed {
            forward_m_s: short(speed.forward_linear_speed_m_s),
            backward_m_s: short(speed.backward_linear_speed_m_s),
            forward_time_per_line_s: short(speed.forward_time_per_line_s),
            backward_time_per_line_s: short(speed.backward_time_per_line_s),
            keep: match speed.keep_parameter_constant {
                0 => KeepConstant::LinearSpeed,
                _ => KeepConstant::TimePerLine,
            },
            backward_ratio: short(speed.speed_ratio),
        },
    })
}

fn read_bias_and_scan(controller: &mut dyn SpmController) -> Result<BiasAndScan, SpmError> {
    Ok(BiasAndScan {
        bias_v: controller.get_bias()?,
        scan: read_scan(controller)?,
    })
}

fn require_all(controller: &dyn SpmController) -> Result<(), SpmError> {
    require(controller, Capability::Bias)?;
    require(controller, Capability::Scanning)?;
    require(controller, Capability::Controllers)
}

/// Read everything an operating point sets.
pub fn read_state(controller: &mut dyn SpmController) -> Result<OperatingState, SpmError> {
    require_all(controller)?;
    let BiasAndScan { bias_v, scan } = read_bias_and_scan(controller)?;
    let mut entries = Vec::new();
    for id in controller.controllers()? {
        entries.push(ProfileEntry {
            id,
            params: controller.read_controller(id)?.params,
        });
    }
    Ok(OperatingState {
        bias_v,
        scan,
        controllers: entries,
    })
}

/// Apply a point: the scan's size, angle, resolution and speed with the
/// frame's centre kept, then every loop in full, setpoint included, then the
/// bias, and read it all back.
///
/// Refused while a scan runs, and before anything is written when a Z loop
/// names an input the module has not defined. No loop is switched on or
/// off. Writes a `controller/applied` per loop, an
/// `operating_point/applied` for the bias and scan, and an
/// `operating_point/read` of the result.
pub fn apply(
    point: &OperatingPoint,
    controller: &mut dyn SpmController,
    events: &dyn EventEmitter,
) -> Result<OperatingState, SpmError> {
    point.validate().map_err(SpmError::Workflow)?;
    require_all(controller)?;
    if controller.scan_status()? {
        return Err(SpmError::Workflow(format!(
            "a scan is running; stop it before applying operating point {:?}",
            point.name
        )));
    }
    let what = format!("operating point {:?}", point.name);
    let mut current = Vec::new();
    for entry in &point.controllers {
        let reading = controller.read_controller(entry.id)?;
        controllers::refuse_unknown_z_loop(&what, &entry.params, &reading)?;
        current.push(reading.params);
    }
    let before = read_bias_and_scan(controller)?;

    let s = &point.scan;
    let frame = controller.scan_frame_get()?;
    controller.scan_frame_set(ScanFrame::new(
        frame.center,
        s.width_m as f32,
        s.height_m as f32,
        s.angle_deg as f32,
    ))?;
    let buffer = controller.scan_buffer_get()?;
    controller.scan_buffer_set(&ScanBuffer {
        channels: buffer.channels,
        pixels: s.pixels,
        lines: s.lines,
    })?;
    controller.scan_speed_set(ScanConfig {
        forward_linear_speed_m_s: s.speed.forward_m_s as f32,
        backward_linear_speed_m_s: s.speed.backward_m_s as f32,
        forward_time_per_line_s: s.speed.forward_time_per_line_s as f32,
        backward_time_per_line_s: s.speed.backward_time_per_line_s as f32,
        keep_parameter_constant: match s.speed.keep {
            KeepConstant::LinearSpeed => 0,
            KeepConstant::TimePerLine => 1,
        },
        speed_ratio: s.speed.backward_ratio as f32,
    })?;

    for (entry, before) in point.controllers.iter().zip(current) {
        controllers::write_and_report(controller, events, entry.id, before, &entry.params)?;
    }
    // The bias last, after the setpoints it goes with.
    controller.set_bias(point.bias_v)?;

    let after = read_bias_and_scan(controller)?;
    events.emit(Event::typed(&OperatingPointApplied {
        name: point.name.clone(),
        before,
        after,
    }));
    let state = read_state(controller)?;
    events.emit(Event::typed(&state));
    Ok(state)
}

/// The events these jobs write: the controllers' own and the two here.
pub fn log_schema() -> ToolSchema {
    controllers::log_schema()
        .with::<OperatingState>()
        .with::<OperatingPointApplied>()
}

/// Read everything an operating point sets, as an `operating_point/read`
/// event, for the workbench to save under a name.
#[derive(Debug, Default)]
pub struct CaptureOperatingPoint;

impl Job for CaptureOperatingPoint {
    fn name(&self) -> &str {
        "operating_point_capture"
    }

    fn log_schema(&self) -> ToolSchema {
        log_schema()
    }

    fn header_config(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        let state = read_state(cx.controller)?;
        cx.events.emit(Event::typed(&state));
        Ok(Outcome::Completed)
    }
}

/// Apply one operating point; see [`apply`].
#[derive(Debug, Clone)]
pub struct ApplyOperatingPoint {
    pub point: OperatingPoint,
}

impl Job for ApplyOperatingPoint {
    fn name(&self) -> &str {
        "operating_point_apply"
    }

    fn log_schema(&self) -> ToolSchema {
        log_schema()
    }

    fn header_config(&self) -> serde_json::Value {
        serde_json::to_value(&self.point).unwrap_or(serde_json::Value::Null)
    }

    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        apply(&self.point, cx.controller, cx.events)?;
        Ok(Outcome::Completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::{ControllerId, ZControllerParams};

    fn point(name: &str) -> OperatingPoint {
        OperatingPoint {
            name: name.into(),
            note: String::new(),
            saved_at: None,
            bias_v: 0.5,
            scan: ScanSettings {
                width_m: 20e-9,
                height_m: 20e-9,
                angle_deg: 0.0,
                pixels: 256,
                lines: 256,
                speed: ScanSpeed {
                    forward_m_s: 10e-9,
                    backward_m_s: 10e-9,
                    forward_time_per_line_s: 2.0,
                    backward_time_per_line_s: 2.0,
                    keep: KeepConstant::LinearSpeed,
                    backward_ratio: 1.0,
                },
            },
            controllers: vec![ProfileEntry {
                id: ControllerId::Z,
                params: ControllerParams::Z(ZControllerParams::default()),
            }],
        }
    }

    /// A point survives the file: written, read back, the same.
    #[test]
    fn a_point_round_trips_through_its_toml_file() {
        let dir = std::env::temp_dir().join(format!("rusty-tip-points-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = OperatingPointStore::new(dir.join("points.toml"));
        assert!(
            store.list().unwrap().is_empty(),
            "no file is an empty store"
        );
        store.put(point("imaging")).unwrap();
        store.put(point("tip prep")).unwrap();
        let mut replaced = point("Imaging");
        replaced.bias_v = -1.0;
        store.put(replaced).unwrap();
        let points = store.list().unwrap();
        assert_eq!(points.len(), 2, "a name is replaced, case aside");
        assert_eq!(points[0].bias_v, -1.0);
        store.remove("tip prep").unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nonsense_points_are_refused() {
        let bad = |p: OperatingPoint| assert!(p.validate().is_err(), "{p:?}");
        bad(point(" "));
        let mut no_lines = point("a");
        no_lines.scan.lines = 0;
        bad(no_lines);
        let mut flat = point("a");
        flat.scan.width_m = 0.0;
        bad(flat);
        let mut wrong_kind = point("a");
        wrong_kind.controllers[0].id = ControllerId::PllPhase { modulator: 1 };
        bad(wrong_kind);
    }
}
