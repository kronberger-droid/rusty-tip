//! Tip preparation as a workbench tool.
//!
//! Setup is a form drawn from `AppConfig`'s JSON Schema, with the settings
//! that decide a run at the top and the rest in sections below, plus load
//! and save as TOML. The Run panel is the old Control tab: tip state, the
//! frequency-shift trace with the sharp band, and the pulse voltage
//! history.

use std::path::PathBuf;

use eframe::egui;
use egui_plot::{AxisHints, HLine, Line, Plot, PlotPoints, Points};

use rusty_tip::config::AppConfig;
use rusty_tip::experiment_log::{LogEvent, ToolSchema};
use rusty_tip::routine::{Outcome, run_routine};
use rusty_tip::session::{Job, JobCx, NanonisBackend};
use rusty_tip::spm_error::SpmError;
use rusty_tip::tip_prep::{CycleEvent, MaxPulseEvent, TipPrep};
use serde::Deserialize;

use super::{SetupCx, Tool, load_toml, save_toml};
use crate::connection::ConnectionSettings;
use crate::form::SchemaForm;
use crate::run_view::RunView;
use crate::units::{format_si, number};
use crate::widgets::{Note, Palette, note, path_field};

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
        let current = cx
            .registry
            .get_by_name("current")
            .ok_or_else(|| SpmError::Workflow("the controller has no current signal".into()))?
            .signal_index();
        let mut routine = TipPrep::new(&self.config, fs, current);
        run_routine(cx.controller, cx.events, cx.shutdown, &mut routine)
    }
}

/// Series the panel reads off the [`RunView`], by the names the log uses.
const FREQ_SHIFT_SERIES: &str = "stable_read.value";
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

    /// The sharp window as the form has it now, for the plot.
    fn sharp_bounds(&self) -> Option<(f64, f64)> {
        let b = self.value.get("tip_prep")?.get("sharp_tip_bounds")?;
        Some((b.get(0)?.as_f64()?, b.get(1)?.as_f64()?))
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
            let width = (ui.available_width() - 40.0).max(120.0);
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
                .add_enabled(!self.path.is_empty(), egui::Button::new("Reload"))
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
                        }
                    });
            });
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Everything").strong());
            ui.label(
                egui::RichText::new(
                    "Where the controller is and where logs go are set on the Connection \
                     page; Save writes them into the file so the CLI reads the same one.",
                )
                .weak(),
            );
            changed |= self
                .form
                .render_except(ui, &mut self.value, CONNECTION_SECTIONS);
            if changed {
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
        let shape = if phase == "stable" {
            "Stable"
        } else if is_sharp {
            "Sharp"
        } else if cycle > 0 {
            "Blunt"
        } else {
            "-"
        };
        let run = by_cycle(view);

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
                            .map(|f| format_si(f, "Hz"))
                            .unwrap_or_else(|| "-".into()),
                    );
                    ui.end_row();

                    ui.label("Pulse voltage:");
                    ui.label(
                        run.last_pulse()
                            .map(|v| format_si(v, "V"))
                            .unwrap_or_else(|| "-".into()),
                    );
                    ui.label("Sharp band:");
                    ui.label(
                        self.sharp_bounds()
                            .map(|(lo, hi)| format!("{} to {} Hz", number(lo), number(hi)))
                            .unwrap_or_else(|| "-".into()),
                    );
                    ui.end_row();
                });
        });

        let colors = Palette::for_theme(ui.visuals().dark_mode);

        ui.add_space(6.0);
        ui.label("Freq shift measured after each cycle");
        let fs_line =
            Line::new("Freq shift", PlotPoints::from(run.freq_shift.clone())).color(colors.first);
        let fs_marks = Points::new("Freq shift", PlotPoints::from(run.freq_shift))
            .color(colors.first)
            .radius(MARKER_RADIUS);
        let bounds = self.sharp_bounds();
        let mut plot = cycle_plot("tip_prep_freq_shift", 160.0, "Hz");
        if let Some((lower, upper)) = bounds {
            plot = plot.include_y(lower).include_y(upper);
        }
        plot.show(ui, |plot_ui| {
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
        ui.label("Pulse fired at the start of each cycle; max pulses between");
        let v_line = Line::new("Pulse", PlotPoints::from(run.pulses.clone())).color(colors.second);
        let v_marks = Points::new("Pulse", PlotPoints::from(run.pulses))
            .color(colors.second)
            .radius(MARKER_RADIUS);
        let max_marks = Points::new("Max pulse", PlotPoints::from(run.max_pulses))
            .color(colors.bounds)
            .shape(egui_plot::MarkerShape::Diamond)
            .radius(MARKER_RADIUS + 1.5);
        cycle_plot("tip_prep_pulses", 120.0, "V").show(ui, |plot_ui| {
            plot_ui.line(v_line);
            plot_ui.points(v_marks);
            plot_ui.points(max_marks);
        });
        ui.label(
            egui::RichText::new(
                "Drag to pan, scroll to zoom, right-drag a box to zoom into it, double-click \
                 to fit. The two plots pan together.",
            )
            .weak()
            .small(),
        );
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

/// The run by cycle: what each cycle fired and then measured, and the max
/// pulses fired during a stability check, placed half a cycle after the
/// cycle they followed so they sit between the regular ones.
#[derive(Debug, Default, PartialEq)]
struct ByCycle {
    freq_shift: Vec<[f64; 2]>,
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
    run
}

/// A plot over cycles: integer ticks on x, a fixed-width y axis with
/// tick labels in the unit, and navigation on. Both tip-prep plots share
/// one x range and one cursor, so panning one pans the other.
fn cycle_plot<'a>(id: &str, height: f32, unit: &'static str) -> Plot<'a> {
    const LINK: &str = "tip_prep_cycles";
    Plot::new(id)
        .height(height)
        .allow_drag([true, true])
        .allow_zoom([true, true])
        .allow_scroll(true)
        .allow_boxed_zoom(true)
        .allow_double_click_reset(true)
        .link_axis(LINK, [true, false])
        .link_cursor(LINK, [true, false])
        .custom_x_axes(vec![AxisHints::new_x().label("Cycle").formatter(
            |mark, _| {
                let whole = mark.value.round();
                if (mark.value - whole).abs() < 1e-6 && whole >= 0.0 {
                    format!("{whole:.0}")
                } else {
                    String::new()
                }
            },
        )])
        // The y strip is sized to its widest tick label by default, which
        // on a narrow run starts too thin to draw any; a fixed minimum
        // keeps the labels there from the first point on.
        .custom_y_axes(vec![
            AxisHints::new_y()
                .label(unit)
                .min_thickness(64.0)
                .formatter(move |mark, _| format_si(mark.value, unit)),
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
}
