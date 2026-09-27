//! Feedback controllers as a workbench tool: read what the Z-controller
//! and the PLL loops hold, edit it, write it back, and keep sets of them
//! as profiles.
//!
//! Every read and write is a job on the session, so it is logged like a
//! run and the GUI thread never touches the controller. A read fills the
//! forms; Apply writes every controller whose form differs from its last
//! reading, after loading the profile's settings file, and reads all of
//! them back.

use std::collections::BTreeMap;
use std::path::PathBuf;

use eframe::egui;
use serde_json::Value;

use rusty_tip::controllers::{
    ApplyProfile, ControllerAppliedEvent, ControllerId, ControllerParams, ControllerProfile,
    ControllerReading, ProfileEntry, ReadControllers, SetControllerEnabled, SettingsLoadedEvent,
};
use rusty_tip::experiment_log::LogEvent;
use rusty_tip::session::Job;

use super::{SetupCx, Tool};
use crate::form::SchemaForm;
use crate::run_view::RunView;
use crate::units::format_si;
use crate::widgets::{Note, note, path_field};

/// The controllers a profile can be composed for before anything is read.
const KNOWN: [ControllerId; 3] = [
    ControllerId::Z,
    ControllerId::PllAmplitude { modulator: 1 },
    ControllerId::PllPhase { modulator: 1 },
];

pub struct ControllersTool {
    /// The profile file the forms were loaded from, or will be saved to.
    profile_path: String,
    /// The settings file the profile loads before writing; empty for none.
    settings_file: String,
    /// What the controller last reported, by controller.
    readings: BTreeMap<ControllerId, ControllerReading>,
    /// The forms' values: each controller's parameters as flat fields.
    edits: BTreeMap<ControllerId, Value>,
    selected: Option<ControllerId>,
    /// One form per kind, keyed by [`kind_key`].
    forms: BTreeMap<&'static str, SchemaForm>,
    message: Option<Note>,
    /// Which run's `controller/read` events have been taken into the
    /// forms: the run's start time and how many rows.
    seen: (Option<f64>, usize),
}

impl Default for ControllersTool {
    fn default() -> Self {
        let forms = KNOWN
            .iter()
            .map(|id| {
                (
                    kind_key(*id),
                    SchemaForm::new(ControllerParams::schema_for(*id)),
                )
            })
            .collect();
        Self {
            profile_path: String::new(),
            settings_file: String::new(),
            readings: BTreeMap::new(),
            edits: BTreeMap::new(),
            selected: Some(ControllerId::Z),
            forms,
            message: None,
            seen: (None, 0),
        }
    }
}

/// The form a controller's kind uses.
fn kind_key(id: ControllerId) -> &'static str {
    match id {
        ControllerId::Z => "z",
        ControllerId::PllAmplitude { .. } => "pll_amplitude",
        ControllerId::PllPhase { .. } => "pll_phase",
    }
}

impl ControllersTool {
    /// Every controller there is a form for: read, edited, or known.
    fn ids(&self) -> Vec<ControllerId> {
        let mut ids: Vec<ControllerId> = KNOWN.to_vec();
        for id in self.readings.keys().chain(self.edits.keys()) {
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
        ids
    }

    /// Take new `controller/read` events into the readings and the forms.
    fn take_readings(&mut self, view: &RunView) {
        let start = view.started_at();
        if start != self.seen.0 {
            self.seen = (start, 0);
        }
        let rows = view.custom(ControllerReading::KIND);
        for (_, data) in rows.iter().skip(self.seen.1) {
            if let Ok(reading) = serde_json::from_value::<ControllerReading>(data.clone()) {
                self.edits.insert(reading.id, reading.params.to_fields());
                self.readings.insert(reading.id, reading);
            }
        }
        self.seen.1 = rows.len();
    }

    /// The controllers whose form differs from the last reading, or every
    /// form when nothing has been read, as a profile.
    fn profile(&self) -> Result<ControllerProfile, String> {
        let mut controllers = Vec::new();
        for (id, fields) in &self.edits {
            let params = ControllerParams::from_fields(*id, fields.clone())
                .map_err(|e| format!("{id}: {e}"))?;
            let unchanged = self.readings.get(id).is_some_and(|r| r.params == params);
            if !unchanged {
                controllers.push(ProfileEntry { id: *id, params });
            }
        }
        Ok(ControllerProfile {
            settings_file: (!self.settings_file.trim().is_empty())
                .then(|| PathBuf::from(self.settings_file.trim())),
            controllers,
        })
    }

    /// Every form as a profile, for saving.
    fn whole_profile(&self) -> Result<ControllerProfile, String> {
        let mut controllers = Vec::new();
        for (id, fields) in &self.edits {
            let params = ControllerParams::from_fields(*id, fields.clone())
                .map_err(|e| format!("{id}: {e}"))?;
            controllers.push(ProfileEntry { id: *id, params });
        }
        Ok(ControllerProfile {
            settings_file: (!self.settings_file.trim().is_empty())
                .then(|| PathBuf::from(self.settings_file.trim())),
            controllers,
        })
    }

    fn load_profile(&mut self) {
        let loaded = std::fs::read_to_string(&self.profile_path)
            .map_err(|e| format!("Cannot read {}: {e}", self.profile_path))
            .and_then(|text| toml::from_str::<ControllerProfile>(&text).map_err(|e| e.to_string()))
            .and_then(|p| p.validate().map(|()| p));
        match loaded {
            Ok(profile) => {
                self.settings_file = profile
                    .settings_file
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                for entry in &profile.controllers {
                    self.edits.insert(entry.id, entry.params.to_fields());
                }
                if let Some(first) = profile.controllers.first() {
                    self.selected = Some(first.id);
                }
                self.message = Some(Note::ok(format!("Loaded {}", self.profile_path)));
            }
            Err(e) => self.message = Some(Note::err(e)),
        }
    }

    fn save_profile(&mut self) {
        if !self.profile_path.to_lowercase().ends_with(".toml") {
            self.profile_path.push_str(".toml");
        }
        let written = self
            .whole_profile()
            .and_then(|p| toml::to_string_pretty(&p).map_err(|e| e.to_string()))
            .and_then(|text| {
                std::fs::write(&self.profile_path, text)
                    .map_err(|e| format!("Cannot write {}: {e}", self.profile_path))
            });
        self.message = Some(match written {
            Ok(()) => Note::ok(format!("Saved {}", self.profile_path)),
            Err(e) => Note::err(e),
        });
    }

    /// The selected controller's form, its rows in the schema's order,
    /// with the Z-controller's `active` as a choice among the defined
    /// controllers once a reading has named them.
    fn render_form(&mut self, ui: &mut egui::Ui, id: ControllerId) -> bool {
        let Some(form) = self.forms.get(kind_key(id)) else {
            return false;
        };
        let available: Vec<String> = self
            .readings
            .get(&id)
            .map(|r| r.available.clone())
            .unwrap_or_default();
        let value = self
            .edits
            .entry(id)
            .or_insert_with(|| ControllerParams::default_for(id).to_fields());
        let mut changed = false;
        egui::Grid::new(format!("controller_form_{}", id.key()))
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                for key in form.keys() {
                    if key == "active" && !available.is_empty() {
                        ui.label("Active").on_hover_text(
                            "Which of the controllers Nanonis has defined runs; the input \
                             signal and log or linear input are part of that definition",
                        );
                        let current = value
                            .get("active")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        egui::ComboBox::from_id_salt("z_active")
                            .selected_text(if current.is_empty() {
                                "unchanged"
                            } else {
                                &current
                            })
                            .show_ui(ui, |ui| {
                                for name in &available {
                                    if ui.selectable_label(*name == current, name).clicked() {
                                        value["active"] = Value::String(name.clone());
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();
                    } else {
                        changed |= form.render_path(ui, value, &key);
                    }
                }
            });
        changed
    }
}

impl Tool for ControllersTool {
    fn id(&self) -> &'static str {
        "controllers"
    }

    fn label(&self) -> &str {
        "Controllers"
    }

    fn setup(&mut self, ui: &mut egui::Ui, cx: &mut SetupCx) {
        self.take_readings(cx.view);

        ui.horizontal(|ui| {
            ui.label("Profile");
            let width = (ui.available_width() - 150.0).max(120.0);
            let (_, picked) = path_field(ui, &mut self.profile_path, width, || {
                rfd::FileDialog::new()
                    .add_filter("TOML", &["toml"])
                    .pick_file()
            });
            if picked {
                self.load_profile();
            }
            if ui
                .add_enabled(!self.profile_path.is_empty(), egui::Button::new("Load"))
                .clicked()
            {
                self.load_profile();
            }
            if ui
                .add_enabled(!self.profile_path.is_empty(), egui::Button::new("Save"))
                .on_hover_text("Every form, with the settings file, as TOML")
                .clicked()
            {
                self.save_profile();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Settings file").on_hover_text(
                "Loaded into the controller before anything is written: where a \
                 controller's input signal and log or linear input are defined. \
                 Empty loads nothing.",
            );
            let width = (ui.available_width() - 40.0).max(120.0);
            path_field(ui, &mut self.settings_file, width, || {
                rfd::FileDialog::new()
                    .add_filter("Nanonis settings", &["ini"])
                    .pick_file()
            });
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(cx.can_run, egui::Button::new("Read from controller"))
                .on_hover_text("Fill every form with what the controller holds now")
                .on_disabled_hover_text("Connect first, and let any run finish")
                .clicked()
            {
                cx.run = Some(Box::new(ReadControllers));
            }
            if ui
                .add_enabled(cx.can_run, egui::Button::new("Apply"))
                .on_hover_text(
                    "Load the settings file, write every controller whose form differs \
                     from its last reading, and read everything back. Switches nothing \
                     on or off.",
                )
                .on_disabled_hover_text("Connect first, and let any run finish")
                .clicked()
            {
                match self.job() {
                    Ok(job) => cx.run = Some(job),
                    Err(e) => self.message = Some(Note::err(e)),
                }
            }
            if ui
                .add_enabled(!self.readings.is_empty(), egui::Button::new("Revert"))
                .on_hover_text("Put every form back to its last reading")
                .clicked()
            {
                for (id, reading) in &self.readings {
                    self.edits.insert(*id, reading.params.to_fields());
                }
                self.message = None;
            }
        });
        note(ui, &self.message);
        ui.add_space(6.0);

        ui.horizontal_wrapped(|ui| {
            for id in self.ids() {
                let marker = match self.readings.get(&id) {
                    Some(r) if r.enabled => " ●",
                    _ => "",
                };
                if ui
                    .selectable_label(self.selected == Some(id), format!("{id}{marker}"))
                    .clicked()
                {
                    self.selected = Some(id);
                }
            }
        });
        let Some(id) = self.selected else {
            return;
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| match self.readings.get(&id) {
                Some(r) => {
                    ui.label(format!(
                        "{}, {}",
                        r.status,
                        if r.enabled { "on" } else { "off" }
                    ));
                    if !r.available.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("defined: {}", r.available.join(", ")))
                                .weak(),
                        );
                    }
                    let (label, on) = if r.enabled {
                        ("Switch off", false)
                    } else {
                        ("Switch on", true)
                    };
                    if ui
                        .add_enabled(cx.can_run, egui::Button::new(label))
                        .on_hover_text("Switches this loop only; the forms are not written")
                        .clicked()
                    {
                        cx.run = Some(Box::new(SetControllerEnabled { id, on }));
                    }
                }
                None => {
                    ui.label(egui::RichText::new("not read yet").weak());
                }
            });
            ui.add_space(4.0);
            if self.render_form(ui, id) {
                self.message = None;
            }

            if let (Some(reading), Some(fields)) = (self.readings.get(&id), self.edits.get(&id)) {
                let diff = changed_fields(&reading.params.to_fields(), fields);
                ui.add_space(6.0);
                if diff.is_empty() {
                    ui.label(egui::RichText::new("matches the last reading").weak());
                } else {
                    ui.label(format!(
                        "{} field{} differ{} from the last reading: {}",
                        diff.len(),
                        if diff.len() == 1 { "" } else { "s" },
                        if diff.len() == 1 { "s" } else { "" },
                        diff.join(", ")
                    ));
                }
            }
        });
    }

    fn job(&self) -> Result<Box<dyn Job>, String> {
        let profile = self.profile()?;
        if profile.controllers.is_empty() && profile.settings_file.is_none() {
            return Err("nothing differs from the last reading, and no settings file".into());
        }
        Ok(Box::new(ApplyProfile { profile }))
    }

    fn panel(&mut self, ui: &mut egui::Ui, view: &RunView) {
        for (_, data) in view.custom(SettingsLoadedEvent::KIND) {
            if let Ok(e) = serde_json::from_value::<SettingsLoadedEvent>(data.clone()) {
                ui.label(format!("Loaded settings file {}", e.path));
            }
        }
        let applied: Vec<ControllerAppliedEvent> = view
            .custom(ControllerAppliedEvent::KIND)
            .iter()
            .filter_map(|(_, d)| serde_json::from_value(d.clone()).ok())
            .collect();
        if !applied.is_empty() {
            ui.label(egui::RichText::new("Applied").strong());
            egui::Grid::new("controllers_applied")
                .num_columns(4)
                .striped(true)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for e in &applied {
                        let before = e.before.to_fields();
                        let after = e.after.to_fields();
                        let schema = self.forms.get(kind_key(e.id));
                        let changed = changed_fields(&before, &after);
                        if changed.is_empty() {
                            ui.label(e.id.to_string());
                            ui.label(egui::RichText::new("no change").weak());
                            ui.label("");
                            ui.label("");
                            ui.end_row();
                        }
                        for key in changed {
                            ui.label(e.id.to_string());
                            ui.label(&key);
                            ui.label(field_text(schema, &key, &before[&key]));
                            ui.label(field_text(schema, &key, &after[&key]));
                            ui.end_row();
                        }
                    }
                });
        }
        let reads: BTreeMap<ControllerId, ControllerReading> = view
            .custom(ControllerReading::KIND)
            .iter()
            .filter_map(|(_, d)| serde_json::from_value::<ControllerReading>(d.clone()).ok())
            .map(|r| (r.id, r))
            .collect();
        if !reads.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Read").strong());
            egui::Grid::new("controllers_read")
                .num_columns(3)
                .striped(true)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for (id, r) in &reads {
                        ui.label(id.to_string());
                        ui.label(&r.status);
                        let schema = self.forms.get(kind_key(*id));
                        let fields = r.params.to_fields();
                        let summary: Vec<String> = fields
                            .as_object()
                            .map(|o| {
                                o.iter()
                                    .map(|(k, v)| format!("{k} {}", field_text(schema, k, v)))
                                    .collect()
                            })
                            .unwrap_or_default();
                        ui.label(summary.join(", "));
                        ui.end_row();
                    }
                });
        }
    }

    fn prefs(&self) -> Value {
        serde_json::json!({
            "profile_path": self.profile_path,
            "settings_file": self.settings_file,
            "edits": self.edits,
            "selected": self.selected,
        })
    }

    fn restore(&mut self, prefs: &Value) {
        if let Some(p) = prefs.get("profile_path").and_then(Value::as_str) {
            self.profile_path = p.to_string();
        }
        if let Some(p) = prefs.get("settings_file").and_then(Value::as_str) {
            self.settings_file = p.to_string();
        }
        if let Some(edits) = prefs
            .get("edits")
            .and_then(|e| serde_json::from_value::<BTreeMap<ControllerId, Value>>(e.clone()).ok())
        {
            // Only fields that still deserialize are worth keeping.
            for (id, fields) in edits {
                if ControllerParams::from_fields(id, fields.clone()).is_ok() {
                    self.edits.insert(id, fields);
                }
            }
        }
        if let Some(selected) = prefs
            .get("selected")
            .and_then(|s| serde_json::from_value(s.clone()).ok())
        {
            self.selected = selected;
        }
    }
}

/// The keys whose values differ between two flat field objects.
fn changed_fields(a: &Value, b: &Value) -> Vec<String> {
    let (Some(a), Some(b)) = (a.as_object(), b.as_object()) else {
        return Vec::new();
    };
    a.iter()
        .filter(|(k, v)| b.get(*k) != Some(v))
        .map(|(k, _)| k.clone())
        .collect()
}

/// A field's value for a table: numbers in the unit the schema gives
/// them, the rest as JSON.
fn field_text(form: Option<&SchemaForm>, key: &str, value: &Value) -> String {
    let unit = form.and_then(|f| f.unit_of(key));
    match (value.as_f64(), unit) {
        (Some(v), Some(unit)) => format_si(v, unit),
        (Some(v), None) => crate::units::number(v),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tip::controllers::ZControllerParams;
    use rusty_tip::event::Event;

    fn reading(id: ControllerId, params: ControllerParams) -> Event {
        Event::typed(&ControllerReading {
            id,
            params,
            enabled: true,
            status: "on".into(),
            available: vec!["Current log".into(), "Freq shift".into()],
        })
    }

    #[test]
    fn a_read_fills_the_forms_and_apply_writes_only_what_differs() {
        let mut tool = ControllersTool::default();
        let mut view = RunView::default();
        let z = ControllerParams::Z(ZControllerParams {
            active: "Current log".into(),
            setpoint: 50e-12,
            ..Default::default()
        });
        view.apply_event(&reading(ControllerId::Z, z.clone()));
        view.apply_event(&reading(
            ControllerId::PllPhase { modulator: 1 },
            ControllerParams::default_for(ControllerId::PllPhase { modulator: 1 }),
        ));
        tool.take_readings(&view);
        assert_eq!(tool.readings.len(), 2);
        assert_eq!(tool.edits[&ControllerId::Z]["setpoint"], 50e-12);
        assert!(
            tool.job().is_err(),
            "nothing differs, so there is nothing to apply"
        );

        tool.edits.get_mut(&ControllerId::Z).unwrap()["setpoint"] = serde_json::json!(80e-12);
        let profile = tool.profile().unwrap();
        assert_eq!(
            profile.controllers.len(),
            1,
            "only the Z-controller changed"
        );
        assert_eq!(profile.controllers[0].id, ControllerId::Z);
        assert!(tool.job().is_ok());

        // The same events again are not taken twice; a new run's are.
        tool.take_readings(&view);
        assert_eq!(tool.edits[&ControllerId::Z]["setpoint"], 80e-12);
        let mut fresh = RunView::default();
        fresh.apply_event(&reading(ControllerId::Z, z));
        tool.take_readings(&fresh);
        assert_eq!(tool.edits[&ControllerId::Z]["setpoint"], 50e-12);
    }

    #[test]
    fn changed_fields_and_units() {
        let a = serde_json::json!({"p_gain_m": 1e-11, "setpoint": 1.0});
        let b = serde_json::json!({"p_gain_m": 2e-11, "setpoint": 1.0});
        assert_eq!(changed_fields(&a, &b), vec!["p_gain_m"]);
        let tool = ControllersTool::default();
        let form = tool.forms.get("z");
        assert_eq!(
            field_text(form, "p_gain_m", &serde_json::json!(2e-11)),
            "20.00 pm"
        );
        assert_eq!(
            field_text(form, "setpoint", &serde_json::json!(1e-10)),
            "1.000e-10"
        );
    }
}
