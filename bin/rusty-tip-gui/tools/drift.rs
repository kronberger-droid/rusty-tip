//! Z drift compensation as a workbench tool: status, measure, compensate,
//! off. Runs between passes with the tip engaged and leaves it there.

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints, Points};

use rusty_tip::drift::{DriftOp, DriftParams, DriftRoutine};
use rusty_tip::experiment_log::ToolSchema;
use rusty_tip::routine::{Outcome, run_routine};
use rusty_tip::session::{Job, JobCx};
use rusty_tip::spm_error::SpmError;

use super::Tool;
use crate::run_view::RunView;

/// Picometres to metres.
const PM: f64 = 1e-12;

pub struct DriftJob {
    z_name: String,
    params: DriftParams,
}

impl Job for DriftJob {
    fn name(&self) -> &str {
        "drift"
    }

    fn log_schema(&self) -> ToolSchema {
        rusty_tip::drift::log_schema()
    }

    fn header_config(&self) -> serde_json::Value {
        serde_json::json!({ "z_signal": self.z_name, "params": self.params })
    }

    fn run(&mut self, cx: JobCx<'_>) -> Result<Outcome, SpmError> {
        let z = cx
            .registry
            .get_by_name(&self.z_name)
            .ok_or_else(|| {
                SpmError::Workflow(format!(
                    "no signal called {:?} on this controller",
                    self.z_name
                ))
            })?
            .signal_index();
        let mut routine = DriftRoutine::new(z, self.params.clone());
        run_routine(cx.controller, cx.events, cx.shutdown, &mut routine)
    }
}

/// Which response the form offers. The controller's sign convention is
/// learned with a trial burst unless the operator already knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseChoice {
    Learn,
    Adds,
    Subtracts,
}

pub struct DriftTool {
    op: DriftOp,
    z_name: String,
    window_s: String,
    bursts: String,
    trial_pm_s: String,
    response: ResponseChoice,
    samples: String,
    require_stream: bool,
    message: Option<String>,
}

impl Default for DriftTool {
    fn default() -> Self {
        let d = DriftParams::default();
        Self {
            op: d.op,
            z_name: "Z (m)".into(),
            window_s: format!("{}", d.window_ms as f64 / 1000.0),
            bursts: d.bursts.to_string(),
            trial_pm_s: format!("{}", d.trial_vz / PM),
            response: ResponseChoice::Learn,
            samples: d.samples.to_string(),
            require_stream: d.require_stream,
            message: None,
        }
    }
}

impl DriftTool {
    fn params(&self) -> Result<DriftParams, String> {
        let window_s = self
            .window_s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|w| w.is_finite() && *w > 0.0)
            .ok_or_else(|| format!("window {:?} is not a length in seconds", self.window_s))?;
        let bursts = self
            .bursts
            .trim()
            .parse::<usize>()
            .map_err(|_| format!("bursts {:?} is not a count", self.bursts))?;
        let trial_pm_s = self
            .trial_pm_s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|t| t.is_finite() && *t > 0.0)
            .ok_or_else(|| format!("trial {:?} is not a velocity in pm/s", self.trial_pm_s))?;
        let samples = self
            .samples
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|s| *s >= 3)
            .ok_or_else(|| format!("samples {:?} must be a count of at least 3", self.samples))?;
        if self.z_name.trim().is_empty() {
            return Err("the Z signal name is empty".into());
        }
        Ok(DriftParams {
            op: self.op,
            window_ms: (window_s * 1000.0).round() as u64,
            bursts,
            trial_vz: trial_pm_s * PM,
            response: match self.response {
                ResponseChoice::Learn => None,
                ResponseChoice::Adds => Some(1.0),
                ResponseChoice::Subtracts => Some(-1.0),
            },
            samples,
            require_stream: self.require_stream,
        })
    }
}

impl Tool for DriftTool {
    fn id(&self) -> &'static str {
        "drift"
    }

    fn label(&self) -> &str {
        "Drift"
    }

    fn setup(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(
                "Measures how fast Z drifts with the feedback closed and the scan stopped, \
                 and can leave the controller compensating for it. Position the tip on flat, \
                 quiet ground first; the tip is left where it is.",
            )
            .small(),
        );
        ui.add_space(6.0);

        egui::Grid::new("drift_setup")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("Operation");
                egui::ComboBox::from_id_salt("drift_op")
                    .selected_text(op_label(self.op))
                    .show_ui(ui, |ui| {
                        for op in [
                            DriftOp::Status,
                            DriftOp::Measure,
                            DriftOp::Compensate,
                            DriftOp::Off,
                        ] {
                            ui.selectable_value(&mut self.op, op, op_label(op))
                                .on_hover_text(op_help(op));
                        }
                    });
                ui.end_row();

                ui.label("Z signal");
                ui.add(egui::TextEdit::singleline(&mut self.z_name).desired_width(160.0))
                    .on_hover_text("Resolved by name on the controller when the run starts");
                ui.end_row();

                let measures = matches!(self.op, DriftOp::Measure | DriftOp::Compensate);
                ui.add_enabled_ui(measures, |ui| {
                    ui.label("Burst window (s)");
                });
                ui.add_enabled_ui(measures, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.window_s).desired_width(80.0))
                        .on_hover_text(
                            "Length of one measurement burst. The error of a burst falls \
                             with the window to the power 1.5, so a longer window buys \
                             more than more bursts.",
                        );
                });
                ui.end_row();

                let compensates = self.op == DriftOp::Compensate;
                ui.add_enabled_ui(compensates, |ui| {
                    ui.label("Bursts");
                });
                ui.add_enabled_ui(compensates, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.bursts).desired_width(80.0))
                        .on_hover_text(
                            "Bursts to spend, the baseline and the trial included: at \
                             least 3 when the response has to be learned.",
                        );
                });
                ui.end_row();

                ui.add_enabled_ui(compensates, |ui| {
                    ui.label("Response");
                });
                ui.add_enabled_ui(compensates, |ui| {
                    egui::ComboBox::from_id_salt("drift_response")
                        .selected_text(match self.response {
                            ResponseChoice::Learn => "learn with a trial burst",
                            ResponseChoice::Adds => "+1: positive vz adds to the drift",
                            ResponseChoice::Subtracts => "-1: positive vz subtracts",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.response,
                                ResponseChoice::Learn,
                                "learn with a trial burst",
                            );
                            ui.selectable_value(
                                &mut self.response,
                                ResponseChoice::Adds,
                                "+1: positive vz adds to the drift",
                            );
                            ui.selectable_value(
                                &mut self.response,
                                ResponseChoice::Subtracts,
                                "-1: positive vz subtracts",
                            );
                        });
                });
                ui.end_row();

                ui.add_enabled_ui(
                    compensates && self.response == ResponseChoice::Learn,
                    |ui| {
                        ui.label("Trial velocity (pm/s)");
                    },
                );
                ui.add_enabled_ui(
                    compensates && self.response == ResponseChoice::Learn,
                    |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.trial_pm_s).desired_width(80.0),
                        )
                        .on_hover_text(
                            "Large on purpose: with the loop closed it only ramps the Z \
                             output for one burst, and the response comes out clean.",
                        );
                    },
                );
                ui.end_row();

                ui.add_enabled_ui(measures, |ui| {
                    ui.checkbox(&mut self.require_stream, "Require Z in the data stream")
                        .on_hover_text(
                            "Off allows timed polling when the stream does not carry Z. \
                             Far noisier.",
                        );
                });
                ui.end_row();

                ui.add_enabled_ui(measures && !self.require_stream, |ui| {
                    ui.label("Polled reads per window");
                });
                ui.add_enabled_ui(measures && !self.require_stream, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.samples).desired_width(80.0));
                });
                ui.end_row();
            });

        match self.params() {
            Ok(_) => self.message = None,
            Err(e) => self.message = Some(e),
        }
        if let Some(e) = &self.message {
            ui.colored_label(egui::Color32::RED, e);
        }
    }

    fn job(&self) -> Result<Box<dyn Job>, String> {
        Ok(Box::new(DriftJob {
            z_name: self.z_name.trim().to_string(),
            params: self.params()?,
        }))
    }

    fn panel(&mut self, ui: &mut egui::Ui, view: &RunView) {
        let statuses = view.custom("drift/status");
        let before = statuses
            .iter()
            .find(|(_, d)| d["when"] == "before")
            .map(|(_, d)| d);
        let after = statuses
            .iter()
            .rev()
            .find(|(_, d)| d["when"] == "after")
            .map(|(_, d)| d);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::Grid::new("drift_status")
                .num_columns(3)
                .spacing([20.0, 4.0])
                .show(ui, |ui| {
                    ui.label("");
                    ui.label(egui::RichText::new("before").strong());
                    ui.label(egui::RichText::new("after").strong());
                    ui.end_row();
                    ui.label("Compensation");
                    ui.label(before.map(status_line).unwrap_or_else(|| "-".into()));
                    ui.label(after.map(status_line).unwrap_or_else(|| "-".into()));
                    ui.end_row();
                    ui.label("Saturated");
                    ui.label(before.map(saturated).unwrap_or_else(|| "-".into()));
                    ui.label(after.map(saturated).unwrap_or_else(|| "-".into()));
                    ui.end_row();
                });
        });

        if let Some((_, m)) = view.custom("drift/measured").last() {
            ui.add_space(6.0);
            ui.label(format!(
                "Z drift: {:+.3} ± {:.3} pm/s ({} samples over {:.1} s){}",
                pm(&m["rate_m_s"]),
                pm(&m["std_err_m_s"]),
                m["samples"],
                m["window_s"].as_f64().unwrap_or(0.0),
                if m["negligible"] == true {
                    ", consistent with zero"
                } else {
                    ""
                }
            ));
        }

        if let Some((_, c)) = view.custom("drift/compensated").last() {
            ui.add_space(6.0);
            ui.label(format!(
                "Residual: {:+.3} ± {:.3} pm/s after {} bursts, {}",
                pm(&c["residual_rate_m_s"]),
                pm(&c["residual_std_err_m_s"]),
                c["bursts"],
                if c["converged"] == true {
                    "inside its error bar"
                } else {
                    "outside its error bar (one burst does that from noise now and then)"
                }
            ));
            match c["response"].as_f64() {
                Some(r) => ui.label(format!(
                    "Response {r:+.2}: a positive vz {} the measured drift. Choose it in \
                     Setup next time to skip the trial burst.",
                    if r > 0.0 { "adds to" } else { "subtracts from" }
                )),
                None => ui.label("Drift was negligible from the start; nothing was changed."),
            };
        }

        let bursts = view.custom("drift/burst");
        if !bursts.is_empty() {
            ui.add_space(8.0);
            ui.label("Bursts");
            egui::Grid::new("drift_bursts")
                .num_columns(4)
                .striped(true)
                .spacing([16.0, 2.0])
                .show(ui, |ui| {
                    for h in ["burst", "role", "vz (pm/s)", "drift (pm/s)"] {
                        ui.label(egui::RichText::new(h).strong());
                    }
                    ui.end_row();
                    for (_, b) in bursts {
                        ui.label(b["burst"].to_string());
                        ui.label(b["role"].as_str().unwrap_or("?"));
                        ui.label(format!("{:+.3}", pm(&b["vz_m_s"])));
                        ui.label(format!(
                            "{:+.3} ± {:.3}",
                            pm(&b["drift_m_s"]),
                            pm(&b["std_err_m_s"])
                        ));
                        ui.end_row();
                    }
                });

            let drift: Vec<[f64; 2]> = bursts
                .iter()
                .filter_map(|(_, b)| Some([b["burst"].as_f64()?, b["drift_m_s"].as_f64()? / PM]))
                .collect();
            let color = if ui.visuals().dark_mode {
                egui::Color32::LIGHT_BLUE
            } else {
                egui::Color32::from_rgb(0, 84, 159)
            };
            Plot::new("drift_bursts_plot")
                .height(120.0)
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .x_axis_label("burst")
                .y_axis_label("pm/s")
                .show(ui, |plot_ui| {
                    plot_ui.line(Line::new("drift", PlotPoints::from(drift.clone())).color(color));
                    plot_ui.points(
                        Points::new("bursts", PlotPoints::from(drift))
                            .color(color)
                            .radius(3.0),
                    );
                });
        }
    }

    fn prefs(&self) -> serde_json::Value {
        serde_json::json!({
            "op": self.op,
            "z_name": self.z_name,
            "window_s": self.window_s,
            "bursts": self.bursts,
            "trial_pm_s": self.trial_pm_s,
            "response": match self.response {
                ResponseChoice::Learn => "learn",
                ResponseChoice::Adds => "adds",
                ResponseChoice::Subtracts => "subtracts",
            },
            "samples": self.samples,
            "require_stream": self.require_stream,
        })
    }

    fn restore(&mut self, prefs: &serde_json::Value) {
        if let Ok(op) = serde_json::from_value::<DriftOp>(prefs["op"].clone()) {
            self.op = op;
        }
        let text = |key: &str, into: &mut String| {
            if let Some(s) = prefs[key].as_str() {
                *into = s.to_string();
            }
        };
        text("z_name", &mut self.z_name);
        text("window_s", &mut self.window_s);
        text("bursts", &mut self.bursts);
        text("trial_pm_s", &mut self.trial_pm_s);
        text("samples", &mut self.samples);
        self.response = match prefs["response"].as_str() {
            Some("adds") => ResponseChoice::Adds,
            Some("subtracts") => ResponseChoice::Subtracts,
            _ => ResponseChoice::Learn,
        };
        if let Some(b) = prefs["require_stream"].as_bool() {
            self.require_stream = b;
        }
    }
}

fn op_label(op: DriftOp) -> &'static str {
    match op {
        DriftOp::Status => "Status",
        DriftOp::Measure => "Measure",
        DriftOp::Compensate => "Compensate",
        DriftOp::Off => "Off",
    }
}

fn op_help(op: DriftOp) -> &'static str {
    match op {
        DriftOp::Status => "Read the compensation velocities and which axes have saturated",
        DriftOp::Measure => "Fit a drift rate from one burst; changes nothing",
        DriftOp::Compensate => {
            "Measure in bursts, correcting after each, and leave compensation on"
        }
        DriftOp::Off => "Switch compensation off; the velocities are kept",
    }
}

fn pm(v: &serde_json::Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN) / PM
}

fn status_line(d: &serde_json::Value) -> String {
    format!(
        "{}: vx {:+.3}, vy {:+.3}, vz {:+.3} pm/s",
        if d["enabled"] == true { "on" } else { "off" },
        pm(&d["vx_m_s"]),
        pm(&d["vy_m_s"]),
        pm(&d["vz_m_s"]),
    )
}

fn saturated(d: &serde_json::Value) -> String {
    let axes: Vec<&str> = [
        ("x_saturated", "x"),
        ("y_saturated", "y"),
        ("z_saturated", "z"),
    ]
    .into_iter()
    .filter(|(k, _)| d[*k] == true)
    .map(|(_, a)| a)
    .collect();
    if axes.is_empty() {
        format!(
            "none (limit {}% of range)",
            d["saturation_limit_percent"].as_f64().unwrap_or(0.0)
        )
    } else {
        format!(
            "{}: compensation there has stopped; only an off/on cycle restarts it",
            axes.join(", ")
        )
    }
}
