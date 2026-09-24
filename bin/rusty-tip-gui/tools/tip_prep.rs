//! Tip preparation as a workbench tool.
//!
//! Setup is the config file as TOML text, loaded from and saved to disk and
//! validated the way the CLI validates it. That is the stopgap the handoff
//! allows for until schema forms (step 3) exist; it deliberately does not
//! copy the old GUI's field-by-field `EditableConfig`. The Run panel is the
//! old Control tab: tip state, the frequency-shift trace with the sharp
//! band, and the pulse voltage history.

use eframe::egui;
use egui_plot::{HLine, Line, Plot, PlotPoints, Points};

use rusty_tip::config::AppConfig;
use rusty_tip::experiment_log::ToolSchema;
use rusty_tip::routine::{Outcome, run_routine};
use rusty_tip::session::{Job, JobCx};
use rusty_tip::spm_error::SpmError;
use rusty_tip::tip_prep::TipPrep;

use super::Tool;
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

pub struct TipPrepTool {
    /// The config file the text was loaded from, or will be saved to.
    path: String,
    /// The config as TOML, edited in place.
    text: String,
    /// The last parse of `text`, for the sharp band and the header.
    parsed: Option<AppConfig>,
    message: Option<(String, bool)>,
}

impl Default for TipPrepTool {
    fn default() -> Self {
        let mut tool = Self {
            path: String::new(),
            text: String::new(),
            parsed: None,
            message: None,
        };
        tool.reset_to_defaults();
        tool
    }
}

impl TipPrepTool {
    fn reset_to_defaults(&mut self) {
        let config = AppConfig::default();
        self.text = toml::to_string_pretty(&config).unwrap_or_default();
        self.parsed = Some(config);
    }

    /// Parse and validate the text the way the CLI would a file.
    fn parse(&self) -> Result<AppConfig, String> {
        let config: AppConfig = toml::from_str(&self.text).map_err(|e| e.to_string())?;
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
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                self.text = text;
                self.validate();
                if let Some((msg, false)) = &mut self.message {
                    *msg = format!("Loaded {}", self.path);
                }
            }
            Err(e) => self.message = Some((format!("Cannot read {}: {e}", self.path), true)),
        }
    }

    fn save(&mut self) {
        if !self.path.to_lowercase().ends_with(".toml") {
            self.path.push_str(".toml");
        }
        // Save what the operator wrote, not a re-serialization of it, so
        // comments in the file survive.
        match std::fs::write(&self.path, &self.text) {
            Ok(()) => self.message = Some((format!("Saved {}", self.path), false)),
            Err(e) => self.message = Some((format!("Cannot write {}: {e}", self.path), true)),
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
                self.reset_to_defaults();
                self.message = Some(("Reset to the built-in defaults".into(), false));
            }
        });
        if let Some((msg, is_error)) = &self.message {
            let color = if *is_error {
                egui::Color32::RED
            } else {
                egui::Color32::GREEN
            };
            ui.colored_label(color, msg);
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "The run uses this text as its config. Units are SI: 0.2e-9 is 0.2 nm. \
                 A schema-driven form replaces this editor in a later step.",
            )
            .small(),
        );
        egui::ScrollArea::vertical().show(ui, |ui| {
            let editor = egui::TextEdit::multiline(&mut self.text)
                .code_editor()
                .desired_width(f32::INFINITY)
                .desired_rows(30);
            if ui.add(editor).changed() {
                self.message = None;
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
        let (shape, shape_color) = if phase == "stable" {
            ("Stable", egui::Color32::GREEN)
        } else if is_sharp {
            ("Sharp", egui::Color32::YELLOW)
        } else if cycle > 0 {
            ("Blunt", egui::Color32::RED)
        } else {
            ("-", egui::Color32::GRAY)
        };
        let pulse_voltage = latest_pulse(view);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::Grid::new("tip_prep_status")
                .num_columns(4)
                .spacing([20.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Tip shape:");
                    ui.colored_label(shape_color, shape);
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
        serde_json::json!({ "path": self.path, "text": self.text })
    }

    fn restore(&mut self, prefs: &serde_json::Value) {
        if let Some(path) = prefs.get("path").and_then(|p| p.as_str()) {
            self.path = path.to_string();
        }
        if let Some(text) = prefs.get("text").and_then(|t| t.as_str())
            && !text.trim().is_empty()
        {
            self.text = text.to_string();
            self.parsed = self.parse().ok();
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
