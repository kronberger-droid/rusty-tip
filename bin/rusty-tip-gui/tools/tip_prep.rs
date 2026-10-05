//! Tip preparation as a workbench tool.
//!
//! Setup is a form drawn from `AppConfig`'s JSON Schema, with the settings
//! that decide a run at the top and the rest in sections below, plus load
//! and save as TOML. The Run panel is the old Control tab: tip state, the
//! frequency-shift trace with the sharp band, and the pulse voltage
//! history.

use std::path::PathBuf;

use eframe::egui;
use egui_plot::{AxisHints, Bar, BarChart, HLine, Line, Plot, PlotPoints, Points};

use rusty_tip::config::AppConfig;
use rusty_tip::controllers::{ControllerId, Preset, PresetStore, TomlPresetStore};
use rusty_tip::experiment_log::{LogEvent, ToolSchema};
use rusty_tip::routine::{Outcome, run_routine};
use rusty_tip::session::{Job, JobCx, NanonisBackend};
use rusty_tip::spm_error::SpmError;
use rusty_tip::tip_prep::reload::FROZEN;
use rusty_tip::tip_prep::{
    ConfigReload, ConfigReloadedEvent, CycleEvent, MaxPulseEvent, TipPrep, TipPrepSignals,
};
use serde::Deserialize;

use super::{SetupCx, Tool, load_toml, save_toml};
use crate::connection::ConnectionSettings;
use crate::form::SchemaForm;
use crate::run_view::RunView;
use crate::units::{format_si, format_tick, number};
use crate::widgets::{Note, Palette, Y_AXIS_WIDTH, note, path_field, section, stat};

/// Tip prep as a [`Job`]: what the session runs.
pub struct TipPrepJob {
    pub config: AppConfig,
    /// The Run panel's reload button sends into this.
    pub reload: ConfigReload,
}

impl Job for TipPrepJob {
    fn name(&self) -> &str {
        "tip_prep"
    }

    fn log_schema(&self) -> ToolSchema {
        rusty_tip::tip_prep::log_schema()
    }

    fn header_config(&self) -> serde_json::Value {
        serde_json::to_value(&self.config).unwrap_or(serde_json::Value::Null)
    }

    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        let signals = TipPrepSignals::resolve(cx.registry)?;
        let mut routine = TipPrep::new(&self.config, signals.freq_shift, signals.current)
            .with_reload(self.reload.clone());
        let _live = Live::open(&self.reload);
        run_routine(cx.controller, cx.events, cx.shutdown, &mut routine)
    }
}

/// The reload mailbox marked live for as long as this lives, a panic
/// included.
struct Live<'a>(&'a ConfigReload);

impl<'a> Live<'a> {
    fn open(reload: &'a ConfigReload) -> Self {
        reload.set_live(true);
        Self(reload)
    }
}

impl Drop for Live<'_> {
    fn drop(&mut self) {
        self.0.set_live(false);
    }
}

/// Series the panel reads off the [`RunView`], by the names the log uses.
const FREQ_SHIFT_SERIES: &str = "stable_read.value";
const FREQ_SHIFT_STABLE: &str = "stable_read.stable";
const CYCLE: &str = "tip_prep/cycle.cycle";
const CYCLE_FREQ_SHIFT: &str = "tip_prep/cycle.freq_shift";
const CYCLE_SHARP: &str = "tip_prep/cycle.is_sharp";

/// Radius of the per-measurement markers. Both series are discrete
/// measurements, so they are drawn as points with a connecting line; a bare
/// `Line` renders nothing until the second point arrives.
const MARKER_RADIUS: f32 = 2.5;

/// The config tables the Connection page owns, plus `console`, which only
/// the CLI reads. Not drawn here; the connection tables are written from
/// the page on save and offered to the page on load, `console` is kept as
/// the file had it.
const CONNECTION_SECTIONS: &[&str] = &[
    "nanonis",
    "data_acquisition",
    "experiment_logging",
    "console",
    "tcp_channel_mapping",
];

/// The fields that decide a run, shown above everything else, in order.
const FEATURED: &[&str] = &[
    "tip_prep.sharp_tip_bounds",
    "pulse_method",
    "tip_prep.max_cycles",
    "tip_prep.max_duration_secs",
    "tip_prep.stability.check_stability",
    "tip_prep.stability.stable_tip_allowed_change",
    "tip_prep.initial_bias_v",
    "tip_prep.initial_z_setpoint_a",
    "tip_prep.safe_tip_threshold",
];

pub struct TipPrepTool {
    /// The config file the form was loaded from, or will be saved to.
    path: String,
    /// The config as the form edits it: JSON, SI units, the shape of
    /// `AppConfig`.
    value: serde_json::Value,
    form: SchemaForm,
    message: Option<Note>,
    /// The Z presets in the connection's preset file, by name, and which
    /// file at which modification time they were read from.
    z_presets: Vec<Preset>,
    presets_read: Option<(PathBuf, Option<std::time::SystemTime>)>,
    /// Shared with every job this tool starts; live while one runs.
    reload: ConfigReload,
    /// What the last press of the Run panel's reload button came to.
    reload_note: Option<Note>,
}

impl Default for TipPrepTool {
    fn default() -> Self {
        let schema = serde_json::to_value(schemars::schema_for!(AppConfig))
            .expect("the config schema serializes");
        Self {
            path: String::new(),
            value: serde_json::to_value(AppConfig::default()).unwrap_or(serde_json::Value::Null),
            form: SchemaForm::new(schema),
            message: None,
            z_presets: Vec::new(),
            presets_read: None,
            reload: ConfigReload::new(),
            reload_note: None,
        }
    }
}

impl TipPrepTool {
    fn set_config(&mut self, config: &AppConfig) {
        self.value = serde_json::to_value(config).unwrap_or(serde_json::Value::Null);
    }

    /// The form's value as a config, validated the way the CLI validates a
    /// file.
    fn parse(&self) -> Result<AppConfig, String> {
        let config: AppConfig =
            serde_json::from_value(self.value.clone()).map_err(|e| e.to_string())?;
        config.validate().map_err(|e| e.to_string())?;
        Ok(config)
    }

    fn validate(&mut self) {
        self.message = Some(match self.parse() {
            Ok(_) => Note::ok("Config is valid"),
            Err(e) => Note::err(e),
        });
    }

    /// Read the file into the form and offer its connection tables to the
    /// Connection page.
    fn load(&mut self, cx: &mut SetupCx<'_>) {
        match load_toml::<AppConfig>(&self.path) {
            Ok(config) => {
                cx.import = Some(connection_of(&config));
                self.set_config(&config);
                self.message = Some(Note::ok(format!("Loaded {}", self.path)));
            }
            Err(e) => self.message = Some(Note::err(e)),
        }
    }

    /// Write the file with the Connection page's settings in its
    /// connection tables, so the same file drives the CLI.
    fn save(&mut self, connection: &Result<ConnectionSettings, String>) {
        let written = connection
            .clone()
            .map_err(|e| format!("Fix the Connection page first: {e}"))
            .and_then(|connection| {
                let mut config = self.parse()?;
                set_connection(&mut config, &connection);
                Ok(config)
            })
            .and_then(|config| save_toml(&mut self.path, &config));
        self.message = Some(match written {
            Ok(()) => Note::ok(format!("Saved {}", self.path)),
            Err(e) => Note::err(e),
        });
    }

    /// The sharp window the run is judging on, for the plot: its last
    /// reload's, else the one it started with, else the form's.
    fn sharp_bounds(&self, view: &RunView) -> Option<(f64, f64)> {
        let config = last_reload(view)
            .map(|r| r.config)
            .or_else(|| view.header.as_ref().map(|h| h.config.clone()))
            .filter(|c| !c.is_null());
        let b = config.as_ref().unwrap_or(&self.value);
        let b = b.get("tip_prep")?.get("sharp_tip_bounds")?;
        Some((b.get(0)?.as_f64()?, b.get(1)?.as_f64()?))
    }

    /// Hand the Setup form to the running run, which takes it before its
    /// next cycle.
    fn send_reload(&mut self) {
        self.reload_note = match self.parse() {
            Ok(config) => {
                self.reload.send(config);
                None
            }
            Err(e) => Some(Note::err(format!("Not sent: {e}"))),
        };
    }

    /// The Apply to run button, for the row beside Start and Stop, and a
    /// word on what became of the last press.
    fn render_apply(&mut self, ui: &mut egui::Ui, view: &RunView) {
        let live = self.reload.is_live();
        if ui
            .add_enabled(live, egui::Button::new("Apply to run"))
            .on_hover_text(
                "Send the Setup form to the run. It switches before its next pulse, \
                 never during a stability check, and writes nothing to the controller: \
                 the setpoint, initial bias, Z preset, safe-tip threshold and connection \
                 tables stay as the run started. Edited the file instead? Press Re-read \
                 file on Setup first.",
            )
            .on_disabled_hover_text("Only while a tip-prep run is going")
            .clicked()
        {
            self.send_reload();
        }
        if let Some(n) = &self.reload_note {
            ui.colored_label(egui::Color32::RED, "not sent")
                .on_hover_text(&n.text);
        } else if live && self.reload.is_pending() {
            ui.label(egui::RichText::new("waiting for the cycle to end").weak());
        } else if let Some(r) = last_reload(view) {
            ui.label(egui::RichText::new(format!("applied after cycle {}", r.after_cycle)).weak());
            if !r.kept.is_empty() {
                let warn = Palette::for_theme(ui.visuals().dark_mode).second;
                ui.colored_label(warn, format!("{} kept", r.kept.len()))
                    .on_hover_text(format!(
                        "Kept as the run started, applied from the next run:\n{}",
                        r.kept.join("\n")
                    ));
            }
        }
    }
}

/// The voltage of the last pulse fired, cycle or max, sign included.
fn fired_pulse(view: &RunView) -> Option<f64> {
    view.last_started
        .get("bias_pulse")?
        .1
        .get("voltage")?
        .as_f64()
}

/// The run's last switch to a config sent while it ran.
fn last_reload(view: &RunView) -> Option<ConfigReloadedEvent> {
    view.custom(ConfigReloadedEvent::KIND)
        .last()
        .and_then(|(_, data)| ConfigReloadedEvent::deserialize(data).ok())
}

impl TipPrepTool {
    /// The Z presets in the connection's preset file, re-read whenever the
    /// file changes, so one saved on the Controllers page shows up here.
    fn refresh_z_presets(&mut self, connection: &Result<ConnectionSettings, String>) {
        let path = match connection {
            Ok(s) => s.presets_file.clone(),
            Err(_) => PathBuf::from(rusty_tip::config::DEFAULT_PRESETS_FILE),
        };
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if self.presets_read.as_ref() == Some(&(path.clone(), modified)) {
            return;
        }
        self.z_presets = TomlPresetStore::new(&path)
            .list()
            .map(|presets| {
                presets
                    .into_iter()
                    .filter(|p| p.id == ControllerId::Z)
                    .collect()
            })
            .unwrap_or_default();
        self.presets_read = Some((path, modified));
    }

    /// `tip_prep.z_controller_preset` as a pick of the file's Z presets,
    /// one row of the featured grid. Returns whether it changed.
    fn render_z_preset(
        &mut self,
        ui: &mut egui::Ui,
        connection: &Result<ConnectionSettings, String>,
        enabled: bool,
    ) -> bool {
        self.refresh_z_presets(connection);
        let current = self.value["tip_prep"]["z_controller_preset"]
            .as_str()
            .map(str::to_string);
        let mut picked = current.clone();
        ui.add_enabled(enabled, egui::Label::new("Z preset"))
            .on_hover_text(
                "Written to the Z-controller before the first approach, with the run's \
                 setpoint. None runs on whatever the loop holds.",
            );
        let shown = match &current {
            None => "none, the loop as it is".to_string(),
            Some(name) if !self.z_presets.iter().any(|p| p.is_named(name)) => {
                format!("{name} (not in the preset file)")
            }
            Some(name) => name.clone(),
        };
        ui.add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt("tip_prep_z_preset")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut picked, None, "none, the loop as it is");
                    for preset in &self.z_presets {
                        ui.selectable_value(&mut picked, Some(preset.name.clone()), &preset.name);
                    }
                });
        });
        ui.end_row();
        if picked == current {
            return false;
        }
        if let Some(tip_prep) = self.value["tip_prep"].as_object_mut() {
            match picked {
                Some(name) => tip_prep.insert("z_controller_preset".into(), name.into()),
                None => tip_prep.remove("z_controller_preset"),
            };
        }
        true
    }
}

impl Tool for TipPrepTool {
    fn id(&self) -> &'static str {
        "tip_prep"
    }

    fn label(&self) -> &str {
        "Tip prep"
    }

    fn setup(&mut self, ui: &mut egui::Ui, cx: &mut SetupCx<'_>) {
        // The path on its own line, taking the width there is; the buttons
        // wrap below it, so a narrow window pushes nothing off the edge.
        let mut load = false;
        ui.horizontal(|ui| {
            ui.label("Config file");
            let width = (ui.available_width() - 60.0).max(120.0);
            let (field, picked) = path_field(ui, &mut self.path, width, || {
                rfd::FileDialog::new()
                    .add_filter("TOML", &["toml"])
                    .pick_file()
            });
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            load = picked || entered;
        });
        if load {
            self.load(cx);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!self.path.is_empty(), egui::Button::new("Re-read file"))
                .on_hover_text("Read the file again, dropping edits made here")
                .clicked()
            {
                self.load(cx);
            }
            if ui
                .add_enabled(!self.path.is_empty(), egui::Button::new("Save"))
                .on_hover_text(
                    "Write the form to the file, with the Connection page's settings in \
                     its connection tables",
                )
                .clicked()
            {
                self.save(&cx.connection);
            }
            if ui.button("Save as…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("TOML", &["toml"])
                    .save_file()
            {
                self.path = path.display().to_string();
                self.save(&cx.connection);
            }
            if ui.button("Validate").clicked() {
                self.validate();
            }
            if ui.button("Defaults").clicked() {
                self.set_config(&AppConfig::default());
                self.message = Some(Note::ok("Reset to the built-in defaults"));
            }
        });
        note(ui, &self.message);
        ui.add_space(8.0);

        // While a run is going, the fields it keeps from its start are
        // drawn greyed out: Apply to run would not change them.
        let live = self.reload.is_live();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut changed = false;
            section(
                ui,
                "Key settings",
                Some(if live {
                    "The settings that decide a run. Greyed out: kept as the run started \
                     until the next one."
                } else {
                    "The settings that decide a run"
                }),
            );
            egui::Frame::group(ui.style()).show(ui, |ui| {
                egui::Grid::new("tip_prep_featured")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        for path in FEATURED {
                            let enabled = !(live && FROZEN.contains(path));
                            changed |=
                                self.form
                                    .render_path_enabled(ui, &mut self.value, path, enabled);
                        }
                        changed |= self.render_z_preset(ui, &cx.connection, !live);
                    });
            });
            section(
                ui,
                "All settings",
                Some(
                    "Everything else in the config. Where the controller is and where logs \
                     go are set on the Connection page; Save writes them into the file so \
                     the CLI reads the same one.",
                ),
            );
            let hidden: Vec<&str> = CONNECTION_SECTIONS
                .iter()
                .chain(FEATURED)
                .chain(&["tip_prep.z_controller_preset"])
                .copied()
                .collect();
            changed |= self.form.render_except(ui, &mut self.value, &hidden);
            if changed {
                self.message = None;
            }
        });
    }

    fn job(&self) -> Result<Box<dyn Job>, String> {
        let config = self.parse()?;
        Ok(Box::new(TipPrepJob {
            config,
            reload: self.reload.clone(),
        }))
    }

    fn panel(&mut self, ui: &mut egui::Ui, view: &RunView) {
        let cycle = view.latest(CYCLE).unwrap_or(0.0) as usize;
        let phase = view.phase().unwrap_or("");
        let is_sharp = view.latest(CYCLE_SHARP) == Some(1.0);
        let colors = Palette::for_theme(ui.visuals().dark_mode);
        let (shape, shape_color) = if phase == "stable" {
            ("Stable", colors.bounds.to_opaque())
        } else if is_sharp {
            ("Sharp", colors.bounds.to_opaque())
        } else if cycle > 0 {
            ("Blunt", ui.visuals().text_color())
        } else {
            ("-", ui.visuals().weak_text_color())
        };
        // A phase is what a run is doing; once it has ended it is doing
        // nothing, whatever the last phase event said.
        let phase = if view.finish.is_some() {
            "ended"
        } else if phase.is_empty() {
            "-"
        } else {
            phase
        };
        let run = by_cycle(view);
        let big = |text: String| egui::RichText::new(text).size(24.0).strong();
        let value = |text: String| egui::RichText::new(text).size(16.0).monospace();

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                stat(ui, "Tip", big(shape.to_string()).color(shape_color));
                stat(
                    ui,
                    "Cycle",
                    big(if cycle > 0 {
                        cycle.to_string()
                    } else {
                        "-".into()
                    }),
                );
                stat(ui, "Phase", value(phase.replace('_', " ")));
                stat(
                    ui,
                    "Freq shift",
                    value(
                        view.latest(CYCLE_FREQ_SHIFT)
                            .or_else(|| view.latest(FREQ_SHIFT_SERIES))
                            .map(|f| format_si(f, "Hz"))
                            .unwrap_or_else(|| "-".into()),
                    ),
                );
                // The pulse action as it fires: the cycle event only
                // follows after the reposition and the read, which left the
                // field empty all through cycle 1.
                stat(
                    ui,
                    "Pulse",
                    value(
                        fired_pulse(view)
                            .or_else(|| run.last_pulse())
                            .map(|v| format_si(v, "V"))
                            .unwrap_or_else(|| "-".into()),
                    ),
                );
                stat(
                    ui,
                    "Sharp band",
                    value(
                        self.sharp_bounds(view)
                            .map(|(lo, hi)| format!("{} to {} Hz", number(lo), number(hi)))
                            .unwrap_or_else(|| "-".into()),
                    ),
                );
            });
        });

        // The last cycle any series reaches, so both plots span the same
        // cycles and their x axes line up.
        let last_cycle = run
            .freq_shift
            .iter()
            .chain(&run.other_reads)
            .chain(&run.pulses)
            .chain(&run.max_pulses)
            .map(|p| p[0])
            .fold(1.0, f64::max);
        let height = plot_height(ui);
        let between = colors.second.gamma_multiply(0.85);

        plot_header(
            ui,
            "Frequency shift",
            &[
                (Mark::Dot, "after a cycle", colors.first),
                (Mark::Ring, "between cycles", between),
                (Mark::Dash, "sharp band", colors.bounds.to_opaque()),
            ],
            "Measured after each cycle. Hollow points are the readings between cycles: \
             the initial read, and a stability check's confirmations and final read.",
        );
        let fs_line = Line::new("", PlotPoints::from(run.freq_shift.clone())).color(colors.first);
        let fs_marks = Points::new("after a cycle", PlotPoints::from(run.freq_shift))
            .color(colors.first)
            .radius(MARKER_RADIUS);
        let other_marks = Points::new("between cycles", PlotPoints::from(run.other_reads))
            .color(between)
            .filled(false)
            .radius(MARKER_RADIUS + 1.0);
        let bounds = self.sharp_bounds(view);
        let mut plot = cycle_plot("tip_prep_freq_shift", "Hz", height, last_cycle);
        if let Some((lower, upper)) = bounds {
            plot = plot.include_y(lower).include_y(upper);
        }
        plot.show(ui, |plot_ui| {
            plot_ui.line(fs_line);
            plot_ui.points(fs_marks);
            plot_ui.points(other_marks);
            if let Some((lower, upper)) = bounds {
                for y in [lower, upper] {
                    plot_ui.hline(
                        HLine::new("sharp band", y)
                            .color(colors.bounds)
                            .style(egui_plot::LineStyle::Dashed { length: 5.0 }),
                    );
                }
            }
        });

        plot_header(
            ui,
            "Pulse voltage",
            &[(Mark::Bar, "pulse", colors.second)],
            "Every pulse fired, as a bar from 0 V. A bar between two cycles is the pulse a \
             failed stability check fires before the next cycle.",
        );
        // One series: a stability check's pulse is a pulse like any other,
        // it only sits half a cycle on. At ±0.2 around their places, a
        // cycle's bar and one half a cycle on leave a gap between them.
        let pulses: Vec<[f64; 2]> = run.pulses.iter().chain(&run.max_pulses).copied().collect();
        let bars = pulse_bars("pulse", &pulses, 0.4, colors.second);
        let zero = ui.visuals().weak_text_color();
        cycle_plot("tip_prep_pulses", "V", height, last_cycle)
            // Pulses run over about ±10 V: a line every volt, heavier every
            // five and ten.
            .y_grid_spacer(egui_plot::uniform_grid_spacer(|_| [1.0, 5.0, 10.0]))
            .include_y(0.0)
            .show(ui, |plot_ui| {
                plot_ui.hline(HLine::new("", 0.0).color(zero).width(1.0_f32));
                plot_ui.bar_chart(bars);
            });
    }

    fn run_controls(&mut self, ui: &mut egui::Ui, view: &RunView) {
        self.render_apply(ui, view);
    }

    fn prefs(&self) -> serde_json::Value {
        serde_json::json!({ "path": self.path, "config": self.value })
    }

    fn restore(&mut self, prefs: &serde_json::Value) {
        if let Some(path) = prefs.get("path").and_then(|p| p.as_str()) {
            self.path = path.to_string();
        }
        // Only a config that still deserializes is worth restoring; a
        // saved value from an older field layout falls back to the defaults.
        if let Some(saved) = prefs.get("config")
            && let Ok(config) = serde_json::from_value::<AppConfig>(saved.clone())
        {
            self.set_config(&config);
        }
    }
}

/// The connection tables of a config, as the Connection page holds them.
fn connection_of(config: &AppConfig) -> ConnectionSettings {
    ConnectionSettings {
        backend: NanonisBackend::from_config(config),
        log_dir: config
            .experiment_logging
            .enabled
            .then(|| PathBuf::from(&config.experiment_logging.output_path)),
        presets_file: PathBuf::from(&config.controllers.presets_file),
    }
}

/// Put the Connection page's settings into a config's connection tables.
fn set_connection(config: &mut AppConfig, s: &ConnectionSettings) {
    s.backend.write_into(config);
    match &s.log_dir {
        Some(dir) => {
            config.experiment_logging.enabled = true;
            config.experiment_logging.output_path = dir.display().to_string();
        }
        None => config.experiment_logging.enabled = false,
    }
    config.controllers.presets_file = s.presets_file.display().to_string();
}

/// The run by cycle: what each cycle fired and then measured, and what
/// happened between cycles, the max pulses of a stability check and the
/// readings that are not a cycle's own (the initial one, a check's
/// confirmations and final read), spread across the gap after the cycle
/// they followed.
#[derive(Debug, Default, PartialEq)]
struct ByCycle {
    freq_shift: Vec<[f64; 2]>,
    other_reads: Vec<[f64; 2]>,
    pulses: Vec<[f64; 2]>,
    max_pulses: Vec<[f64; 2]>,
}

impl ByCycle {
    /// The voltage of the pulse fired last, cycle or max. A max pulse sits
    /// half a cycle after its cycle, so the later x is the later pulse.
    fn last_pulse(&self) -> Option<f64> {
        match (self.pulses.last(), self.max_pulses.last()) {
            (Some(p), Some(m)) => Some(if m[0] > p[0] { m[1] } else { p[1] }),
            (p, m) => p.or(m).map(|p| p[1]),
        }
    }
}

fn by_cycle(view: &RunView) -> ByCycle {
    // Both kinds arrive in time order, so one pass over the cycles places
    // every max pulse after the last cycle that came before it.
    let cycles: Vec<(f64, CycleEvent)> = view
        .custom(CycleEvent::KIND)
        .iter()
        .filter_map(|(at, data)| Some((*at, CycleEvent::deserialize(data).ok()?)))
        .collect();
    let mut run = ByCycle::default();
    for (_, c) in &cycles {
        run.freq_shift.push([c.cycle as f64, c.freq_shift]);
        run.pulses.push([c.cycle as f64, c.pulse_voltage]);
    }
    let mut before = 0;
    for (at, data) in view.custom(MaxPulseEvent::KIND) {
        let Ok(max) = MaxPulseEvent::deserialize(data) else {
            continue;
        };
        while before < cycles.len() && cycles[before].0 <= *at {
            before += 1;
        }
        let after = before.checked_sub(1).map_or(0, |i| cycles[i].1.cycle);
        run.max_pulses.push([after as f64 + 0.5, max.pulse_voltage]);
    }

    // A cycle's own reading is the last one before its event; every other
    // reading since the cycle before goes between the two. A read that was
    // retried is not a reading, so only the batches that held count.
    let (values, stable) = (
        view.points(FREQ_SHIFT_SERIES),
        view.points(FREQ_SHIFT_STABLE),
    );
    let reads: Vec<[f64; 2]> = if values.len() == stable.len() {
        values
            .iter()
            .zip(stable)
            .filter(|(_, s)| s[1] == 1.0)
            .map(|(v, _)| *v)
            .collect()
    } else {
        values.to_vec()
    };
    let mut next = 0;
    for gap in 0..=cycles.len() {
        let end = cycles.get(gap).map_or(f64::INFINITY, |(at, _)| *at);
        let start = next;
        while next < reads.len() && reads[next][0] <= end {
            next += 1;
        }
        let own = usize::from(gap < cycles.len() && next > start);
        let between = &reads[start..next - own];
        let base = gap.checked_sub(1).map_or(0, |g| cycles[g].1.cycle) as f64;
        let spacing = 1.0 / (between.len() + 1) as f64;
        for (j, read) in between.iter().enumerate() {
            run.other_reads
                .push([base + (j + 1) as f64 * spacing, read[1]]);
        }
    }
    run
}

/// Height of each tip-prep plot: half the window's height after the status
/// box, the headers and the log sections, so the two plots fill the tab,
/// but never less than a readable minimum.
fn plot_height(ui: &egui::Ui) -> f32 {
    const RESERVED: f32 = 360.0;
    ((ui.ctx().content_rect().height() - RESERVED) / 2.0).clamp(180.0, 420.0)
}

/// How to move around a plot, said on every plot's header.
const PLOT_NAVIGATION: &str = "Drag to pan, scroll to zoom, right-drag a box to zoom into \
     it, double-click to fit. The two plots pan together.";

/// How a series is marked, for the key beside a plot's title. Painted,
/// since the UI font has no circle or diamond glyphs.
#[derive(Clone, Copy)]
enum Mark {
    Dot,
    Ring,
    Bar,
    Dash,
}

/// Pulses as bars from 0 V, one per `[x, volts]`, `width` wide in cycles.
fn pulse_bars(name: &str, pulses: &[[f64; 2]], width: f64, color: egui::Color32) -> BarChart {
    let bars = pulses
        .iter()
        .map(|&[x, volts]| {
            Bar::new(x, volts)
                .width(width)
                .fill(color.gamma_multiply(0.7))
                .stroke(egui::Stroke::new(1.0_f32, color))
        })
        .collect();
    BarChart::new(name, bars).color(color)
}

/// A plot's title, with its key beside it and what it shows and how to
/// navigate it on hover. The key sits here rather than on the plot, where
/// it covered the first cycles.
fn plot_header(ui: &mut egui::Ui, title: &str, key: &[(Mark, &str, egui::Color32)], about: &str) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).heading().size(17.0))
            .on_hover_text(format!("{about}\n\n{PLOT_NAVIGATION}"));
        ui.add_space(12.0);
        for (mark, label, color) in key {
            key_mark(ui, *mark, *color);
            ui.label(egui::RichText::new(*label).size(12.0));
            ui.add_space(8.0);
        }
    });
}

fn key_mark(ui: &mut egui::Ui, mark: Mark, color: egui::Color32) {
    let size = ui.text_style_height(&egui::TextStyle::Small);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size * 1.4, size), egui::Sense::hover());
    let painter = ui.painter();
    let c = rect.center();
    let r = size * 0.3;
    match mark {
        Mark::Dot => {
            painter.circle_filled(c, r, color);
        }
        Mark::Ring => {
            painter.circle_stroke(c, r, egui::Stroke::new(1.5_f32, color));
        }
        Mark::Bar => {
            let bar = egui::Rect::from_min_max(
                c + egui::vec2(-r * 0.8, -size * 0.45),
                c + egui::vec2(r * 0.8, size * 0.45),
            );
            painter.rect_filled(bar, 0.0, color);
        }
        Mark::Dash => {
            let stroke = egui::Stroke::new(1.5_f32, color);
            let w = rect.width() / 2.0;
            painter.line_segment(
                [c - egui::vec2(w, 0.0), c - egui::vec2(w * 0.2, 0.0)],
                stroke,
            );
            painter.line_segment(
                [c + egui::vec2(w * 0.2, 0.0), c + egui::vec2(w, 0.0)],
                stroke,
            );
        }
    }
}

/// Grid lines over cycles: whole cycles only, about eight across, at 1, 2
/// or 5 times a power of ten. The default spacer drew tenths of a cycle,
/// a dense field of lines that mean nothing.
fn cycle_grid(input: egui_plot::GridInput) -> Vec<egui_plot::GridMark> {
    let (lo, hi) = input.bounds;
    let raw = ((hi - lo) / 8.0).max(1.0);
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|m| m * magnitude)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * magnitude);
    let mut marks = Vec::new();
    let mut value = (lo / step).ceil() * step;
    while value <= hi {
        marks.push(egui_plot::GridMark {
            value,
            step_size: step,
        });
        value += step;
    }
    marks
}

/// A plot over cycles: whole-cycle ticks on x from 0 to past
/// `last_cycle`, a y axis of fixed width with tick labels in the unit, and
/// navigation on. Both tip-prep plots span the same cycles with the same
/// axis width, and share one x range and one cursor, so their cycles line
/// up and panning one pans the other.
fn cycle_plot<'a>(id: &str, unit: &'static str, height: f32, last_cycle: f64) -> Plot<'a> {
    const LINK: &str = "tip_prep_cycles";
    Plot::new(id)
        .height(height)
        .include_x(0.0)
        .include_x(last_cycle + 0.5)
        .allow_drag([true, true])
        .allow_zoom([true, true])
        .allow_scroll(true)
        .allow_boxed_zoom(true)
        .allow_double_click_reset(true)
        .link_axis(LINK, [true, false])
        .link_cursor(LINK, [true, false])
        .x_grid_spacer(cycle_grid)
        .custom_x_axes(vec![AxisHints::new_x().label("Cycle").formatter(
            |mark, _| {
                let whole = mark.value.round();
                if (mark.value - whole).abs() < 1e-6 && whole >= 0.0 {
                    // `+ 0.0` turns -0 into 0, which `round` makes of a tick a hair below zero.
                    format!("{:.0}", whole + 0.0)
                } else {
                    String::new()
                }
            },
        )])
        .custom_y_axes(vec![
            AxisHints::new_y()
                .min_thickness(Y_AXIS_WIDTH)
                .formatter(move |mark, _| format_tick(mark.value, unit)),
        ])
        .label_formatter(move |name, point| {
            let what = if name.is_empty() { "" } else { name };
            format!("{what}\ncycle {:.1}: {}", point.x, format_si(point.y, unit))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tip::config::TcpChannelMapping;

    #[test]
    fn the_series_keys_follow_the_event_kinds() {
        for key in [CYCLE, CYCLE_FREQ_SHIFT, CYCLE_SHARP] {
            assert!(key.starts_with(&format!("{}.", CycleEvent::KIND)), "{key}");
        }
    }

    /// A file's connection tables go into the Connection page and come back
    /// out unchanged, so saving from the workbench cannot lose what the CLI
    /// needs.
    #[test]
    fn connection_tables_round_trip_through_the_page() {
        let mut config = AppConfig::default();
        config.nanonis.host_ip = "192.168.1.10".into();
        config.nanonis.layout_file = Some("a.lyt".into());
        config.data_acquisition.sample_rate = 500;
        config.tcp_channel_mapping = Some(vec![TcpChannelMapping {
            nanonis_index: 76,
            tcp_channel: 3,
        }]);
        let settings = connection_of(&config);
        assert_eq!(settings.backend.host, "192.168.1.10");
        assert_eq!(settings.backend.port, 6501);
        assert_eq!(settings.backend.sample_rate_hz, 500.0);
        assert_eq!(settings.log_dir, Some(PathBuf::from("./experiments")));

        let mut written = AppConfig::default();
        set_connection(&mut written, &settings);
        assert_eq!(connection_of(&written), settings);
        assert_eq!(written.nanonis.host_ip, "192.168.1.10");
        assert_eq!(written.tcp_channel_mapping.unwrap()[0].nanonis_index, 76);
    }

    #[test]
    fn a_page_without_a_log_dir_switches_logging_off_in_the_file() {
        let settings = ConnectionSettings {
            log_dir: None,
            ..connection_of(&AppConfig::default())
        };
        let mut written = AppConfig::default();
        set_connection(&mut written, &settings);
        assert!(!written.experiment_logging.enabled);
        assert_eq!(connection_of(&written).log_dir, None);
    }

    /// The plots run over cycles: each cycle's shift and pulse sit at its
    /// number, and a max pulse fired during a stability check sits half a
    /// cycle after the cycle it followed.
    #[test]
    fn the_plots_run_over_cycles_with_max_pulses_between() {
        let mut view = RunView::default();
        let cycle = |n: usize, fs: f64, v: f64| {
            rusty_tip::event::Event::typed(&CycleEvent {
                cycle: n,
                elapsed_secs: 0.0,
                freq_shift: fs,
                pulse_voltage: v,
                is_sharp: false,
            })
        };
        let max = |v: f64| rusty_tip::event::Event::typed(&MaxPulseEvent { pulse_voltage: v });
        for event in [
            cycle(1, -3.0, 4.0),
            cycle(2, -1.5, 5.0),
            max(8.0),
            cycle(3, -2.5, 4.5),
        ] {
            view.apply_event(event);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let run = by_cycle(&view);
        assert_eq!(run.freq_shift, vec![[1.0, -3.0], [2.0, -1.5], [3.0, -2.5]]);
        assert_eq!(run.pulses, vec![[1.0, 4.0], [2.0, 5.0], [3.0, 4.5]]);
        assert_eq!(run.max_pulses, vec![[2.5, 8.0]]);
        assert_eq!(run.last_pulse(), Some(4.5));
    }

    /// The pulse shows as soon as it fires, before the cycle that fired it
    /// has read and logged its event.
    #[test]
    fn the_pulse_shows_before_its_cycle_event() {
        let mut view = RunView::default();
        assert_eq!(fired_pulse(&view), None);
        view.apply_event(rusty_tip::event::Event::action_started(
            "bias_pulse",
            serde_json::json!({ "voltage": -4.5, "duration_ms": 50 }),
            1,
        ));
        assert_eq!(fired_pulse(&view), Some(-4.5));
        assert!(by_cycle(&view).pulses.is_empty());
    }

    /// The readings that are not a cycle's own sit between the cycles: the
    /// initial one before cycle 1, a stability check's after the cycle it
    /// followed. A batch that was retried is left out.
    #[test]
    fn readings_between_cycles_sit_between_them() {
        let mut view = RunView::default();
        let read = |fs: f64, stable: bool| {
            rusty_tip::event::Event::data_collected(
                "stable_read",
                serde_json::json!({ "value": fs, "stable": stable }),
            )
        };
        let cycle = |n: usize, fs: f64| {
            rusty_tip::event::Event::typed(&CycleEvent {
                cycle: n,
                elapsed_secs: 0.0,
                freq_shift: fs,
                pulse_voltage: 4.0,
                is_sharp: false,
            })
        };
        for event in [
            read(-1.0, true), // initial
            read(-9.0, false),
            read(-3.0, true),
            cycle(1, -3.0),
            read(-0.5, true),
            cycle(2, -0.5),
            read(-0.6, true), // confirmations
            read(-0.7, true),
            read(-0.8, true),
            read(-1.2, true), // final read
            read(-2.5, true), // the site the max pulse moved on to
            read(-2.4, true),
            cycle(3, -2.4),
        ] {
            view.apply_event(event);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let run = by_cycle(&view);
        let values: Vec<f64> = run.other_reads.iter().map(|p| p[1]).collect();
        assert_eq!(values, vec![-1.0, -0.6, -0.7, -0.8, -1.2, -2.5]);
        assert!(run.other_reads[0][0] > 0.0 && run.other_reads[0][0] < 1.0);
        assert!(
            run.other_reads[1..]
                .iter()
                .all(|p| p[0] > 2.0 && p[0] < 3.0)
        );
    }
}
