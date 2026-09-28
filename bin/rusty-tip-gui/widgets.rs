//! Small pieces the pages share: the status dot, a path field with its file
//! dialog, a message line, and a strip chart for the last seconds of a
//! signal.

use std::path::PathBuf;

use eframe::egui;
use egui_plot::{HLine, Line, LineStyle, Plot, PlotPoints};

/// A filled circle the height of a line of text, the one place the
/// workbench uses colour for state.
pub fn status_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let size = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), size * 0.32, color);
}

/// A text field for a path with a `…` button that opens `dialog`. Returns
/// the field's response and whether the dialog picked a path, which is
/// then in `text`.
pub fn path_field(
    ui: &mut egui::Ui,
    text: &mut String,
    width: f32,
    dialog: impl FnOnce() -> Option<PathBuf>,
) -> (egui::Response, bool) {
    let response = ui.add(egui::TextEdit::singleline(text).desired_width(width));
    let mut picked = false;
    if ui.button("…").clicked()
        && let Some(path) = dialog()
    {
        *text = path.display().to_string();
        picked = true;
    }
    (response, picked)
}

/// A line of feedback under a control: plain for news, red for a problem.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub text: String,
    pub is_error: bool,
}

impl Note {
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    pub fn err(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

pub fn note(ui: &mut egui::Ui, note: &Option<Note>) {
    if let Some(note) = note {
        if note.is_error {
            ui.colored_label(egui::Color32::RED, &note.text);
        } else {
            ui.label(&note.text);
        }
    }
}

/// A strip chart: the last seconds of a signal, time running to zero at
/// the right, the value in whatever unit the caller scaled to, and a
/// dashed line where a setpoint sits. Not interactive.
pub struct StripChart<'a> {
    pub id: &'a str,
    pub unit: &'a str,
    pub setpoint: Option<f64>,
    pub window_s: f64,
    pub size: [f32; 2],
    pub color: egui::Color32,
}

impl StripChart<'_> {
    pub fn show(&self, ui: &mut egui::Ui, points: Vec<[f64; 2]>) {
        Plot::new(self.id)
            .width(self.size[0])
            .height(self.size[1])
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .show_x(false)
            .include_x(-self.window_s)
            .include_x(0.0)
            .x_axis_label("s")
            .y_axis_label(self.unit)
            .show(ui, |plot_ui| {
                if let Some(sp) = self.setpoint {
                    plot_ui.hline(
                        HLine::new("Setpoint", sp)
                            .color(self.color.gamma_multiply(0.6))
                            .style(LineStyle::Dashed { length: 5.0 }),
                    );
                }
                plot_ui.line(Line::new("Signal", PlotPoints::from(points)).color(self.color));
            });
    }
}
