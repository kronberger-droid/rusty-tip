//! Small pieces the pages share: the status dot, a path field with its file
//! dialog, a message line, and the SI prefix table behind display units.

use std::path::PathBuf;

use eframe::egui;

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

/// How many of `display_unit` make one base unit, from the SI prefix it
/// starts with: `pA` gives 1e12, `mV` gives 1e3, `Hz` gives nothing.
pub fn prefix_scale(display_unit: &str) -> Option<f64> {
    let first = display_unit.chars().next()?;
    // Only when the rest is a plain base unit, so `m/s` is not "milli".
    let rest = &display_unit[first.len_utf8()..];
    if rest.is_empty() || !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    match first {
        'p' => Some(1e12),
        'n' => Some(1e9),
        'µ' | 'u' => Some(1e6),
        'm' => Some(1e3),
        'k' => Some(1e-3),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_scale_and_bare_units_do_not() {
        assert_eq!(prefix_scale("pA"), Some(1e12));
        assert_eq!(prefix_scale("mV"), Some(1e3));
        assert_eq!(prefix_scale("nm/s"), Some(1e9));
        assert_eq!(prefix_scale("m/s"), None, "metres per second is not milli");
        assert_eq!(prefix_scale("Hz"), None);
        assert_eq!(prefix_scale("V"), None);
    }
}
