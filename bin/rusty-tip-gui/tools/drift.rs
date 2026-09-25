//! Z drift compensation as a workbench tool: status, measure, compensate,
//! off. Runs between passes with the tip engaged and leaves it there.

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints, Points};

use rusty_tip::action::drift::DriftBurstEvent;
use rusty_tip::drift::{
    DriftCompensatedEvent, DriftMeasuredEvent, DriftOp, DriftParams, DriftRoutine,
    DriftStatusEvent, PM,
};
use rusty_tip::experiment_log::{LogEvent, ToolSchema};
use rusty_tip::routine::{Outcome, run_routine};
use rusty_tip::session::{Job, JobCx};
use rusty_tip::spm_error::SpmError;
use serde::{Deserialize, Serialize};

use super::{SetupCx, Tool};
use crate::run_view::RunView;

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

/// The setup, which is also what is saved between starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftTool {
    z_name: String,
    params: DriftParams,
}

impl Default for DriftTool {
    fn default() -> Self {
        Self {
            z_name: "Z (m)".into(),
            params: DriftParams::default(),
        }
    }
}

impl Tool for DriftTool {
    fn id(&self) -> &'static str {
        "drift"
    }

    fn label(&self) -> &str {
        "Drift"
    }

    fn setup(&mut self, ui: &mut egui::Ui, _cx: &mut SetupCx) {
        ui.label(
            egui::RichText::new(
                "Measures how fast Z drifts with the feedback closed and the scan stopped, \
                 and can leave the controller compensating for it. Position the tip on flat, \
                 quiet ground first; the tip is left where it is.",
            )
            .small(),
        );
        ui.add_space(6.0);

        let p = &mut self.params;
        let measures = matches!(p.op, DriftOp::Measure | DriftOp::Compensate);
        let compensates = p.op == DriftOp::Compensate;
        let learns = compensates && p.response.is_none();
        let polls = measures && !p.require_stream;

        egui::Grid::new("drift_setup")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("Operation");
                egui::ComboBox::from_id_salt("drift_op")
                    .selected_text(op_label(p.op))
                    .show_ui(ui, |ui| {
                        for op in [
                            DriftOp::Status,
                            DriftOp::Measure,
                            DriftOp::Compensate,
                            DriftOp::Off,
                        ] {
                            ui.selectable_value(&mut p.op, op, op_label(op))
                                .on_hover_text(op_help(op));
                        }
                    });
                ui.end_row();

                ui.label("Z signal");
                ui.add(egui::TextEdit::singleline(&mut self.z_name).desired_width(160.0))
                    .on_hover_text("Resolved by name on the controller when the run starts");
                ui.end_row();

                ui.add_enabled(measures, egui::Label::new("Burst window"));
                let mut secs = p.window_ms as f64 / 1000.0;
                if ui
                    .add_enabled(
                        measures,
                        egui::DragValue::new(&mut secs)
                            .range(0.1..=600.0)
                            .speed(0.1)
                            .suffix(" s"),
                    )
                    .on_hover_text(
                        "Length of one measurement burst. The error of a burst falls with \
                         the window to the power 1.5, so a longer window buys more than \
                         more bursts.",
                    )
                    .changed()
                {
                    p.window_ms = (secs * 1000.0).round() as u64;
                }
                ui.end_row();

                ui.add_enabled(compensates, egui::Label::new("Bursts"));
                ui.add_enabled(
                    compensates,
                    egui::DragValue::new(&mut p.bursts).range(2..=100),
                )
                .on_hover_text(
                    "Bursts to spend, the baseline and the trial included: at least 3 \
                     when the response has to be learned.",
                );
                ui.end_row();

                ui.add_enabled(compensates, egui::Label::new("Response"));
                ui.add_enabled_ui(compensates, |ui| {
                    egui::ComboBox::from_id_salt("drift_response")
                        .selected_text(response_label(p.response))
                        .show_ui(ui, |ui| {
                            for choice in [None, Some(1.0), Some(-1.0)] {
                                ui.selectable_value(
                                    &mut p.response,
                                    choice,
                                    response_label(choice),
                                );
                            }
                        });
                });
                ui.end_row();

                ui.add_enabled(learns, egui::Label::new("Trial velocity"));
                let mut trial = p.trial_vz / PM;
                if ui
                    .add_enabled(
                        learns,
                        egui::DragValue::new(&mut trial)
                            .range(0.1..=10_000.0)
                            .speed(1.0)
                            .suffix(" pm/s"),
                    )
                    .on_hover_text(
                        "Large on purpose: with the loop closed it only ramps the Z output \
                         for one burst, and the response comes out clean.",
                    )
                    .changed()
                {
                    p.trial_vz = trial * PM;
                }
                ui.end_row();

                ui.add_enabled(
                    measures,
                    egui::Checkbox::new(&mut p.require_stream, "Require Z in the data stream"),
                )
                .on_hover_text(
                    "Off allows timed polling when the stream does not carry Z. Far noisier.",
                );
                ui.end_row();

                ui.add_enabled(polls, egui::Label::new("Polled reads per window"));
                ui.add_enabled(
                    polls,
                    egui::DragValue::new(&mut p.samples).range(3..=10_000),
                );
                ui.end_row();
            });

        if self.z_name.trim().is_empty() {
            ui.colored_label(egui::Color32::RED, "The Z signal name is empty");
        }
    }

    fn job(&self) -> Result<Box<dyn Job>, String> {
        if self.z_name.trim().is_empty() {
            return Err("the Z signal name is empty".into());
        }
        Ok(Box::new(DriftJob {
            z_name: self.z_name.trim().to_string(),
            params: self.params.clone(),
        }))
    }

    fn panel(&mut self, ui: &mut egui::Ui, view: &RunView) {
        let statuses: Vec<DriftStatusEvent> = view
            .custom(DriftStatusEvent::KIND)
            .iter()
            .filter_map(|(_, d)| serde_json::from_value(d.clone()).ok())
            .collect();
        let before = statuses.iter().find(|s| !s.after());
        let after = statuses.iter().rev().find(|s| s.after());

        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::Grid::new("drift_status")
                .num_columns(2)
                .spacing([20.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Before");
                    ui.label(
                        before
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "-".into()),
                    );
                    ui.end_row();
                    ui.label("After");
                    ui.label(after.map(ToString::to_string).unwrap_or_else(|| "-".into()));
                    ui.end_row();
                });
        });

        if let Some(m) = view
            .custom(DriftMeasuredEvent::KIND)
            .last()
            .and_then(|(_, d)| serde_json::from_value::<DriftMeasuredEvent>(d.clone()).ok())
        {
            ui.add_space(6.0);
            ui.label(m.to_string());
        }

        if let Some(c) = view
            .custom(DriftCompensatedEvent::KIND)
            .last()
            .and_then(|(_, d)| serde_json::from_value::<DriftCompensatedEvent>(d.clone()).ok())
        {
            ui.add_space(6.0);
            ui.label(c.to_string());
            match c.response {
                Some(r) => ui.label(format!(
                    "Response {r:+.2}: a positive vz {} the measured drift. Choose it in \
                     Setup next time to skip the trial burst.",
                    if r > 0.0 { "adds to" } else { "subtracts from" }
                )),
                None => ui.label("Drift was negligible from the start; nothing was changed."),
            };
        }

        let bursts: Vec<DriftBurstEvent> = view
            .custom(DriftBurstEvent::KIND)
            .iter()
            .filter_map(|(_, d)| serde_json::from_value(d.clone()).ok())
            .collect();
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
                    for b in &bursts {
                        ui.label(b.burst.to_string());
                        ui.label(format!("{:?}", b.role).to_lowercase());
                        ui.label(format!("{:+.3}", b.vz_m_s / PM));
                        ui.label(format!(
                            "{:+.3} ± {:.3}",
                            b.drift_m_s / PM,
                            b.std_err_m_s / PM
                        ));
                        ui.end_row();
                    }
                });

            let drift: Vec<[f64; 2]> = bursts
                .iter()
                .map(|b| [b.burst as f64, b.drift_m_s / PM])
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
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, prefs: &serde_json::Value) {
        if let Ok(saved) = serde_json::from_value::<DriftTool>(prefs.clone()) {
            *self = saved;
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

fn response_label(response: Option<f64>) -> &'static str {
    match response {
        None => "learn with a trial burst",
        Some(r) if r > 0.0 => "+1: positive vz adds to the drift",
        Some(_) => "-1: positive vz subtracts",
    }
}
