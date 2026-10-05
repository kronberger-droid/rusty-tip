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
use std::path::{Path, PathBuf};

use eframe::egui;
use serde_json::Value;

use rusty_tip::controllers::{
    ApplyPreset, ApplyProfile, ControllerAppliedEvent, ControllerId, ControllerParams,
    ControllerProfile, ControllerReading, Preset, PresetStore, ProfileEntry, ReadControllers,
    SetControllerEnabled, TomlPresetStore, TunedAt, ZLaw, ZLoopInput, ZQuantity,
};
use rusty_tip::experiment_log::LogEvent;
use rusty_tip::routine::SettingsLoadedEvent;
use rusty_tip::session::{Job, Readout};

use super::{SetupCx, Tool, load_toml, save_toml};
use crate::form::{SchemaForm, number_field};
use crate::run_view::RunView;
use crate::units::{display_unit_for, format_si, number, prefix_scale};
use crate::widgets::{Note, Palette, StripChart, Tone, note, path_field, section, toned_if};

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
    /// The presets on file, and which file that was.
    presets: Vec<Preset>,
    presets_path: Option<PathBuf>,
    /// The preset picked in the drop-down, by name.
    selected_preset: Option<String>,
    /// The name and note for "Save form as".
    preset_name: String,
    preset_note: String,
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
            presets: Vec::new(),
            presets_path: None,
            selected_preset: None,
            preset_name: String::new(),
            preset_note: String::new(),
        }
    }
}

impl ControllersTool {
    /// Read the preset file again, remembering which file it was.
    fn refresh_presets(&mut self, path: &Path) {
        match TomlPresetStore::new(path).list() {
            Ok(presets) => self.presets = presets,
            Err(e) => {
                self.presets.clear();
                self.message = Some(Note::err(e));
            }
        }
        self.presets_path = Some(path.to_path_buf());
    }

    /// Put a preset's gains into its controller's form. Gains only, as
    /// Apply writes them: the setpoint in the form is the run's, not the
    /// preset's.
    fn load_preset(&mut self, preset: &Preset) {
        let id = preset.id;
        let form = self
            .edits
            .get(&id)
            .and_then(|f| ControllerParams::from_fields(id, f.clone()).ok());
        let params = match &form {
            Some(form) => preset.params_over(form),
            None => preset.params.clone(),
        };
        self.edits.insert(id, params.to_fields());
        self.message = Some(Note::ok(format!(
            "Preset {:?} is in the form, setpoint kept",
            preset.name
        )));
    }

    /// The picked preset, when it is one of this controller's.
    fn picked_preset(&self, id: ControllerId) -> Option<Preset> {
        let name = self.selected_preset.as_deref()?;
        self.presets
            .iter()
            .find(|p| p.id == id && p.is_named(name))
            .cloned()
    }

    /// This controller's form as a preset under `preset_name`, tuned at
    /// the form's setpoint, the live bias, and the amplitude loop's
    /// setpoint as last read.
    fn save_preset(&mut self, path: &Path, id: ControllerId, readouts: &[Readout]) {
        let fields = self
            .edits
            .get(&id)
            .cloned()
            .unwrap_or_else(|| ControllerParams::default_for(id).to_fields());
        let params = match ControllerParams::from_fields(id, fields) {
            Ok(p) => p,
            Err(e) => {
                self.message = Some(Note::err(format!("{id}: {e}")));
                return;
            }
        };
        let tuned_at = TunedAt {
            setpoint: params.setpoint(),
            bias_v: readouts.iter().find(|r| r.key == "bias").map(|r| r.value),
            amplitude_m: self
                .readings
                .get(&ControllerId::PllAmplitude { modulator: 1 })
                .and_then(|r| r.params.setpoint()),
            note: self.preset_note.trim().to_string(),
        };
        let preset = Preset {
            name: self.preset_name.trim().to_string(),
            id,
            params,
            tuned_at,
        };
        let mut store = TomlPresetStore::new(path);
        match store.put(preset.clone()) {
            Ok(()) => {
                self.message = Some(Note::ok(format!(
                    "Saved preset {:?} to {}",
                    preset.name,
                    path.display()
                )));
                self.selected_preset = Some(preset.name);
                self.refresh_presets(path);
            }
            Err(e) => self.message = Some(Note::err(e)),
        }
    }

    /// The preset row for the selected controller: pick one, load it into
    /// the form, apply it, delete it; and save the form as one.
    fn render_presets(&mut self, ui: &mut egui::Ui, cx: &mut SetupCx<'_>, id: ControllerId) {
        let path = match &cx.connection {
            Ok(s) => s.presets_file.clone(),
            Err(_) => PathBuf::from(rusty_tip::config::DEFAULT_PRESETS_FILE),
        };
        if self.presets_path.as_deref() != Some(path.as_path()) {
            self.refresh_presets(&path);
        }
        let names: Vec<String> = self
            .presets
            .iter()
            .filter(|p| p.id == id)
            .map(|p| p.name.clone())
            .collect();
        let chosen = self.picked_preset(id);

        section(
            ui,
            "Presets",
            Some(&format!(
                "This loop's saved parameter sets, from {}",
                path.display()
            )),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Preset")
                .on_hover_text(format!("From {}", path.display()));
            let shown = match &chosen {
                Some(p) => p.name.clone(),
                None if names.is_empty() => "none saved".to_string(),
                None => "pick one".to_string(),
            };
            egui::ComboBox::from_id_salt(format!("preset_{id}"))
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for name in &names {
                        let picked = chosen.as_ref().is_some_and(|p| &p.name == name);
                        if ui.selectable_label(picked, name).clicked() {
                            self.selected_preset = Some(name.clone());
                        }
                    }
                });
            if ui
                .add_enabled(chosen.is_some(), egui::Button::new("Load into form"))
                .on_hover_text(
                    "The form takes the preset's gains and keeps the setpoint it has; \
                     nothing is written",
                )
                .clicked()
                && let Some(p) = &chosen
            {
                self.load_preset(p);
            }
            if ui
                .add_enabled(
                    chosen.is_some() && cx.can_run,
                    toned_if(
                        ui,
                        chosen.is_some() && cx.can_run,
                        "Apply preset",
                        Tone::Write,
                    ),
                )
                .on_hover_text(
                    "Write the preset's gains, keeping the setpoint the loop holds, and read \
                     it back. Switches nothing on or off.",
                )
                .on_disabled_hover_text("Pick a preset, connect, and let any run finish")
                .clicked()
                && let Some(p) = chosen.clone()
            {
                cx.run = Some(Box::new(ApplyPreset { preset: p }));
            }
            if ui
                .add_enabled(
                    chosen.is_some(),
                    toned_if(ui, chosen.is_some(), "Delete", Tone::Danger),
                )
                .on_hover_text("Remove it from the preset file")
                .clicked()
                && let Some(p) = &chosen
            {
                match TomlPresetStore::new(&path).remove(&p.name) {
                    Ok(()) => {
                        self.message = Some(Note::ok(format!("Deleted preset {:?}", p.name)));
                        self.selected_preset = None;
                    }
                    Err(e) => self.message = Some(Note::err(e)),
                }
                self.refresh_presets(&path);
            }
            if ui
                .button("Refresh presets")
                .on_hover_text("Read the preset file again")
                .clicked()
            {
                self.refresh_presets(&path);
            }
        });
        if let Some(p) = &chosen {
            ui.label(egui::RichText::new(tuned_at_line(p)).weak());
        }
        ui.horizontal(|ui| {
            ui.label("Save form as");
            ui.add(
                egui::TextEdit::singleline(&mut self.preset_name)
                    .desired_width(140.0)
                    .hint_text("name"),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.preset_note)
                    .desired_width(220.0)
                    .hint_text("note: sample, tip state"),
            );
            if ui
                .add_enabled(
                    !self.preset_name.trim().is_empty(),
                    egui::Button::new("Save preset"),
                )
                .on_hover_text(
                    "This form's parameters under that name, with the setpoint, bias and \
                     amplitude they were tuned at. A preset of that name is replaced.",
                )
                .clicked()
            {
                self.save_preset(&path, id, cx.readouts);
            }
        });
    }
}

/// One line on where a preset was tuned, and whether that matters.
fn tuned_at_line(p: &Preset) -> String {
    let t = &p.tuned_at;
    let mut parts = Vec::new();
    if let Some(sp) = t.setpoint {
        let unit = match &p.params {
            ControllerParams::Z(z) => z.input().unit(),
            ControllerParams::PllAmplitude(_) => Some("m"),
            ControllerParams::PllPhase(_) => None,
        };
        let shown = unit.map_or_else(|| number(sp), |u| format_si(sp, u));
        parts.push(format!("setpoint {shown}"));
    }
    if let Some(b) = t.bias_v {
        parts.push(format!("bias {}", format_si(b, "V")));
    }
    if let Some(a) = t.amplitude_m {
        parts.push(format!("amplitude {}", format_si(a, "m")));
    }
    let mut line = if parts.is_empty() {
        "tuned at: not recorded".to_string()
    } else {
        format!("tuned at {}", parts.join(", "))
    };
    if !t.note.is_empty() {
        line.push_str(" · ");
        line.push_str(&t.note);
    }
    line.push_str(if p.depends_on_operating_point() {
        " (the gains depend on this)"
    } else {
        " (a log current loop: the gains transfer)"
    });
    line
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

    /// The forms as a profile: every one for saving, or only those that
    /// differ from the last reading (every one when nothing has been
    /// read) for applying.
    fn profile(&self, only_changed: bool) -> Result<ControllerProfile, String> {
        let mut controllers = Vec::new();
        for (id, fields) in &self.edits {
            let params = ControllerParams::from_fields(*id, fields.clone())
                .map_err(|e| format!("{id}: {e}"))?;
            let unchanged = self.readings.get(id).is_some_and(|r| r.params == params);
            if !(only_changed && unchanged) {
                controllers.push(ProfileEntry { id: *id, params });
            }
        }
        let settings_file = self.settings_file.trim();
        Ok(ControllerProfile {
            settings_file: (!settings_file.is_empty()).then(|| PathBuf::from(settings_file)),
            controllers,
        })
    }

    fn load_profile(&mut self) {
        let loaded = load_toml::<ControllerProfile>(&self.profile_path)
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
        let written = self
            .profile(false)
            .and_then(|p| save_toml(&mut self.profile_path, &p));
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
        egui::Grid::new(format!("controller_form_{id}"))
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
                    } else if key == "setpoint" && id == ControllerId::Z {
                        // The setpoint's unit is the active loop's input,
                        // which only its name says.
                        let input = ZLoopInput::from_name(
                            value.get("active").and_then(Value::as_str).unwrap_or(""),
                        );
                        let label = match input.law {
                            ZLaw::Log => "Setpoint (log loop)",
                            ZLaw::Linear => "Setpoint",
                        };
                        ui.label(label).on_hover_text(
                            "In the active loop's input signal; a log loop forms its error \
                             from the ratio to this, a linear one from the difference",
                        );
                        let unit = input.unit();
                        changed |= number_field(
                            ui,
                            &mut value["setpoint"],
                            unit,
                            unit.map(display_unit_for),
                        );
                        ui.end_row();
                    } else {
                        changed |= form.render_path(ui, value, &key);
                    }
                }
            });
        changed
    }
}

/// How far back the live charts look.
const CHART_WINDOW_S: f64 = 10.0;
const CHART_HEIGHT: f32 = 120.0;

impl ControllersTool {
    /// The Z loop live: its input against the setpoint the form holds,
    /// and Z, from the last seconds of the stream. Drawn from the
    /// session's idle tap, so it stands still while a job runs.
    fn render_z_chart(&self, ui: &mut egui::Ui, cx: &SetupCx<'_>) {
        if cx.readouts.is_empty() {
            ui.label(egui::RichText::new("Connect to see the loop live").weak());
            return;
        }
        let fields = self.edits.get(&ControllerId::Z);
        let active = fields
            .and_then(|f| f.get("active"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let input = ZLoopInput::from_name(active);
        // By the registry name the session resolved, as the pane shows it.
        let readout = |key: &str| cx.readouts.iter().find(|r| r.key == key).map(|r| r.index.0);
        let input_signal = match input.quantity {
            ZQuantity::Frequency => readout("freq shift"),
            ZQuantity::Current => readout("current"),
            _ => None,
        };
        let (Some(input_index), Some(z_index)) = (input_signal, readout("z")) else {
            ui.label(
                egui::RichText::new("The loop's input or Z is not among the connection's signals")
                    .weak(),
            );
            return;
        };
        if !cx.samples.has(input_index) && !cx.samples.has(z_index) {
            ui.label(egui::RichText::new("Waiting for the stream").weak());
            return;
        }
        let unit = input.unit().unwrap_or("");
        let display = display_unit_for(unit);
        let scale = prefix_scale(display).unwrap_or(1.0);
        let setpoint = fields
            .and_then(|f| f.get("setpoint"))
            .and_then(Value::as_f64)
            .map(|s| s * scale);
        let gap = ui.spacing().item_spacing.x;
        let half = ((ui.available_width() - gap) / 2.0).max(200.0);
        let colors = Palette::for_theme(ui.visuals().dark_mode);
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.set_width(half);
                let latest = cx
                    .samples
                    .latest(input_index)
                    .map(|v| format_si(v, unit))
                    .unwrap_or_default();
                ui.label(format!("{active}: {latest}"));
                StripChart {
                    id: "z_loop_input",
                    unit: display,
                    setpoint,
                    window_s: CHART_WINDOW_S,
                    size: [half, CHART_HEIGHT],
                    color: colors.first,
                }
                .show(ui, cx.samples.points(input_index, scale));
            });
            ui.vertical(|ui| {
                ui.set_width(half);
                let latest = cx
                    .samples
                    .latest(z_index)
                    .map(|v| format_si(v, "m"))
                    .unwrap_or_default();
                ui.label(format!("Z: {latest}"));
                StripChart {
                    id: "z_loop_z",
                    unit: "nm",
                    setpoint: None,
                    window_s: CHART_WINDOW_S,
                    size: [half, CHART_HEIGHT],
                    color: colors.second,
                }
                .show(ui, cx.samples.points(z_index, 1e9));
            });
        });
    }
}

impl Tool for ControllersTool {
    fn id(&self) -> &'static str {
        "controllers"
    }

    fn label(&self) -> &str {
        "Controllers"
    }

    /// Every job here starts from a Setup button; the last one's result
    /// shows under the setup.
    fn has_run_tab(&self) -> bool {
        false
    }

    fn setup(&mut self, ui: &mut egui::Ui, cx: &mut SetupCx) {
        self.take_readings(cx.view);

        section(
            ui,
            "Profile",
            Some(
                "Every loop's form and the settings file, saved together as one TOML file \
                 to load again later",
            ),
        );
        ui.horizontal(|ui| {
            ui.label("Profile file");
            let width = (ui.available_width() - 190.0).max(120.0);
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
            let width = (ui.available_width() - 60.0).max(120.0);
            path_field(ui, &mut self.settings_file, width, || {
                rfd::FileDialog::new()
                    .add_filter("Nanonis settings", &["ini"])
                    .pick_file()
            });
        });

        section(
            ui,
            "All loops",
            Some("Read every loop from the controller, or write every form that differs"),
        );
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
                .add_enabled(
                    cx.can_run,
                    toned_if(ui, cx.can_run, "Apply to controller", Tone::Write),
                )
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

        section(ui, "Loop", None);
        ui.horizontal_wrapped(|ui| {
            for id in self.ids() {
                // Words, not a dot: the UI font has no circle glyph.
                let marker = match self.readings.get(&id) {
                    Some(r) if r.enabled => " (on)",
                    _ => "",
                };
                let text = egui::RichText::new(format!("{id}{marker}")).size(15.0);
                if ui
                    .selectable_label(self.selected == Some(id), text)
                    .clicked()
                {
                    self.selected = Some(id);
                }
            }
        });
        let Some(id) = self.selected else {
            return;
        };
        ui.add_space(4.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| match self.readings.get(&id) {
                Some(r) => {
                    let colors = Palette::for_theme(ui.visuals().dark_mode);
                    let (word, color) = if r.enabled {
                        ("ON", colors.bounds.to_opaque())
                    } else {
                        ("OFF", ui.visuals().weak_text_color())
                    };
                    let mut about = format!("Controller status: {}", r.status);
                    if !r.available.is_empty() {
                        about.push_str(&format!("\nInputs defined: {}", r.available.join(", ")));
                    }
                    ui.label(egui::RichText::new(word).strong().size(16.0).color(color))
                        .on_hover_text(about);
                    let (label, on) = if r.enabled {
                        ("Switch off", false)
                    } else {
                        ("Switch on", true)
                    };
                    let tone = if on { Tone::Write } else { Tone::Danger };
                    if ui
                        .add_enabled(cx.can_run, toned_if(ui, cx.can_run, label, tone))
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
            self.render_presets(ui, cx, id);
            section(ui, "Parameters", None);
            if self.render_form(ui, id) {
                self.message = None;
            }
            if id == ControllerId::Z {
                ui.add_space(6.0);
                self.render_z_chart(ui, cx);
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
        let profile = self.profile(true)?;
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
        // A JSON object needs string keys, so the edits go as pairs.
        let edits: Vec<(&ControllerId, &Value)> = self.edits.iter().collect();
        serde_json::json!({
            "profile_path": self.profile_path,
            "settings_file": self.settings_file,
            "edits": edits,
            "selected": self.selected,
            "selected_preset": self.selected_preset,
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
            .and_then(|e| serde_json::from_value::<Vec<(ControllerId, Value)>>(e.clone()).ok())
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
        if let Some(name) = prefs.get("selected_preset").and_then(Value::as_str) {
            self.selected_preset = Some(name.to_string());
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
            available: vec!["log Current".into(), "Frequency (neg)".into()],
        })
    }

    #[test]
    fn loading_a_preset_keeps_the_forms_setpoint() {
        let mut tool = ControllersTool::default();
        let mut view = RunView::default();
        view.apply_event(reading(
            ControllerId::Z,
            ControllerParams::Z(ZControllerParams {
                setpoint: 80e-12,
                ..Default::default()
            }),
        ));
        tool.take_readings(&view);
        let preset = Preset {
            name: "tip-prep".into(),
            id: ControllerId::Z,
            params: ControllerParams::Z(ZControllerParams {
                setpoint: 1e-9,
                p_gain_m: 1.5e-12,
                ..Default::default()
            }),
            tuned_at: TunedAt {
                setpoint: Some(1e-9),
                bias_v: None,
                amplitude_m: None,
                note: String::new(),
            },
        };

        tool.load_preset(&preset);
        assert_eq!(tool.edits[&ControllerId::Z]["setpoint"], 80e-12);
        assert_eq!(tool.edits[&ControllerId::Z]["p_gain_m"], 1.5e-12);
    }

    #[test]
    fn a_read_fills_the_forms_and_apply_writes_only_what_differs() {
        let mut tool = ControllersTool::default();
        let mut view = RunView::default();
        let z = ControllerParams::Z(ZControllerParams {
            active: "log Current".into(),
            setpoint: 50e-12,
            ..Default::default()
        });
        view.apply_event(reading(ControllerId::Z, z.clone()));
        view.apply_event(reading(
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
        let profile = tool.profile(true).unwrap();
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
        fresh.apply_event(reading(ControllerId::Z, z));
        tool.take_readings(&fresh);
        assert_eq!(tool.edits[&ControllerId::Z]["setpoint"], 50e-12);
    }

    #[test]
    fn prefs_round_trip_with_the_edits() {
        let mut tool = ControllersTool::default();
        tool.edits.insert(
            ControllerId::PllPhase { modulator: 1 },
            ControllerParams::default_for(ControllerId::PllPhase { modulator: 1 }).to_fields(),
        );
        tool.selected = Some(ControllerId::PllPhase { modulator: 1 });
        tool.profile_path = "loops.toml".into();
        let prefs = tool.prefs();
        assert!(
            prefs["edits"].is_array(),
            "ids are not strings, so not a map"
        );

        let mut back = ControllersTool::default();
        back.restore(&prefs);
        assert_eq!(back.edits, tool.edits);
        assert_eq!(back.selected, tool.selected);
        assert_eq!(back.profile_path, "loops.toml");
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
