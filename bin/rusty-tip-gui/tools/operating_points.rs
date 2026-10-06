//! The Controllers tool's operating points page: the saved points as a
//! list, the picked one beside it as a table against what the controller
//! holds now, and one button to apply it.
//!
//! "Now" is the last state any capture or apply of this tool read back.
//! It is kept across other jobs, with the time it was read, since the
//! run view only ever holds the latest run.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eframe::egui;
use serde_json::Value;

use rusty_tip::controllers::{ControllerId, ControllerParams, ProfileEntry};
use rusty_tip::experiment_log::LogEvent;
use rusty_tip::operating_point::{
    ApplyOperatingPoint, CaptureOperatingPoint, DEFAULT_OPERATING_POINTS_FILE, OperatingPoint,
    OperatingPointApplied, OperatingPointStore, OperatingState, ScanSettings,
};

use super::SetupCx;
use super::controllers::kind_key;
use crate::form::SchemaForm;
use crate::run_view::RunView;
use crate::units::{format_si, number};
use crate::widgets::{Note, Palette, Tone, note, section, toned_if};

/// What the controller held when it was last read, and when.
#[derive(Debug, Clone)]
struct Now {
    state: OperatingState,
    at: String,
}

/// A save waiting for the capture it started. The first run to start
/// after the click decides it: a capture that finished saves, anything
/// else drops it, so an apply's read-back is never saved by mistake.
#[derive(Debug, Clone)]
struct PendingSave {
    name: String,
    note: String,
    /// The run view's start when the save was asked for.
    after: Option<f64>,
}

#[derive(Default)]
pub struct PointsPage {
    /// The operating points on file, and which file that was.
    points: Vec<OperatingPoint>,
    path: Option<PathBuf>,
    /// The point picked in the list, by name.
    pub selected: Option<String>,
    now: Option<Now>,
    /// Which run's `operating_point/read` rows have been taken: the run's
    /// start time and how many rows.
    seen: (Option<f64>, usize),
    pending: Option<PendingSave>,
    /// The point whose Delete was clicked once, waiting for the second.
    confirm_delete: Option<String>,
    /// The name and note to save the current state under.
    name: String,
    note: String,
    message: Option<Note>,
}

impl PointsPage {
    /// Read the operating points file again, remembering which file it was.
    fn refresh(&mut self, path: &Path) {
        match OperatingPointStore::new(path).list() {
            Ok(points) => self.points = points,
            Err(e) => {
                self.points.clear();
                self.message = Some(Note::err(e));
            }
        }
        self.path = Some(path.to_path_buf());
    }

    fn selected_index(&self) -> Option<usize> {
        let name = self.selected.as_deref()?;
        self.points.iter().position(|p| p.is_named(name))
    }

    /// Take a newly read state as "now", and settle a pending save once
    /// the run after it has finished.
    pub fn take_state(&mut self, view: &RunView, path: &Path) {
        let start = view.started_at();
        if start != self.seen.0 {
            self.seen = (start, 0);
        }
        let rows = view.custom(OperatingState::KIND);
        let read = rows
            .get(self.seen.1..)
            .and_then(<[_]>::last)
            .and_then(|(_, data)| serde_json::from_value::<OperatingState>(data.clone()).ok());
        if let Some(state) = read {
            let at = chrono::Local::now().format("%H:%M:%S").to_string();
            self.now = Some(Now { state, at });
        }
        self.seen.1 = rows.len();

        let decided = self
            .pending
            .as_ref()
            .is_some_and(|p| start.is_some() && start != p.after && view.finish.is_some());
        if !decided {
            return;
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        let applied = !view.custom(OperatingPointApplied::KIND).is_empty();
        let state = rows
            .last()
            .and_then(|(_, data)| serde_json::from_value::<OperatingState>(data.clone()).ok());
        match (applied, state) {
            (false, Some(state)) => self.save(path, &pending, state),
            _ => {
                self.message = Some(Note::err(format!(
                    "Not saved: the read for {:?} did not finish",
                    pending.name
                )))
            }
        }
    }

    fn save(&mut self, path: &Path, pending: &PendingSave, state: OperatingState) {
        let saved_at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        let point = OperatingPoint::from_state(&pending.name, &pending.note, Some(saved_at), state);
        match OperatingPointStore::new(path).put(point.clone()) {
            Ok(()) => {
                self.message = Some(Note::ok(format!("Saved {:?}", point.name)));
                self.selected = Some(point.name);
                self.name.clear();
                self.note.clear();
            }
            Err(e) => self.message = Some(Note::err(e)),
        }
        self.refresh(path);
    }

    fn delete(&mut self, path: &Path, name: &str) {
        match OperatingPointStore::new(path).remove(name) {
            Ok(()) => self.message = Some(Note::ok(format!("Deleted {name:?}"))),
            Err(e) => self.message = Some(Note::err(e)),
        }
        self.confirm_delete = None;
        self.refresh(path);
    }

    /// The file the connection names, or the default before one is set.
    pub fn path_of(cx: &SetupCx<'_>) -> PathBuf {
        match &cx.connection {
            Ok(s) => s.operating_points_file.clone(),
            Err(_) => PathBuf::from(DEFAULT_OPERATING_POINTS_FILE),
        }
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        cx: &mut SetupCx<'_>,
        forms: &BTreeMap<&'static str, SchemaForm>,
    ) {
        let path = Self::path_of(cx);
        if self.path.as_deref() != Some(path.as_path()) {
            self.refresh(&path);
        }
        if self.selected_index().is_none() {
            self.selected = self.points.first().map(|p| p.name.clone());
        }
        note(ui, &self.message);

        let list_width = (ui.available_width() * 0.34).clamp(220.0, 320.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(list_width);
                self.render_list(ui, cx, forms, &path);
            });
            ui.separator();
            ui.vertical(|ui| self.render_detail(ui, cx, forms, &path));
        });
    }

    /// The saved points as cards, and the form to save the current state.
    fn render_list(
        &mut self,
        ui: &mut egui::Ui,
        cx: &mut SetupCx<'_>,
        forms: &BTreeMap<&'static str, SchemaForm>,
        path: &Path,
    ) {
        ui.horizontal(|ui| {
            section(
                ui,
                "Saved",
                Some(&format!(
                    "Every loop with its setpoint, the bias, and the scan's size, angle, \
                     resolution and speed, under one name, from {}",
                    path.display()
                )),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("Refresh")
                    .on_hover_text("Read the operating points file again")
                    .clicked()
                {
                    self.refresh(path);
                }
            });
        });

        let height = (ui.available_height() - 190.0).max(140.0);
        egui::ScrollArea::vertical()
            .id_salt("operating_points_list")
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if self.points.is_empty() {
                    ui.label(egui::RichText::new("none saved yet").weak());
                }
                let mut picked = None;
                let now = self.now.as_ref().map(|n| &n.state);
                for p in &self.points {
                    let selected = self.selected.as_deref() == Some(p.name.as_str());
                    let current = now.is_some_and(|n| {
                        comparison(p, Some(n), forms)
                            .iter()
                            .flat_map(|g| &g.rows)
                            .all(|r| !r.differs())
                    });
                    if card(ui, p, selected, current).clicked() {
                        picked = Some(p.name.clone());
                    }
                    ui.add_space(2.0);
                }
                if let Some(name) = picked {
                    self.selected = Some(name);
                    self.confirm_delete = None;
                }
            });

        section(
            ui,
            "Save current",
            Some(
                "Reads every loop, the bias and the scan from the controller and saves \
                 them under this name. A point of the same name is replaced.",
            ),
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.name)
                .desired_width(f32::INFINITY)
                .hint_text("name"),
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.note)
                .desired_width(f32::INFINITY)
                .hint_text("note: sample, tip state"),
        );
        ui.horizontal(|ui| {
            let can_save = cx.can_run && !self.name.trim().is_empty() && self.pending.is_none();
            if ui
                .add_enabled(can_save, egui::Button::new("Read and save"))
                .on_disabled_hover_text("Give it a name, connect, and let any run finish")
                .clicked()
            {
                self.pending = Some(PendingSave {
                    name: self.name.trim().to_string(),
                    note: self.note.trim().to_string(),
                    after: cx.view.started_at(),
                });
                self.message = None;
                cx.run = Some(Box::new(CaptureOperatingPoint));
            }
            if self.pending.is_some() {
                ui.spinner();
                ui.label(egui::RichText::new("reading").weak());
            }
        });
    }

    /// The picked point against now, with Apply, Read current and Delete.
    fn render_detail(
        &mut self,
        ui: &mut egui::Ui,
        cx: &mut SetupCx<'_>,
        forms: &BTreeMap<&'static str, SchemaForm>,
        path: &Path,
    ) {
        let Some(i) = self.selected_index() else {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Save the controller's current state to start a list").weak(),
            );
            return;
        };
        let point = self.points[i].clone();
        let groups = comparison(&point, self.now.as_ref().map(|n| &n.state), forms);
        let differing = groups
            .iter()
            .flat_map(|g| &g.rows)
            .filter(|r| r.differs())
            .count();

        ui.add_space(4.0);
        ui.label(egui::RichText::new(&point.name).heading().size(20.0));
        let mut about = Vec::new();
        if !point.note.is_empty() {
            about.push(point.note.clone());
        }
        if let Some(at) = &point.saved_at {
            about.push(format!("saved {at}"));
        }
        if !about.is_empty() {
            ui.label(egui::RichText::new(about.join(" · ")).weak());
        }
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    cx.can_run,
                    toned_if(ui, cx.can_run, "Apply", Tone::Write).min_size(egui::vec2(96.0, 30.0)),
                )
                .on_hover_text(
                    "Write the scan settings, every loop with its setpoint, then the bias, \
                     and read it all back. Keeps the frame's centre, switches no loop on or \
                     off, and refuses while a scan runs.",
                )
                .on_disabled_hover_text("Connect first, and let any run finish")
                .clicked()
            {
                self.message = None;
                cx.run = Some(Box::new(ApplyOperatingPoint {
                    point: point.clone(),
                }));
            }
            match &self.now {
                None => ui.label(egui::RichText::new("now not read").weak()),
                Some(_) if differing == 0 => ui.label("matches now"),
                Some(_) => ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "{differing} value{} differ{} from now",
                        if differing == 1 { "" } else { "s" },
                        if differing == 1 { "s" } else { "" },
                    ),
                ),
            };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let confirming = self.confirm_delete.as_deref() == Some(point.name.as_str());
                let delete = if confirming {
                    ui.add(toned_if(ui, true, "Delete for good", Tone::Danger))
                        .on_hover_text("Click again to remove it from the file")
                } else {
                    ui.button("Delete")
                        .on_hover_text("Remove it from the operating points file")
                };
                if delete.clicked() {
                    if confirming {
                        self.delete(path, &point.name);
                    } else {
                        self.confirm_delete = Some(point.name.clone());
                    }
                }
                if ui
                    .add_enabled(cx.can_run, egui::Button::new("Read current"))
                    .on_hover_text("Read every loop, the bias and the scan, for the Now column")
                    .on_disabled_hover_text("Connect first, and let any run finish")
                    .clicked()
                {
                    cx.run = Some(Box::new(CaptureOperatingPoint));
                }
            });
        });
        ui.add_space(6.0);

        egui::ScrollArea::vertical()
            .id_salt("operating_point_detail")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                // One grid per group, so the stripes start afresh under each
                // title; fixed column widths keep the groups lined up.
                let columns = if self.now.is_some() { 3 } else { 2 };
                let gap = 12.0;
                let width = ((ui.available_width() - gap * (columns as f32 - 1.0))
                    / columns as f32)
                    .clamp(96.0, MAX_COLUMN_WIDTH);
                let grid = |id: &str| {
                    egui::Grid::new(("operating_point_vs_now", id.to_owned()))
                        .num_columns(columns)
                        .striped(true)
                        .min_col_width(width)
                        .max_col_width(width)
                        .spacing([gap, 5.0])
                };
                grid("head").show(ui, |ui| {
                    ui.label("");
                    ui.label(egui::RichText::new("Point").strong());
                    if let Some(now) = &self.now {
                        ui.label(egui::RichText::new(format!("Now · {}", now.at)).strong())
                            .on_hover_text("The last capture or apply, read back");
                    }
                    ui.end_row();
                });
                let warn = ui.visuals().warn_fg_color;
                for group in &groups {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(&group.title).strong().size(14.0));
                    grid(&group.title).show(ui, |ui| {
                        for row in &group.rows {
                            ui.label(egui::RichText::new(&row.label).weak());
                            ui.label(&row.point);
                            if let Some(now) = &row.now {
                                if row.differs() {
                                    ui.colored_label(warn, now);
                                } else {
                                    ui.label(now);
                                }
                            }
                            ui.end_row();
                        }
                    });
                }
            });
    }
}

/// Widest a column of the point-against-now table gets; a narrow window
/// shares what it has.
const MAX_COLUMN_WIDTH: f32 = 160.0;

/// One saved point in the list: its name, the numbers that tell it apart,
/// and its note. `current` marks the one the controller holds now.
fn card(ui: &mut egui::Ui, p: &OperatingPoint, selected: bool, current: bool) -> egui::Response {
    let visuals = ui.visuals();
    let fill = if selected {
        visuals.selection.bg_fill
    } else {
        visuals.faint_bg_color
    };
    let hover_stroke = visuals.widgets.hovered.bg_stroke;
    let frame = egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .corner_radius(4)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&p.name).strong().size(15.0));
                if current {
                    let green = Palette::for_theme(ui.visuals().dark_mode)
                        .bounds
                        .to_opaque();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new("current").size(12.0).color(green))
                            .on_hover_text("Every value matches the controller's, as last read");
                    });
                }
            });
            ui.label(egui::RichText::new(point_summary(p)).size(12.0));
            if !p.note.is_empty() {
                ui.label(egui::RichText::new(&p.note).weak().italics().size(12.0));
            }
        });
    let rect = frame.response.rect;
    let response = ui.interact(rect, ui.id().with(("point", &p.name)), egui::Sense::click());
    if response.hovered() && !selected {
        ui.painter()
            .rect_stroke(rect, 4, hover_stroke, egui::StrokeKind::Inside);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Bias, Z setpoint and frame: what tells one point from another at a
/// glance.
fn point_summary(p: &OperatingPoint) -> String {
    let mut parts = vec![format_si(p.bias_v, "V")];
    if let Some(z) = z_setpoint(&p.controllers) {
        parts.push(z);
    }
    parts.push(frame_text(&p.scan));
    parts.join(" · ")
}

/// The Z loop's setpoint in its input's unit, if there is a Z loop.
pub fn z_setpoint(controllers: &[ProfileEntry]) -> Option<String> {
    controllers.iter().find_map(|e| match &e.params {
        ControllerParams::Z(z) => Some(match z.input().unit() {
            Some(unit) => format_si(z.setpoint, unit),
            None => number(z.setpoint),
        }),
        _ => None,
    })
}

/// Width by height, the unit once when both sides share it:
/// `50.00 × 50.00 nm`, but `500.0 nm × 1.000 µm`.
pub fn frame_text(scan: &ScanSettings) -> String {
    let (w, h) = (format_si(scan.width_m, "m"), format_si(scan.height_m, "m"));
    let mut text = match (w.split_once(' '), h.split_once(' ')) {
        (Some((w_number, w_unit)), Some((_, h_unit))) if w_unit == h_unit => {
            format!("{w_number} × {h}")
        }
        _ => format!("{w} × {h}"),
    };
    if scan.angle_deg != 0.0 {
        text.push_str(&format!(", {}°", number(scan.angle_deg)));
    }
    text
}

/// One value of a point, beside the controller's if it has been read.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    label: String,
    point: String,
    now: Option<String>,
}

impl Row {
    /// Compared as shown, so a read-back that differs below the shown
    /// precision still matches.
    fn differs(&self) -> bool {
        self.now.as_ref().is_some_and(|n| *n != self.point)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Group {
    title: String,
    rows: Vec<Row>,
}

/// A point's values by group, bias and scan first, then each loop it
/// holds, each beside what `now` holds when it has been read.
fn comparison(
    point: &OperatingPoint,
    now: Option<&OperatingState>,
    forms: &BTreeMap<&'static str, SchemaForm>,
) -> Vec<Group> {
    let scan_rows = |bias_v: f64, scan: &ScanSettings| {
        vec![
            ("Bias", format_si(bias_v, "V")),
            ("Frame", frame_text(scan)),
            ("Pixels", format!("{} × {}", scan.pixels, scan.lines)),
            (
                "Speed",
                format!("{}/s", format_si(scan.speed.forward_m_s, "m")),
            ),
            (
                "Time per line",
                format_si(scan.speed.forward_time_per_line_s, "s"),
            ),
        ]
    };
    let mut groups = vec![Group {
        title: "Bias and scan".into(),
        rows: pair_rows(
            scan_rows(point.bias_v, &point.scan),
            now.map(|n| scan_rows(n.bias_v, &n.scan)),
        ),
    }];
    for entry in &point.controllers {
        let theirs = now.map(|n| {
            n.controllers
                .iter()
                .find(|e| e.id == entry.id)
                .map(|e| param_rows(e.id, &e.params, forms))
                .unwrap_or_default()
        });
        let mine = param_rows(entry.id, &entry.params, forms);
        let rows = mine
            .into_iter()
            .map(|(label, point)| {
                let now = theirs.as_ref().map(|t| {
                    t.iter()
                        .find(|(l, _)| *l == label)
                        .map_or_else(|| "not read".to_string(), |(_, v)| v.clone())
                });
                Row { label, point, now }
            })
            .collect();
        groups.push(Group {
            title: entry.id.to_string(),
            rows,
        });
    }
    groups
}

fn pair_rows(mine: Vec<(&str, String)>, theirs: Option<Vec<(&str, String)>>) -> Vec<Row> {
    mine.into_iter()
        .enumerate()
        .map(|(i, (label, point))| Row {
            label: label.to_string(),
            point,
            now: theirs.as_ref().map(|t| t[i].1.clone()),
        })
        .collect()
}

/// A loop's parameters as `(label, value)` in its form's order and units.
fn param_rows(
    id: ControllerId,
    params: &ControllerParams,
    forms: &BTreeMap<&'static str, SchemaForm>,
) -> Vec<(String, String)> {
    let form = forms.get(kind_key(id));
    let fields = params.to_fields();
    let Some(fields) = fields.as_object() else {
        return Vec::new();
    };
    let keys: Vec<String> = match form {
        Some(f) => f.keys(),
        None => fields.keys().cloned().collect(),
    };
    keys.iter()
        .filter_map(|key| {
            let value = fields.get(key)?;
            let label = form.map_or_else(|| key.clone(), |f| f.label_of(key));
            let text = match params {
                ControllerParams::Z(_) if key == "setpoint" => z_setpoint(&[ProfileEntry {
                    id,
                    params: params.clone(),
                }])?,
                _ => value_text(form.and_then(|f| f.unit_of(key)), value),
            };
            Some((label, text))
        })
        .collect()
}

/// A field's value for the table: numbers in their unit, a pair of them
/// as a range, switches as on or off.
fn value_text(unit: Option<&str>, value: &Value) -> String {
    let one = |v: f64| unit.map_or_else(|| number(v), |u| format_si(v, u));
    match value {
        Value::Null => "unset".into(),
        Value::Bool(on) => if *on { "on" } else { "off" }.into(),
        Value::String(s) if s.is_empty() => "-".into(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), one),
        Value::Array(items) => {
            let numbers: Option<Vec<f64>> = items.iter().map(Value::as_f64).collect();
            match numbers {
                Some(n) => n.into_iter().map(one).collect::<Vec<_>>().join(" to "),
                None => value.to_string(),
            }
        }
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tip::controllers::ZControllerParams;
    use rusty_tip::operating_point::{KeepConstant, ScanSpeed};

    fn forms() -> BTreeMap<&'static str, SchemaForm> {
        [ControllerId::Z]
            .into_iter()
            .map(|id| {
                (
                    kind_key(id),
                    SchemaForm::new(ControllerParams::schema_for(id)),
                )
            })
            .collect()
    }

    fn scan(width_m: f64) -> ScanSettings {
        ScanSettings {
            width_m,
            height_m: 50e-9,
            angle_deg: 0.0,
            pixels: 256,
            lines: 256,
            speed: ScanSpeed {
                forward_m_s: 5e-9,
                backward_m_s: 5e-9,
                forward_time_per_line_s: 1.0,
                backward_time_per_line_s: 1.0,
                keep: KeepConstant::LinearSpeed,
                backward_ratio: 1.0,
            },
        }
    }

    fn z(setpoint: f64, p_gain_m: f64) -> ProfileEntry {
        ProfileEntry {
            id: ControllerId::Z,
            params: ControllerParams::Z(ZControllerParams {
                setpoint,
                p_gain_m,
                ..Default::default()
            }),
        }
    }

    fn point(width_m: f64, entry: ProfileEntry) -> OperatingPoint {
        let state = OperatingState {
            bias_v: 0.5,
            scan: scan(width_m),
            controllers: vec![entry],
        };
        OperatingPoint::from_state("a", "", None, state)
    }

    fn row<'a>(groups: &'a [Group], group: &str, label: &str) -> &'a Row {
        groups
            .iter()
            .find(|g| g.title == group)
            .and_then(|g| g.rows.iter().find(|r| r.label == label))
            .unwrap_or_else(|| panic!("no {label} under {group}"))
    }

    #[test]
    fn without_a_read_nothing_differs_and_there_is_no_now() {
        let groups = comparison(&point(50e-9, z(50e-12, 2e-10)), None, &forms());
        assert_eq!(groups[0].title, "Bias and scan");
        assert_eq!(groups[1].title, "Z-controller");
        assert!(groups.iter().flat_map(|g| &g.rows).all(|r| r.now.is_none()));
        assert!(groups.iter().flat_map(|g| &g.rows).all(|r| !r.differs()));
    }

    #[test]
    fn the_rows_that_differ_from_now_are_the_changed_ones() {
        let p = point(50e-9, z(50e-12, 2e-10));
        let now = OperatingState {
            bias_v: 0.5,
            scan: scan(100e-9),
            controllers: vec![z(50e-12, 3e-10)],
        };
        let groups = comparison(&p, Some(&now), &forms());
        let differing: Vec<&str> = groups
            .iter()
            .flat_map(|g| &g.rows)
            .filter(|r| r.differs())
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(differing, ["Frame", "P gain"]);
        assert_eq!(
            row(&groups, "Bias and scan", "Bias").now.as_deref(),
            Some("500.0 mV")
        );
    }

    #[test]
    fn a_loop_the_read_did_not_find_says_so() {
        let p = point(50e-9, z(50e-12, 2e-10));
        let now = OperatingState {
            bias_v: 0.5,
            scan: scan(50e-9),
            controllers: Vec::new(),
        };
        let groups = comparison(&p, Some(&now), &forms());
        assert_eq!(
            row(&groups, "Z-controller", "P gain").now.as_deref(),
            Some("not read")
        );
    }

    #[test]
    fn a_frame_names_its_unit_once_when_both_sides_share_it() {
        assert_eq!(frame_text(&scan(50e-9)), "50.00 × 50.00 nm");
        assert_eq!(frame_text(&scan(1e-6)), "1.000 µm × 50.00 nm");
    }

    #[test]
    fn values_read_as_units_ranges_and_switches() {
        assert_eq!(value_text(Some("m"), &serde_json::json!(2e-10)), "200.0 pm");
        assert_eq!(value_text(None, &serde_json::json!(true)), "on");
        assert_eq!(value_text(Some("m/s"), &Value::Null), "unset");
        assert_eq!(value_text(None, &serde_json::json!("")), "-");
        assert_eq!(
            value_text(Some("m"), &serde_json::json!([-1e-6, 2e-6])),
            "-1.000 µm to 2.000 µm"
        );
    }
}
