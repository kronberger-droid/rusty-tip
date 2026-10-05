//! Small pieces the pages share: the status dot, a path field with its file
//! dialog, a message line, and a strip chart for the last seconds of a
//! signal.

use std::path::PathBuf;

use eframe::egui;
use egui_plot::{HLine, Line, LineStyle, Plot, PlotPoint, PlotPoints};

use crate::units::number;

/// A base16 colour scheme: eight shades from background to foreground,
/// then eight accents, by the slots base16 names them.
#[allow(
    dead_code,
    reason = "the whole scheme, so a new series picks its accent from it"
)]
pub struct Base16 {
    /// Default background.
    pub base00: egui::Color32,
    /// Lighter background: bars, fields.
    pub base01: egui::Color32,
    /// Selection background.
    pub base02: egui::Color32,
    /// Comments, invisibles.
    pub base03: egui::Color32,
    /// Dark foreground.
    pub base04: egui::Color32,
    /// Default foreground.
    pub base05: egui::Color32,
    /// Light foreground.
    pub base06: egui::Color32,
    /// Lightest background.
    pub base07: egui::Color32,
    /// Red: errors.
    pub base08: egui::Color32,
    /// Urgent; red again in this scheme.
    pub base09: egui::Color32,
    /// Yellow: warnings.
    pub base0a: egui::Color32,
    /// Green.
    pub base0b: egui::Color32,
    /// Cyan.
    pub base0c: egui::Color32,
    /// Blue.
    pub base0d: egui::Color32,
    /// Magenta.
    pub base0e: egui::Color32,
    /// Brown, an accent.
    pub base0f: egui::Color32,
}

/// The desktop's scheme, a dark one, as the NixOS config's
/// `theming/base16-scheme.nix` sets it. The window takes it in dark mode.
/// A dark scheme's accents are picked against a dark background and its
/// shades do not flip into a light theme, so light mode keeps egui's own
/// and its plots take these hues darker (see [`Palette::for_theme`]).
pub const SCHEME: Base16 = Base16 {
    base00: egui::Color32::from_rgb(0x1e, 0x1e, 0x1e),
    base01: egui::Color32::from_rgb(0x2c, 0x2f, 0x33),
    base02: egui::Color32::from_rgb(0x37, 0x3c, 0x45),
    base03: egui::Color32::from_rgb(0x55, 0x55, 0x55),
    base04: egui::Color32::from_rgb(0xc0, 0xc5, 0xce),
    base05: egui::Color32::from_rgb(0xdf, 0xe1, 0xe8),
    base06: egui::Color32::from_rgb(0xef, 0xf0, 0xf1),
    base07: egui::Color32::from_rgb(0xf5, 0xf5, 0xf5),
    base08: egui::Color32::from_rgb(0xac, 0x41, 0x42),
    base09: egui::Color32::from_rgb(0xac, 0x41, 0x42),
    base0a: egui::Color32::from_rgb(0xe5, 0xb5, 0x66),
    base0b: egui::Color32::from_rgb(0x7e, 0x8d, 0x50),
    base0c: egui::Color32::from_rgb(0x6f, 0xb3, 0xad),
    base0d: egui::Color32::from_rgb(0x6c, 0x99, 0xba),
    base0e: egui::Color32::from_rgb(0x9e, 0x4e, 0x85),
    base0f: egui::Color32::from_rgb(0x8a, 0x81, 0x77),
};

/// `color` with alpha `a`, unmultiplied.
fn with_alpha(color: egui::Color32, a: u8) -> egui::Color32 {
    let [r, g, b, _] = color.to_array();
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Plot colours for one theme, shared by every tool's plots.
pub struct Palette {
    /// The first series: what the tool is about.
    pub first: egui::Color32,
    /// A second series, or a control signal.
    pub second: egui::Color32,
    /// A band of acceptable values.
    pub bounds: egui::Color32,
}

impl Palette {
    /// The scheme's accents, so the plots sit with the buttons instead of
    /// shouting over them. Dark mode takes base0D blue, base0A amber and
    /// base0B green as they are; on white those wash out, so light mode
    /// gets the same hues darker, and far more opaque bounds. That is what
    /// makes a screenshot survive being printed.
    pub fn for_theme(dark_mode: bool) -> Self {
        if dark_mode {
            Self {
                first: SCHEME.base0d,
                second: SCHEME.base0a,
                bounds: with_alpha(SCHEME.base0b, 110),
            }
        } else {
            Self {
                first: egui::Color32::from_rgb(0x3d, 0x67, 0x87),
                second: egui::Color32::from_rgb(0x9a, 0x6e, 0x22),
                bounds: egui::Color32::from_rgba_unmultiplied(0x5a, 0x66, 0x33, 190),
            }
        }
    }
}

/// Width of every plot's y-axis strip, fixed so stacked plots share one
/// x scale whatever their tick labels are (`-10.00 Hz` against `4.000 V`).
pub const Y_AXIS_WIDTH: f32 = 84.0;

/// Spacing for the whole window. egui's defaults are sized for dense
/// tool panels; a lab window is read at arm's length and clicked in
/// gloves, so buttons get room and rows breathe.
pub fn apply_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        let s = &mut style.spacing;
        s.button_padding = egui::vec2(10.0, 4.0);
        s.interact_size.y = 26.0;
        s.item_spacing = egui::vec2(8.0, 6.0);
    });
    ctx.style_mut_of(egui::Theme::Dark, |style| apply_scheme(&mut style.visuals));
}

/// Dress dark mode in [`SCHEME`]: backgrounds from the dark shades, text
/// from the light ones, accents for links, selection, warnings and errors.
/// Buttons that carry a [`Tone`] keep their own fills.
fn apply_scheme(v: &mut egui::Visuals) {
    let s = &SCHEME;
    v.panel_fill = s.base00;
    v.window_fill = s.base01;
    v.extreme_bg_color = s.base01;
    v.faint_bg_color = s.base01;
    v.code_bg_color = s.base01;
    v.window_stroke.color = s.base02;
    v.hyperlink_color = s.base0d;
    v.warn_fg_color = s.base0a;
    v.error_fg_color = s.base08;
    v.weak_text_color = Some(s.base04);
    v.selection.bg_fill = with_alpha(s.base0d, 90);
    v.selection.stroke.color = s.base06;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = s.base00;
    w.noninteractive.bg_stroke.color = s.base02;
    w.noninteractive.fg_stroke.color = s.base05;
    for (state, fill, text) in [
        (&mut w.inactive, s.base02, s.base05),
        (&mut w.hovered, s.base03, s.base06),
        (&mut w.active, s.base03, s.base07),
        (&mut w.open, s.base02, s.base06),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.fg_stroke.color = text;
    }
}

/// What a button does, which its colour says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// The page's main action: Start, Connect.
    Primary,
    /// Stops or cuts something off: Stop, Disconnect, Delete.
    Danger,
    /// Writes to the controller: Apply, Apply preset, Switch on.
    Write,
}

/// A button coloured by what it does. egui greys a disabled one out on top.
pub fn toned(ui: &egui::Ui, text: impl Into<String>, tone: Tone) -> egui::Button<'static> {
    let dark = ui.visuals().dark_mode;
    let (fill, text_color) = match (tone, dark) {
        (Tone::Primary, true) => (egui::Color32::from_rgb(30, 110, 60), egui::Color32::WHITE),
        (Tone::Primary, false) => (egui::Color32::from_rgb(40, 140, 75), egui::Color32::WHITE),
        (Tone::Danger, true) => (egui::Color32::from_rgb(150, 40, 40), egui::Color32::WHITE),
        (Tone::Danger, false) => (egui::Color32::from_rgb(190, 50, 50), egui::Color32::WHITE),
        (Tone::Write, true) => (egui::Color32::from_rgb(140, 95, 20), egui::Color32::WHITE),
        (Tone::Write, false) => (egui::Color32::from_rgb(165, 95, 0), egui::Color32::WHITE),
    };
    egui::Button::new(egui::RichText::new(text.into()).color(text_color).strong()).fill(fill)
}

/// [`toned`] while `enabled`, a plain button otherwise: a greyed-out
/// colour still reads as live, so a button that cannot act drops it. Pass
/// the same flag to `add_enabled`.
pub fn toned_if(
    ui: &egui::Ui,
    enabled: bool,
    text: impl Into<String>,
    tone: Tone,
) -> egui::Button<'static> {
    if enabled {
        toned(ui, text, tone)
    } else {
        egui::Button::new(text.into())
    }
}

/// A section heading, with what the section is about on hover rather than
/// as a paragraph under it.
pub fn section(ui: &mut egui::Ui, title: &str, help: Option<&str>) {
    ui.add_space(6.0);
    let response = ui.label(egui::RichText::new(title).heading().size(17.0));
    if let Some(help) = help {
        response.on_hover_text(help);
    }
    ui.add_space(2.0);
}

/// A label and a value, the value large, for the numbers a page is about.
pub fn stat(ui: &mut egui::Ui, label: &str, value: impl Into<egui::WidgetText>) -> egui::Response {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(label).weak().size(12.0));
        ui.label(value);
    })
    .response
}

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
    let response = ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(width)
            .hint_text("none, or … to pick one"),
    );
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
            ui.colored_label(ui.visuals().error_fg_color, &note.text);
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
    pub fn show(&self, ui: &mut egui::Ui, points: impl IntoIterator<Item = [f64; 2]>) {
        let points: Vec<PlotPoint> = points.into_iter().map(PlotPoint::from).collect();
        let unit = self.unit.to_string();
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
            .custom_y_axes(vec![
                egui_plot::AxisHints::new_y()
                    .min_thickness(Y_AXIS_WIDTH)
                    .formatter(move |mark, _| format!("{} {unit}", number(mark.value))),
            ])
            .show(ui, |plot_ui| {
                if let Some(sp) = self.setpoint {
                    plot_ui.hline(
                        HLine::new("Setpoint", sp)
                            .color(self.color.gamma_multiply(0.6))
                            .style(LineStyle::Dashed { length: 5.0 }),
                    );
                }
                plot_ui.line(Line::new("Signal", PlotPoints::Owned(points)).color(self.color));
            });
    }
}
