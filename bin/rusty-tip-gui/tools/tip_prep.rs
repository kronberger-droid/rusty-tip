//! Tip preparation as a workbench tool.
//!
//! Setup is a form drawn from `AppConfig`'s JSON Schema, with the settings
//! that decide a run at the top and the rest in sections below, plus load
//! and save as TOML. The Run panel is the old Control tab: tip state, the
//! frequency-shift trace with the sharp band, and the pulse voltage
//! history.

use eframe::egui;
use egui_plot::{HLine, Line, Plot, PlotPoints, Points};

use rusty_tip::config::AppConfig;
use rusty_tip::experiment_log::ToolSchema;
use rusty_tip::routine::{Outcome, run_routine};
use rusty_tip::session::{Job, JobCx};
use rusty_tip::spm_error::SpmError;
use rusty_tip::tip_prep::TipPrep;

use super::Tool;
use crate::form::SchemaForm;
use crate::run_view::RunView;

/// Tip prep as a [`Job`]: what the session runs.
pub struct TipPrepJob {
    pub config: AppConfig,
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
        let fs = cx
            .registry
            .get_by_name("freq shift")
            .ok_or_else(|| {
                SpmError::Workflow("the controller has no frequency-shift signal".into())
            })?
            .signal_index();
        let mut routine = TipPrep::new(&self.config, fs);
        run_routine(cx.controller, cx.events, cx.shutdown, &mut routine)
    }
}

/// Series the panel reads off the [`RunView`], by the names the log uses.
const FREQ_SHIFT_SERIES: &str = "stable_read.value";
const CYCLE: &str = "tip_prep/cycle.cycle";
const CYCLE_FREQ_SHIFT: &str = "tip_prep/cycle.freq_shift";
const CYCLE_PULSE: &str = "tip_prep/cycle.pulse_voltage";
const CYCLE_SHARP: &str = "tip_prep/cycle.is_sharp";
const MAX_PULSE: &str = "tip_prep/max_pulse.pulse_voltage";

/// Radius of the per-measurement markers. Both series are discrete
/// measurements, so they are drawn as points with a connecting line; a bare
/// `Line` renders nothing until the second point arrives.
const MARKER_RADIUS: f32 = 2.5;

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
    /// The last successful parse of `value`, for the sharp band.
    parsed: Option<AppConfig>,
    message: Option<(String, bool)>,
}

impl Default for TipPrepTool {
    fn default() -> Self {
        let schema = serde_json::to_value(schemars::schema_for!(AppConfig))
            .expect("the config schema serializes");
        let mut tool = Self {
            path: String::new(),
            value: serde_json::Value::Null,
            form: SchemaForm::new(schema),
            parsed: None,
            message: None,
        };
        tool.set_config(&AppConfig::default());
        tool
    }
}

impl TipPrepTool {
    fn set_config(&mut self, config: &AppConfig) {
        self.value = serde_json::to_value(config).unwrap_or(serde_json::Value::Null);
        self.parsed = Some(config.clone());
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
        match self.parse() {
            Ok(config) => {
                self.parsed = Some(config);
                self.message = Some(("Config is valid".into(), false));
            }
            Err(e) => self.message = Some((e, true)),
        }
    }

    fn load(&mut self) {
        let loaded = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("Cannot read {}: {e}", self.path))
            .and_then(|text| toml::from_str::<AppConfig>(&text).map_err(|e| e.to_string()));
        match loaded {
            Ok(config) => {
                self.set_config(&config);
                self.message = Some((format!("Loaded {}", self.path), false));
            }
            Err(e) => self.message = Some((e, true)),
        }
    }

    fn save(&mut self) {
        if !self.path.to_lowercase().ends_with(".toml") {
            self.path.push_str(".toml");
        }
        let written = self
            .parse()
            .and_then(|config| toml::to_string_pretty(&config).map_err(|e| e.to_string()))
            .and_then(|text| {
                std::fs::write(&self.path, text)
                    .map_err(|e| format!("Cannot write {}: {e}", self.path))
            });
        match written {
            Ok(()) => self.message = Some((format!("Saved {}", self.path), false)),
            Err(e) => self.message = Some((e, true)),
        }
    }

    fn sharp_bounds(&self) -> Option<(f64, f64)> {
        let b = self.parsed.as_ref()?.tip_prep.sharp_tip_bounds;
        Some((b[0], b[1]))
    }
}

impl Tool for TipPrepTool {
    fn id(&self) -> &'static str {
        "tip_prep"
    }

    fn label(&self) -> &str {
        "Tip prep"
    }

    fn setup(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Config file:");
            ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(360.0));
            if ui.button("Browse…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("TOML", &["toml"])
                    .pick_file()
            {
                self.path = path.display().to_string();
                self.load();
            }
            if ui
                .add_enabled(!self.path.is_empty(), egui::Button::new("Load"))
                .clicked()
            {
                self.load();
            }
            if ui
                .add_enabled(!self.path.is_empty(), egui::Button::new("Save"))
                .clicked()
            {
                self.save();
            }
            if ui.button("Validate").clicked() {
                self.validate();
            }
            if ui.button("Defaults").clicked() {
                self.set_config(&AppConfig::default());
                self.message = Some(("Reset to the built-in defaults".into(), false));
            }
        });
        if let Some((msg, is_error)) = &self.message {
            if *is_error {
                ui.colored_label(egui::Color32::RED, msg);
            } else {
                ui.label(msg);
            }
        }
        ui.add_space(8.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut changed = false;
            ui.label(egui::RichText::new("Key settings").strong());
            egui::Frame::group(ui.style()).show(ui, |ui| {
                egui::Grid::new("tip_prep_featured")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        for path in FEATURED {
                            changed |= self.form.render_path(ui, &mut self.value, path);
                            ui.end_row();
                        }
                    });
            });
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Everything").strong());
            ui.label(
                egui::RichText::new(
                    "The connection and logging sections are kept for the CLI; the \
                     workbench uses its Connection page instead.",
                )
                .weak(),
            );
            changed |= self.form.render(ui, &mut self.value);
            if changed {
                self.message = None;
                // Keep the sharp band on the plot in step with the form.
                self.parsed = serde_json::from_value(self.value.clone()).ok();
            }
        });
    }

    fn job(&self) -> Result<Box<dyn Job>, String> {
        let config = self.parse()?;
        Ok(Box::new(TipPrepJob { config }))
    }

    fn panel(&mut self, ui: &mut egui::Ui, view: &RunView) {
        let cycle = view.latest(CYCLE).unwrap_or(0.0) as usize;
        let phase = view.phase().unwrap_or("");
        let is_sharp = view.latest(CYCLE_SHARP) == Some(1.0);
        let shape = if phase == "stable" {
            "Stable"
        } else if is_sharp {
            "Sharp"
        } else if cycle > 0 {
            "Blunt"
        } else {
            "-"
        };
        let pulse_voltage = latest_pulse(view);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::Grid::new("tip_prep_status")
                .num_columns(4)
                .spacing([20.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Tip shape:");
                    ui.label(shape);
                    ui.label("Phase:");
                    ui.label(if phase.is_empty() { "-" } else { phase });
                    ui.end_row();

                    ui.label("Cycle:");
                    ui.label(if cycle > 0 {
                        cycle.to_string()
                    } else {
                        "-".into()
                    });
                    ui.label("Freq shift:");
                    ui.label(
                        view.latest(CYCLE_FREQ_SHIFT)
                            .or_else(|| view.latest(FREQ_SHIFT_SERIES))
                            .map(|f| format!("{f:.2} Hz"))
                            .unwrap_or_else(|| "-".into()),
                    );
                    ui.end_row();

                    ui.label("Pulse voltage:");
                    ui.label(
                        pulse_voltage
                            .map(|v| format!("{v:.2} V"))
                            .unwrap_or_else(|| "-".into()),
                    );
                    ui.label("Sharp band:");
                    ui.label(
                        self.sharp_bounds()
                            .map(|(lo, hi)| format!("{lo:.2} to {hi:.2} Hz"))
                            .unwrap_or_else(|| "-".into()),
                    );
                    ui.end_row();
                });
        });

        let colors = PlotColors::for_theme(ui.visuals().dark_mode);

        ui.add_space(6.0);
        ui.label("Freq shift, one point per stable read");
        let fs: Vec<[f64; 2]> = view.points(FREQ_SHIFT_SERIES).to_vec();
        let fs_line =
            Line::new("Freq shift (Hz)", PlotPoints::from(fs.clone())).color(colors.freq_shift);
        let fs_marks = Points::new("Measurements", PlotPoints::from(fs))
            .color(colors.freq_shift)
            .radius(MARKER_RADIUS);
        let bounds = self.sharp_bounds();
        Plot::new("tip_prep_freq_shift")
            .height(160.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .x_axis_label("Time (s)")
            .y_axis_label("Hz")
            .show(ui, |plot_ui| {
                plot_ui.line(fs_line);
                plot_ui.points(fs_marks);
                if let Some((lower, upper)) = bounds {
                    for (name, y) in [("Lower bound", lower), ("Upper bound", upper)] {
                        plot_ui.hline(
                            HLine::new(name, y)
                                .color(colors.bounds)
                                .style(egui_plot::LineStyle::Dashed { length: 5.0 }),
                        );
                    }
                }
            });

        ui.add_space(6.0);
        ui.label("Pulse voltage, as fired");
        let pulses = pulse_history(view);
        let v_line = Line::new("Pulse voltage (V)", PlotPoints::from(pulses.clone()))
            .color(colors.pulse_voltage);
        let v_marks = Points::new("Pulses", PlotPoints::from(pulses))
            .color(colors.pulse_voltage)
            .radius(MARKER_RADIUS);
        Plot::new("tip_prep_pulses")
            .height(120.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .x_axis_label("Time (s)")
            .y_axis_label("V")
            .show(ui, |plot_ui| {
                plot_ui.line(v_line);
                plot_ui.points(v_marks);
            });
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

/// Every pulse fired, the cycle pulses and the max pulses merged in time.
fn pulse_history(view: &RunView) -> Vec<[f64; 2]> {
    let mut pulses: Vec<[f64; 2]> = view
        .points(CYCLE_PULSE)
        .iter()
        .chain(view.points(MAX_PULSE))
        .copied()
        .collect();
    pulses.sort_by(|a, b| a[0].total_cmp(&b[0]));
    pulses
}

fn latest_pulse(view: &RunView) -> Option<f64> {
    pulse_history(view).last().map(|p| p[1])
}

/// Plot colours for one theme.
struct PlotColors {
    freq_shift: egui::Color32,
    pulse_voltage: egui::Color32,
    bounds: egui::Color32,
}

impl PlotColors {
    /// The dark palette is pale series and faint bounds, which wash out on
    /// white, so light mode gets saturated, darker equivalents and far more
    /// opaque bounds. That is what makes a screenshot survive being printed.
    fn for_theme(dark_mode: bool) -> Self {
        if dark_mode {
            Self {
                freq_shift: egui::Color32::LIGHT_BLUE,
                pulse_voltage: egui::Color32::from_rgb(255, 165, 0),
                bounds: egui::Color32::from_rgba_unmultiplied(0, 255, 0, 80),
            }
        } else {
            Self {
                freq_shift: egui::Color32::from_rgb(0, 84, 159),
                pulse_voltage: egui::Color32::from_rgb(191, 87, 0),
                bounds: egui::Color32::from_rgba_unmultiplied(0, 120, 40, 180),
            }
        }
    }
}
