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
use crate::units::format_tick;
use crate::widgets::{Palette, Y_AXIS_WIDTH, section, stat};

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

    fn setup(&mut self, ui: &mut egui::Ui, _cx: &mut SetupCx<'_>) {
        section(
            ui,
            "Measurement",
            Some(
                "Measures how fast Z drifts with the feedback closed and the scan stopped, \
                 and can leave the controller compensating for it. Position the tip on flat, \
                 quiet ground first; the tip is left where it is.",
            ),
        );

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
                     when the response has to be learned. The bursts spent waiting for \
                     the drift to settle come on top.",
                );
                ui.end_row();

                ui.add_enabled(compensates, egui::Label::new("Settle for at most"));
                let mut settle = p.settle_max_ms as f64 / 1000.0;
                if ui
                    .add_enabled(
                        compensates,
                        egui::DragValue::new(&mut settle)
                            .range(0.0..=1800.0)
                            .speed(1.0)
                            .suffix(" s"),
                    )
                    .on_hover_text(
                        "Before correcting, bursts are taken with nothing changed until \
                         three in a row agree, so creep from the last approach or Z move \
                         has died out. A drift still changing after this long stops the \
                         run with nothing touched. 0 corrects from the first burst.",
                    )
                    .changed()
                {
                    p.settle_max_ms = (settle * 1000.0).round() as u64;
                }
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
        let bursts: Vec<DriftBurstEvent> = view
            .custom(DriftBurstEvent::KIND)
            .iter()
            .filter_map(|(_, d)| serde_json::from_value(d.clone()).ok())
            .collect();
        let colors = Palette::for_theme(ui.visuals().dark_mode);

        // The plot first, where it stays put; the text below it grows with
        // the run.
        if !bursts.is_empty() {
            section(
                ui,
                "Drift per burst",
                Some("Each burst's fitted Z drift, with its standard error as a bar."),
            );
            let drift: Vec<[f64; 2]> = bursts
                .iter()
                .map(|b| [b.burst as f64, b.drift_m_s / PM])
                .collect();
            let color = colors.first;
            let zero = ui.visuals().weak_text_color();
            let last = bursts.len() as f64;
            Plot::new("drift_bursts_plot")
                .height(200.0)
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .include_x(0.5)
                .include_x(last + 0.5)
                .include_y(0.0)
                .x_axis_label("burst")
                .x_grid_spacer(egui_plot::uniform_grid_spacer(|_| [1.0, 5.0, 10.0]))
                .custom_y_axes(vec![
                    egui_plot::AxisHints::new_y()
                        .min_thickness(Y_AXIS_WIDTH)
                        .formatter(|mark, _| format_tick(mark.value * PM, "m/s")),
                ])
                .show(ui, |plot_ui| {
                    plot_ui.hline(
                        egui_plot::HLine::new("", 0.0)
                            .color(zero)
                            .style(egui_plot::LineStyle::Dashed { length: 4.0 }),
                    );
                    for b in &bursts {
                        let (x, y, e) = (b.burst as f64, b.drift_m_s / PM, b.std_err_m_s / PM);
                        plot_ui.line(
                            Line::new("", PlotPoints::from(vec![[x, y - e], [x, y + e]]))
                                .color(color.gamma_multiply(0.6))
                                .width(2.0),
                        );
                    }
                    plot_ui.line(Line::new("drift", PlotPoints::from(drift.clone())).color(color));
                    plot_ui.points(
                        Points::new("bursts", PlotPoints::from(drift))
                            .color(color)
                            .radius(3.0_f32),
                    );
                });
        }

        let measured = view
            .custom(DriftMeasuredEvent::KIND)
            .last()
            .and_then(|(_, d)| serde_json::from_value::<DriftMeasuredEvent>(d.clone()).ok());
        let compensated = view
            .custom(DriftCompensatedEvent::KIND)
            .last()
            .and_then(|(_, d)| serde_json::from_value::<DriftCompensatedEvent>(d.clone()).ok());
        let rate = |v: f64, e: f64| format!("{:+.3} ± {:.3} pm/s", v / PM, e / PM);
        let value = |text: String| egui::RichText::new(text).size(16.0).monospace();

        if measured.is_some() || compensated.is_some() {
            section(ui, "Result", None);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                if let Some(m) = &measured {
                    stat(ui, "Z drift", value(rate(m.rate_m_s, m.std_err_m_s)));
                    stat(
                        ui,
                        "Samples",
                        value(format!("{} over {:.1} s", m.samples, m.window_s)),
                    );
                    stat(
                        ui,
                        "Verdict",
                        value(
                            if m.negligible {
                                "consistent with zero"
                            } else {
                                "drifting"
                            }
                            .into(),
                        ),
                    );
                }
                if let Some(c) = &compensated {
                    stat(
                        ui,
                        "Residual drift",
                        value(rate(c.residual_rate_m_s, c.residual_std_err_m_s)),
                    );
                    stat(
                        ui,
                        "vz left on",
                        value(format!("{:+.3} pm/s", c.vz_m_s / PM)),
                    );
                    stat(ui, "Bursts", value(c.bursts.to_string()));
                    let (word, color) = if c.converged {
                        ("inside error bar", colors.bounds.to_opaque())
                    } else {
                        ("outside error bar", colors.second)
                    };
                    stat(ui, "Converged", value(word.into()).color(color)).on_hover_text(
                        "Outside once is noise now and then; if it repeats, lengthen the window",
                    );
                    match c.response {
                        Some(r) => {
                            stat(ui, "Response", value(format!("{r:+.2}"))).on_hover_text(format!(
                                "A positive vz {} the measured drift. Choose it in Setup next \
                                 time to skip the trial burst.",
                                if r > 0.0 { "adds to" } else { "subtracts from" }
                            ));
                        }
                        None => {
                            stat(ui, "Response", value("not needed".into())).on_hover_text(
                                "Drift was negligible from the start; nothing was changed.",
                            );
                        }
                    }
                }
            });
        }

        if before.is_some() || after.is_some() {
            section(ui, "Compensation", Some("As the controller reported it"));
            egui::Grid::new("drift_status")
                .num_columns(6)
                .striped(true)
                .spacing([20.0, 4.0])
                .show(ui, |ui| {
                    for h in [
                        "",
                        "state",
                        "vx (pm/s)",
                        "vy (pm/s)",
                        "vz (pm/s)",
                        "saturated",
                    ] {
                        ui.label(egui::RichText::new(h).strong());
                    }
                    ui.end_row();
                    for (when, s) in [("Before", before), ("After", after)] {
                        let Some(s) = s else { continue };
                        ui.label(when);
                        ui.label(if s.enabled { "on" } else { "off" });
                        for v in [s.vx_m_s, s.vy_m_s, s.vz_m_s] {
                            ui.label(egui::RichText::new(format!("{:+.3}", v / PM)).monospace());
                        }
                        let saturated: Vec<&str> = [
                            (s.x_saturated, "x"),
                            (s.y_saturated, "y"),
                            (s.z_saturated, "z"),
                        ]
                        .into_iter()
                        .filter_map(|(on, axis)| on.then_some(axis))
                        .collect();
                        ui.label(if saturated.is_empty() {
                            "none".to_string()
                        } else {
                            saturated.join(", ")
                        })
                        .on_hover_text(format!(
                            "An axis saturates at {:.0} % of its range",
                            s.saturation_limit_percent
                        ));
                        ui.end_row();
                    }
                });
        }

        if !bursts.is_empty() {
            section(ui, "Bursts", None);
            egui::Grid::new("drift_bursts")
                .num_columns(4)
                .striped(true)
                .spacing([20.0, 4.0])
                .show(ui, |ui| {
                    for h in ["Burst", "Role", "vz (pm/s)", "Drift (pm/s)"] {
                        ui.label(egui::RichText::new(h).strong());
                    }
                    ui.end_row();
                    for b in &bursts {
                        ui.label(b.burst.to_string());
                        ui.label(format!("{:?}", b.role).to_lowercase());
                        ui.label(egui::RichText::new(format!("{:+.3}", b.vz_m_s / PM)).monospace());
                        ui.label(
                            egui::RichText::new(format!(
                                "{:+.3} ± {:.3}",
                                b.drift_m_s / PM,
                                b.std_err_m_s / PM
                            ))
                            .monospace(),
                        );
                        ui.end_row();
                    }
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
