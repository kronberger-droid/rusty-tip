//! A form drawn from a JSON Schema.
//!
//! [`SchemaForm`] renders an editable `serde_json::Value` against the
//! schema `schemars` generates for the parameter type, so a tool's setup
//! page is derived from the struct and cannot drift from it. Covered: objects
//! as sections, numbers, integers, booleans, strings, `Option<T>` as a
//! checkbox plus the field, string enums as a combo box, internally tagged
//! enums as a variant combo plus that variant's fields, fixed-size arrays
//! and tuples as a row of inputs, and lists with add and remove. Anything
//! else falls back to a raw JSON field, never a panic.
//!
//! Units: values stay SI in the value; a field annotated with `x-unit` and
//! `x-display-unit` is shown scaled (`A` as `pA`, `m` as `nm`) and converted
//! back on edit. Doc comments arrive as `description` and become hover text.
//! Validation is the tool's business: it deserializes the value into its
//! type when the run starts.
//!
//! Every `$ref` is inlined once when the form is built, so drawing walks a
//! plain tree and borrows from it.

use std::borrow::Cow;

use eframe::egui;
use serde_json::{Map, Value, json};

use crate::widgets::prefix_scale;

pub struct SchemaForm {
    root: Value,
}

impl SchemaForm {
    /// From the JSON Schema of the parameter type, as `schemars` gives it.
    pub fn new(mut schema: Value) -> Self {
        let defs = schema
            .as_object_mut()
            .and_then(|o| o.remove("$defs"))
            .unwrap_or(json!({}));
        inline_refs(&mut schema, &defs, 0);
        Self { root: schema }
    }

    /// Draw the whole value, each top-level field a collapsible section,
    /// except the top-level fields named in `hidden`: their values stay in
    /// `value` untouched, they are just not drawn, for sections another
    /// page owns. Returns whether anything was edited.
    pub fn render_except(&self, ui: &mut egui::Ui, value: &mut Value, hidden: &[&str]) -> bool {
        let mut changed = false;
        if let Some(props) = self.root.get("properties").and_then(Value::as_object) {
            let object = ensure_object(value);
            for (key, prop) in props {
                if hidden.contains(&key.as_str()) {
                    continue;
                }
                let field = object
                    .entry(key.clone())
                    .or_insert_with(|| default_for(prop));
                changed |= self.render_section(ui, prop, field, key, key);
            }
        }
        changed
    }

    /// Draw one field by its dotted path as a row of the enclosing grid,
    /// for the featured fields at the top.
    pub fn render_path(&self, ui: &mut egui::Ui, value: &mut Value, path: &str) -> bool {
        let mut schema = &self.root;
        let mut node = value;
        let mut last = path;
        for segment in path.split('.') {
            let Some(prop) = schema.get("properties").and_then(|p| p.get(segment)) else {
                ui.colored_label(egui::Color32::RED, format!("no field {path}"));
                ui.end_row();
                return false;
            };
            let object = ensure_object(node);
            node = object
                .entry(segment.to_string())
                .or_insert_with(|| default_for(prop));
            schema = prop;
            last = segment;
        }
        self.render_field(ui, schema, node, last, path)
    }

    /// A value with every field at its schema default.
    #[cfg(test)]
    pub fn default_value(&self) -> Value {
        default_for(&self.root)
    }

    // -- Rendering --

    /// A top-level field: a collapsible section holding an object's grid, or
    /// a one-row grid for anything else.
    fn render_section(
        &self,
        ui: &mut egui::Ui,
        schema: &Value,
        value: &mut Value,
        key: &str,
        id: &str,
    ) -> bool {
        let mut changed = false;
        let response = egui::CollapsingHeader::new(label_for(key, schema))
            .id_salt(id)
            .default_open(false)
            .show(ui, |ui| {
                changed = if matches!(classify(schema), Kind::Object) {
                    self.render_object(ui, schema, value, id)
                } else {
                    grid(ui, id, |ui| self.render_field(ui, schema, value, key, id))
                };
            });
        if let Some(d) = description(schema) {
            response.header_response.on_hover_text(d);
        }
        changed
    }

    /// An object's fields, scalars in a grid and nested objects as
    /// collapsible sections below it.
    fn render_object(
        &self,
        ui: &mut egui::Ui,
        schema: &Value,
        value: &mut Value,
        id: &str,
    ) -> bool {
        let Some(props) = schema.get("properties").and_then(Value::as_object) else {
            return render_raw(ui, value, id);
        };
        let object = ensure_object(value);
        let mut changed = grid(ui, id, |ui| {
            let mut changed = false;
            for (key, prop) in props {
                if matches!(classify(prop), Kind::Object) {
                    continue;
                }
                let field = object
                    .entry(key.clone())
                    .or_insert_with(|| default_for(prop));
                changed |= self.render_field(ui, prop, field, key, &format!("{id}.{key}"));
            }
            changed
        });
        for (key, prop) in props {
            if !matches!(classify(prop), Kind::Object) {
                continue;
            }
            let field = object
                .entry(key.clone())
                .or_insert_with(|| default_for(prop));
            changed |= self.render_section(ui, prop, field, key, &format!("{id}.{key}"));
        }
        changed
    }

    /// One labelled field as a row of the enclosing two-column grid: the
    /// label, the editor, and the row's end. A tagged enum takes more rows,
    /// one per field of the chosen variant, so they line up with everything
    /// else instead of nesting a grid in a cell.
    fn render_field(
        &self,
        ui: &mut egui::Ui,
        schema: &Value,
        value: &mut Value,
        key: &str,
        id: &str,
    ) -> bool {
        let label = ui.label(label_for(key, schema));
        if let Some(d) = description(schema) {
            label.on_hover_text(d);
        }
        let Kind::Tagged { tag, variants } = classify(schema) else {
            let changed = render_editor(ui, schema, value, id);
            ui.end_row();
            return changed;
        };

        let mut changed = false;
        let names: Vec<&str> = variants
            .iter()
            .filter_map(|v| v.get("properties")?.get(tag)?.get("const")?.as_str())
            .collect();
        let current = value
            .get(tag)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        egui::ComboBox::from_id_salt(format!("{id}.{tag}"))
            .selected_text(&current)
            .show_ui(ui, |ui| {
                for (name, variant) in names.iter().zip(variants) {
                    let response = ui.selectable_label(*name == current, *name);
                    if let Some(d) = description(variant) {
                        response.clone().on_hover_text(d);
                    }
                    if response.clicked() && *name != current {
                        *value = default_variant(tag, variant);
                        changed = true;
                    }
                }
            });
        ui.end_row();

        let current = value
            .get(tag)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let Some(variant) = names
            .iter()
            .position(|n| *n == current)
            .and_then(|i| variants.get(i))
        else {
            return changed;
        };
        let Some(props) = variant.get("properties").and_then(Value::as_object) else {
            return changed;
        };
        let object = ensure_object(value);
        for (key, prop) in props {
            if key == tag {
                continue;
            }
            let label = ui.label(format!("    {}", label_for(key, prop)));
            if let Some(d) = description(prop) {
                label.on_hover_text(d);
            }
            let field = object
                .entry(key.clone())
                .or_insert_with(|| default_for(prop));
            changed |= render_editor(ui, prop, field, &format!("{id}.{current}.{key}"));
            ui.end_row();
        }
        changed
    }
}

/// A two-column grid for field rows.
fn grid(ui: &mut egui::Ui, id: &str, body: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    egui::Grid::new(format!("{id}.grid"))
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, body)
        .inner
}

/// The editor for one value, in one grid cell.
fn render_editor(ui: &mut egui::Ui, schema: &Value, value: &mut Value, id: &str) -> bool {
    match classify(schema) {
        Kind::Optional(inner) => {
            let mut set = !value.is_null();
            let mut changed = false;
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut set, "")
                    .on_hover_text("Unset leaves it at the default, which for a limit means none")
                    .changed()
                {
                    changed = true;
                    *value = if set {
                        match schema.get("default") {
                            Some(d) if !d.is_null() => d.clone(),
                            _ => default_for(&inner),
                        }
                    } else {
                        Value::Null
                    };
                }
                if set {
                    changed |= render_editor(ui, &inner, value, id);
                } else {
                    ui.label(egui::RichText::new("unset").weak());
                }
            });
            changed
        }
        Kind::StringEnum(options) => {
            let current = value.as_str().unwrap_or("").to_string();
            let mut changed = false;
            egui::ComboBox::from_id_salt(id)
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for option in options {
                        if ui.selectable_label(option == current, option).clicked()
                            && option != current
                        {
                            *value = json!(option);
                            changed = true;
                        }
                    }
                });
            changed
        }
        // A tagged enum needs rows of its own; one standing in a cell is
        // edited as text.
        Kind::Tagged { .. } => render_raw(ui, value, id),
        Kind::Object => {
            // An object standing in a cell (an optional struct, a list
            // item): its fields on one line.
            let Some(props) = schema.get("properties").and_then(Value::as_object) else {
                return render_raw(ui, value, id);
            };
            let object = ensure_object(value);
            let mut changed = false;
            ui.horizontal(|ui| {
                for (key, prop) in props {
                    let field = object
                        .entry(key.clone())
                        .or_insert_with(|| default_for(prop));
                    ui.label(label_for(key, prop));
                    changed |= render_editor(ui, prop, field, &format!("{id}.{key}"));
                }
            });
            changed
        }
        Kind::Bool => {
            let mut b = value.as_bool().unwrap_or(false);
            if ui.checkbox(&mut b, "").changed() {
                *value = json!(b);
                true
            } else {
                false
            }
        }
        Kind::Integer => {
            let mut i = value.as_i64().unwrap_or(0);
            let min = schema
                .get("minimum")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MIN);
            let max = schema
                .get("maximum")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX);
            let mut drag = egui::DragValue::new(&mut i).range(min..=max);
            if let Some(unit) = unit_suffix(schema) {
                drag = drag.suffix(unit);
            }
            if ui.add(drag).changed() {
                *value = json!(i);
                true
            } else {
                false
            }
        }
        Kind::Number => {
            let scale = display_scale(schema);
            let mut shown = value.as_f64().unwrap_or(0.0) * scale;
            let speed = (shown.abs() * 0.01).max(0.001);
            let mut drag = egui::DragValue::new(&mut shown)
                .speed(speed)
                .max_decimals(6);
            if let Some(unit) = unit_suffix(schema) {
                drag = drag.suffix(unit);
            }
            if ui.add(drag).changed() {
                *value = json!(shown / scale);
                true
            } else {
                false
            }
        }
        Kind::Str => {
            let mut s = value.as_str().unwrap_or("").to_string();
            if ui
                .add(egui::TextEdit::singleline(&mut s).desired_width(240.0))
                .changed()
            {
                *value = json!(s);
                true
            } else {
                false
            }
        }
        Kind::FixedArray(n) => {
            let items = item_schemas(schema, n);
            let array = ensure_array(value, n, |i| default_for(&items[i]));
            let mut changed = false;
            ui.horizontal(|ui| {
                for (i, item) in array.iter_mut().enumerate() {
                    // The row's unit lives on the array, not the items.
                    let mut item_schema = items[i].clone();
                    for key in ["x-unit", "x-display-unit"] {
                        if let Some(u) = schema.get(key) {
                            item_schema[key] = u.clone();
                        }
                    }
                    changed |= render_editor(ui, &item_schema, item, &format!("{id}[{i}]"));
                    if i + 1 < n {
                        ui.label("to");
                    }
                }
            });
            changed
        }
        Kind::List => {
            let item_schema = schema.get("items").cloned().unwrap_or(json!({}));
            let mut changed = false;
            let list = ensure_array(value, 0, |_| Value::Null);
            ui.vertical(|ui| {
                let mut remove = None;
                for (i, item) in list.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        changed |= render_editor(ui, &item_schema, item, &format!("{id}[{i}]"));
                        if ui.small_button("remove").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    list.remove(i);
                    changed = true;
                }
                if ui.small_button("add").clicked() {
                    list.push(default_for(&item_schema));
                    changed = true;
                }
            });
            changed
        }
        Kind::Unknown => render_raw(ui, value, id),
    }
}

/// The fallback: the value as JSON text, parsed back when it parses.
fn render_raw(ui: &mut egui::Ui, value: &mut Value, id: &str) -> bool {
    let mut text = value.to_string();
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id_salt(id)
            .desired_width(320.0),
    );
    if response.changed()
        && let Ok(parsed) = serde_json::from_str::<Value>(&text)
    {
        *value = parsed;
        return true;
    }
    false
}

// -- Schema walking --

/// Replace every `$ref` under `node` with a copy of its target from `defs`,
/// the referring node's own keys (`description`, `default`, `x-unit`) kept
/// on top. The schemas here are not recursive; the depth bound is only
/// insurance.
fn inline_refs(node: &mut Value, defs: &Value, depth: usize) {
    if depth > 32 {
        return;
    }
    match node {
        Value::Object(obj) => {
            if let Some(Value::String(reference)) = obj.remove("$ref") {
                let name = reference
                    .strip_prefix("#/$defs/")
                    .unwrap_or(&reference)
                    .to_string();
                if let Some(target) = defs.get(&name) {
                    let mut target = target.clone();
                    inline_refs(&mut target, defs, depth + 1);
                    if let Some(t) = target.as_object_mut() {
                        for (k, v) in obj.iter() {
                            t.insert(k.clone(), v.clone());
                        }
                    }
                    *node = target;
                    return;
                }
            }
            for v in obj.values_mut() {
                inline_refs(v, defs, depth + 1);
            }
        }
        Value::Array(items) => {
            for v in items {
                inline_refs(v, defs, depth + 1);
            }
        }
        _ => {}
    }
}

fn default_for(schema: &Value) -> Value {
    if let Some(d) = schema.get("default") {
        return d.clone();
    }
    match classify(schema) {
        Kind::Optional(_) => Value::Null,
        Kind::StringEnum(options) => json!(options.first().copied().unwrap_or_default()),
        Kind::Tagged { tag, variants } => variants
            .first()
            .map(|v| default_variant(tag, v))
            .unwrap_or(Value::Null),
        Kind::Object => {
            let mut object = Map::new();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (k, p) in props {
                    object.insert(k.clone(), default_for(p));
                }
            }
            Value::Object(object)
        }
        Kind::Bool => json!(false),
        Kind::Integer => json!(
            schema
                .get("minimum")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(0)
        ),
        Kind::Number => json!(0.0),
        Kind::Str => json!(""),
        Kind::FixedArray(n) => {
            Value::Array(item_schemas(schema, n).iter().map(default_for).collect())
        }
        Kind::List => Value::Array(Vec::new()),
        Kind::Unknown => Value::Null,
    }
}

fn default_variant(tag: &str, variant: &Value) -> Value {
    let mut object = Map::new();
    if let Some(props) = variant.get("properties").and_then(Value::as_object) {
        for (k, p) in props {
            if k == tag {
                object.insert(k.clone(), p.get("const").cloned().unwrap_or(Value::Null));
            } else {
                object.insert(k.clone(), default_for(p));
            }
        }
    }
    Value::Object(object)
}

/// The schema of each slot of a fixed array or tuple.
fn item_schemas(schema: &Value, n: usize) -> Vec<Value> {
    if let Some(prefix) = schema.get("prefixItems").and_then(Value::as_array) {
        return prefix.clone();
    }
    vec![schema.get("items").cloned().unwrap_or(json!({})); n]
}

/// What a schema node asks the form to draw.
#[derive(Debug, Clone, PartialEq)]
enum Kind<'a> {
    Optional(Cow<'a, Value>),
    StringEnum(Vec<&'a str>),
    Tagged { tag: &'a str, variants: &'a [Value] },
    Object,
    Bool,
    Integer,
    Number,
    Str,
    FixedArray(usize),
    List,
    Unknown,
}

fn classify(schema: &Value) -> Kind<'_> {
    // `Option<Struct>`: anyOf [X, null].
    if let Some(alts) = schema.get("anyOf").and_then(Value::as_array)
        && alts.len() == 2
        && let Some(inner) = alts.iter().find(|a| a.get("type") != Some(&json!("null")))
        && alts.iter().any(|a| a.get("type") == Some(&json!("null")))
    {
        return Kind::Optional(Cow::Borrowed(inner));
    }
    // `Option<scalar>`: type [X, "null"].
    if let Some(types) = schema.get("type").and_then(Value::as_array)
        && types.iter().any(|t| t == "null")
        && let Some(other) = types.iter().find(|t| *t != "null")
    {
        let mut inner = schema.clone();
        inner["type"] = other.clone();
        if let Some(o) = inner.as_object_mut() {
            o.remove("default");
        }
        return Kind::Optional(Cow::Owned(inner));
    }
    if let Some(alts) = schema.get("oneOf").and_then(Value::as_array) {
        let consts: Vec<&str> = alts
            .iter()
            .filter_map(|a| a.get("const").and_then(Value::as_str))
            .collect();
        if consts.len() == alts.len() {
            return Kind::StringEnum(consts);
        }
        // Internally tagged: every variant is an object with one property
        // that has a `const`.
        let tag = alts.first().and_then(|a| {
            a.get("properties")?
                .as_object()?
                .iter()
                .find(|(_, p)| p.get("const").is_some())
                .map(|(k, _)| k.as_str())
        });
        if let Some(tag) = tag {
            return Kind::Tagged {
                tag,
                variants: alts,
            };
        }
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("object") => Kind::Object,
        Some("boolean") => Kind::Bool,
        Some("integer") => Kind::Integer,
        Some("number") => Kind::Number,
        Some("string") => Kind::Str,
        Some("array") => {
            let min = schema.get("minItems").and_then(Value::as_u64);
            let max = schema.get("maxItems").and_then(Value::as_u64);
            match (min, max) {
                (Some(a), Some(b)) if a == b && a > 0 => Kind::FixedArray(a as usize),
                _ => Kind::List,
            }
        }
        _ if schema.get("properties").is_some() => Kind::Object,
        _ => Kind::Unknown,
    }
}

fn ensure_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("just made it an object")
}

fn ensure_array(value: &mut Value, n: usize, default: impl Fn(usize) -> Value) -> &mut Vec<Value> {
    if !value.is_array() {
        *value = Value::Array(Vec::new());
    }
    let array = value.as_array_mut().expect("just made it an array");
    while array.len() < n {
        let i = array.len();
        array.push(default(i));
    }
    array
}

fn description(schema: &Value) -> Option<&str> {
    schema.get("description").and_then(Value::as_str)
}

/// `sharp_tip_bounds` becomes `Sharp tip bounds`, and a trailing unit
/// suffix (`_v`, `_ms`, `_secs`) goes when the schema carries the unit.
fn label_for(key: &str, schema: &Value) -> String {
    let mut words: Vec<&str> = key.split('_').collect();
    if schema.get("x-unit").is_some() {
        while let Some(last) = words.last()
            && matches!(*last, "v" | "a" | "m" | "s" | "ms" | "hz" | "secs" | "sec")
            && words.len() > 1
        {
            words.pop();
        }
    }
    let mut label = words.join(" ");
    if let Some(first) = label.get(..1) {
        label = first.to_uppercase() + &label[1..];
    }
    label
}

/// Multiply an SI value by this to show it in the display unit.
fn display_scale(schema: &Value) -> f64 {
    let unit = schema.get("x-unit").and_then(Value::as_str);
    let display = schema.get("x-display-unit").and_then(Value::as_str);
    match (unit, display) {
        (Some(u), Some(d)) if u == d => 1.0,
        (Some(_), Some(d)) => prefix_scale(d).unwrap_or(1.0),
        _ => 1.0,
    }
}

fn unit_suffix(schema: &Value) -> Option<String> {
    let unit = schema
        .get("x-display-unit")
        .or_else(|| schema.get("x-unit"))?
        .as_str()?;
    Some(format!(" {unit}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tip::config::AppConfig;

    fn form() -> SchemaForm {
        SchemaForm::new(serde_json::to_value(schemars::schema_for!(AppConfig)).unwrap())
    }

    /// Lay the whole form out headlessly and return whether it edited.
    fn draw(form: &SchemaForm, value: &mut Value, featured: &[&str]) -> bool {
        let ctx = egui::Context::default();
        let mut changed = false;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                changed |= grid(ui, "featured", |ui| {
                    let mut changed = false;
                    for path in featured {
                        changed |= form.render_path(ui, value, path);
                    }
                    changed
                });
                changed |= form.render_except(ui, value, &[]);
            });
        });
        changed
    }

    #[test]
    fn references_are_inlined_once() {
        let form = form();
        assert!(form.root.get("$defs").is_none());
        assert!(
            !form.root.to_string().contains("$ref"),
            "every reference was inlined"
        );
        let stability = &form.root["properties"]["tip_prep"]["properties"]["stability"];
        assert_eq!(stability["type"], "object");
        assert!(
            stability.get("default").is_some(),
            "the referring node's own keys survive"
        );
    }

    #[test]
    fn drawing_the_default_config_is_a_no_op_and_round_trips() {
        let form = form();
        let config = AppConfig::default();
        let before = serde_json::to_value(&config).unwrap();
        let mut value = before.clone();
        let changed = draw(
            &form,
            &mut value,
            &[
                "tip_prep.sharp_tip_bounds",
                "pulse_method",
                "tip_prep.max_cycles",
            ],
        );
        assert!(!changed, "an untouched form edits nothing");
        assert_eq!(value, before, "drawing must not rewrite the value");
        let back: AppConfig = serde_json::from_value(value).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), before);
    }

    #[test]
    fn every_shipped_config_survives_the_form() {
        let form = form();
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/configs")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let config: AppConfig = toml::from_str(&text).unwrap();
            let before = serde_json::to_value(&config).unwrap();
            let mut value = before.clone();
            draw(
                &form,
                &mut value,
                &["pulse_method", "tip_prep.stability.check_stability"],
            );
            assert_eq!(value, before, "{}", path.display());
            let back: AppConfig = serde_json::from_value(value).unwrap();
            assert_eq!(
                serde_json::to_value(&back).unwrap(),
                before,
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn the_schema_default_deserializes_into_the_config() {
        let form = form();
        let value = form.default_value();
        let config: AppConfig =
            serde_json::from_value(value).expect("schema defaults are a valid config");
        assert_eq!(config.tip_prep.sharp_tip_bounds, [0.0, 0.0]);
    }

    #[test]
    fn switching_the_pulse_method_builds_the_variant_with_its_defaults() {
        let form = form();
        let pm = &form.root["properties"]["pulse_method"];
        let Kind::Tagged { tag, variants } = classify(pm) else {
            panic!("pulse_method is an internally tagged enum");
        };
        assert_eq!(tag, "type");
        let linear = variants
            .iter()
            .find(|v| v["properties"]["type"]["const"] == "linear")
            .unwrap();
        let value = default_variant(tag, linear);
        assert_eq!(value["type"], "linear");
        assert_eq!(value["polarity"], "positive");
        assert!(value["random_polarity_switch"].is_null());
        assert_eq!(value["voltage_bounds"], json!([0.0, 0.0]));
        let method: rusty_tip::PulseMethod = serde_json::from_value(value).unwrap();
        assert_eq!(method.method_name(), "Linear");
    }

    #[test]
    fn units_scale_for_display_and_labels_drop_unit_suffixes() {
        let amps = json!({"x-unit": "A", "x-display-unit": "pA"});
        assert_eq!(display_scale(&amps), 1e12);
        let volts = json!({"x-unit": "V", "x-display-unit": "mV"});
        assert_eq!(display_scale(&volts), 1e3);
        let speed = json!({"x-unit": "m/s", "x-display-unit": "nm/s"});
        assert_eq!(display_scale(&speed), 1e9);
        let plain = json!({"x-unit": "Hz"});
        assert_eq!(display_scale(&plain), 1.0);

        assert_eq!(label_for("initial_bias_v", &volts), "Initial bias");
        assert_eq!(label_for("scan_speed_m_s", &speed), "Scan speed");
        assert_eq!(label_for("max_cycles", &json!({})), "Max cycles");
        assert_eq!(
            label_for("step_period_ms", &json!({"x-unit": "ms"})),
            "Step period"
        );
    }

    #[test]
    fn options_and_enums_are_recognised() {
        let form = form();
        let tip = &form.root["properties"]["tip_prep"];
        assert!(matches!(
            classify(&tip["properties"]["max_cycles"]),
            Kind::Optional(_)
        ));
        assert!(matches!(
            classify(&tip["properties"]["sharp_tip_bounds"]),
            Kind::FixedArray(2)
        ));
        let stab = &tip["properties"]["stability"];
        assert_eq!(
            classify(&stab["properties"]["polarity_mode"]),
            Kind::StringEnum(vec!["positive", "negative", "both"])
        );
        assert!(matches!(
            classify(&stab["properties"]["bias_range"]),
            Kind::FixedArray(2)
        ));
        assert!(matches!(
            classify(&form.root["properties"]["tcp_channel_mapping"]),
            Kind::Optional(_)
        ));
    }
}
