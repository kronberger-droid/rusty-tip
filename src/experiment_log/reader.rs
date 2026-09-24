//! Reading an experiment log back: the lines as records, the header as
//! data, and the action tree the depth field encodes.
//!
//! The reader is deliberately looser than the writer. Lines that predate the
//! envelope (no `seq`, no header, no `depth`) still parse with defaults, and
//! a line that does not parse at all is counted and skipped rather than
//! failing the whole file, since a log cut short by a crash is exactly the
//! one worth reading.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::experiment_log::{LogEvent, SignalFact};
use crate::routine::StreamDumpEvent;
use crate::spm_controller::StreamSnapshot;

/// One line of a log.
#[derive(Deserialize, Debug, Clone)]
pub struct Record {
    #[serde(default)]
    pub seq: u64,
    /// Seconds since the Unix epoch. Zero on lines from before it was
    /// written for every event.
    #[serde(default)]
    pub timestamp: f64,
    #[serde(flatten)]
    pub body: Body,
}

/// What a line says, by its `type`.
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Body {
    RunStarted {
        header: Header,
    },
    RunFinished {
        outcome: String,
        #[serde(default)]
        detail: Option<String>,
        /// Milliseconds.
        duration: f64,
    },
    ActionStarted {
        action: String,
        #[serde(default)]
        params: Value,
        #[serde(default)]
        depth: usize,
    },
    ActionCompleted {
        action: String,
        #[serde(default)]
        output: Value,
        #[serde(default)]
        depth: usize,
        /// Milliseconds.
        duration: f64,
    },
    ActionFailed {
        action: String,
        error: String,
        #[serde(default)]
        depth: usize,
        /// Milliseconds.
        duration: f64,
    },
    DataCollected {
        label: String,
        value: Value,
    },
    Custom {
        kind: String,
        data: Value,
    },
}

/// The `run_started` header, as data.
#[derive(Deserialize, Debug, Clone)]
pub struct Header {
    pub tool: String,
    pub version: String,
    #[serde(default)]
    pub git_commit: Option<String>,
    #[serde(default)]
    pub envelope_version: u32,
    #[serde(default)]
    pub config: Value,
    #[serde(default)]
    pub controller: Value,
    pub schema: SchemaDecl,
}

impl Header {
    /// The signals the run resolved, from the controller facts. Empty for a
    /// log from before they were recorded.
    pub fn signals(&self) -> Vec<SignalFact> {
        self.controller
            .get("signals")
            .and_then(|v| Vec::<SignalFact>::deserialize(v).ok())
            .unwrap_or_default()
    }
}

/// The tool's declared custom event kinds.
#[derive(Deserialize, Debug, Clone)]
pub struct SchemaDecl {
    pub tool: String,
    pub kinds: Vec<KindDecl>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct KindDecl {
    pub kind: String,
    pub schema: Value,
}

impl KindDecl {
    /// The declared scalar fields of this kind's `data`, in declaration
    /// order, with their JSON Schema type. Nested objects are skipped; a
    /// field that can be several types (an internally tagged enum's
    /// variant fields) is included when every variant that has it agrees.
    pub fn scalar_fields(&self) -> Vec<(String, String)> {
        let mut out: BTreeMap<String, String> = BTreeMap::new();
        collect_scalar_fields(&self.schema, &mut out);
        out.into_iter().collect()
    }
}

fn collect_scalar_fields(schema: &Value, out: &mut BTreeMap<String, String>) {
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (name, prop) in props {
            if let Some(ty) = scalar_type(prop) {
                out.entry(name.clone()).or_insert(ty);
            }
        }
    }
    // Enum variants and `anyOf` alternatives each carry their own properties.
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(alts) = schema.get(key).and_then(Value::as_array) {
            for alt in alts {
                collect_scalar_fields(alt, out);
            }
        }
    }
}

/// The scalar type name of a property schema, or `None` for objects and
/// arrays. A nullable scalar (`["number", "null"]`) counts as its scalar.
fn scalar_type(prop: &Value) -> Option<String> {
    let ty = prop.get("type")?;
    let name = match ty {
        Value::String(s) => s.clone(),
        Value::Array(alts) => alts
            .iter()
            .filter_map(Value::as_str)
            .find(|s| *s != "null")?
            .to_string(),
        _ => return None,
    };
    match name.as_str() {
        "string" | "number" | "integer" | "boolean" => Some(name),
        _ => None,
    }
}

/// A parsed log.
#[derive(Debug, Clone, Default)]
pub struct Log {
    pub records: Vec<Record>,
    /// Lines that did not parse, with their 1-based line numbers.
    pub skipped: Vec<(usize, String)>,
}

impl Log {
    pub fn read(path: impl AsRef<Path>) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        Ok(Self::parse(&text))
    }

    pub fn parse(text: &str) -> Self {
        let mut log = Log::default();
        for (i, line) in text.lines().enumerate() {
            if !line.trim().is_empty() {
                log.push_line(i + 1, line);
            }
        }
        log
    }

    /// Only the first and last lines: enough for [`header`](Self::header),
    /// [`started_at`](Self::started_at) and [`finished`](Self::finished),
    /// without parsing everything in between (a stream dump alone can be
    /// megabytes). Line numbers in `skipped` are not meaningful here.
    pub fn read_bookends(path: impl AsRef<Path>) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let mut log = Log::default();
        if let Some(first) = lines.next() {
            log.push_line(1, first);
        }
        if let Some(last) = lines.next_back() {
            log.push_line(0, last);
        }
        Ok(log)
    }

    fn push_line(&mut self, line_no: usize, line: &str) {
        match serde_json::from_str::<Record>(line) {
            Ok(record) => self.records.push(record),
            Err(e) => self.skipped.push((line_no, e.to_string())),
        }
    }

    /// Every stream dump in the log with the record that carried it.
    pub fn stream_dumps(&self) -> impl Iterator<Item = (&Record, StreamSnapshot)> {
        self.records.iter().filter_map(|r| match &r.body {
            Body::Custom { kind, data } if kind == StreamDumpEvent::KIND => {
                StreamDumpEvent::deserialize(data)
                    .ok()
                    .map(|e| (r, e.stream))
            }
            _ => None,
        })
    }

    pub fn header(&self) -> Option<&Header> {
        self.records.iter().find_map(|r| match &r.body {
            Body::RunStarted { header } => Some(header),
            _ => None,
        })
    }

    /// The `run_finished` line, if the run got to write one.
    pub fn finished(&self) -> Option<(&str, Option<&str>, f64)> {
        self.records.iter().rev().find_map(|r| match &r.body {
            Body::RunFinished {
                outcome,
                detail,
                duration,
            } => Some((outcome.as_str(), detail.as_deref(), *duration)),
            _ => None,
        })
    }

    /// Timestamp of the first line, the origin for `time_s`.
    pub fn started_at(&self) -> Option<f64> {
        self.records.iter().map(|r| r.timestamp).find(|t| *t > 0.0)
    }

    /// Seconds from the first line to `record`.
    pub fn time_s(&self, record: &Record) -> f64 {
        match self.started_at() {
            Some(t0) if record.timestamp > 0.0 => record.timestamp - t0,
            _ => 0.0,
        }
    }

    /// The actions as a tree, rebuilt from `depth`. A started action with
    /// no completion (the run was cut off) is kept with `duration_ms`
    /// unset.
    pub fn action_tree(&self) -> Vec<ActionNode> {
        let mut roots: Vec<ActionNode> = Vec::new();
        // Open actions, innermost last.
        let mut open: Vec<ActionNode> = Vec::new();

        fn close_into(node: ActionNode, open: &mut [ActionNode], roots: &mut Vec<ActionNode>) {
            match open.last_mut() {
                Some(parent) => parent.children.push(node),
                None => roots.push(node),
            }
        }

        for record in &self.records {
            match &record.body {
                Body::ActionStarted {
                    action,
                    params,
                    depth,
                } => {
                    // A start at a depth no deeper than an open action means
                    // that action never wrote its end: close it as cut off.
                    while open.last().is_some_and(|n| n.depth >= *depth) {
                        let node = open.pop().unwrap();
                        close_into(node, &mut open, &mut roots);
                    }
                    open.push(ActionNode {
                        seq: record.seq,
                        name: action.clone(),
                        params: params.clone(),
                        depth: *depth,
                        time_s: self.time_s(record),
                        duration_ms: None,
                        error: None,
                        children: Vec::new(),
                    });
                }
                Body::ActionCompleted {
                    action, duration, ..
                }
                | Body::ActionFailed {
                    action, duration, ..
                } => {
                    let error = match &record.body {
                        Body::ActionFailed { error, .. } => Some(error.clone()),
                        _ => None,
                    };
                    if let Some(pos) = open.iter().rposition(|n| n.name == *action) {
                        // Anything opened inside it and never closed is cut off.
                        while open.len() > pos + 1 {
                            let node = open.pop().unwrap();
                            close_into(node, &mut open, &mut roots);
                        }
                        let mut node = open.pop().unwrap();
                        node.duration_ms = Some(*duration);
                        node.error = error;
                        close_into(node, &mut open, &mut roots);
                    }
                }
                _ => {}
            }
        }
        while let Some(node) = open.pop() {
            close_into(node, &mut open, &mut roots);
        }
        roots
    }
}

/// One action in the tree.
#[derive(Debug, Clone)]
pub struct ActionNode {
    pub seq: u64,
    pub name: String,
    pub params: Value,
    pub depth: usize,
    /// Seconds from the start of the log.
    pub time_s: f64,
    /// `None` when the log ends before the action did.
    pub duration_ms: Option<f64>,
    pub error: Option<String>,
    pub children: Vec<ActionNode>,
}

impl ActionNode {
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }

    /// Depth-first walk over this node and everything under it.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a ActionNode)) {
        f(self);
        for child in &self.children {
            child.walk(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"seq":0,"type":"run_started","header":{"tool":"t","version":"1","schema":{"tool":"t","kinds":[{"kind":"t/cycle","schema":{"type":"object","properties":{"cycle":{"type":"integer"},"freq_shift":{"type":["number","null"]},"nested":{"type":"object"}}}}]}},"timestamp":100.0}
{"seq":1,"type":"action_started","action":"outer","params":{"a":1},"depth":0,"timestamp":100.5}
{"seq":2,"type":"action_started","action":"inner","params":{},"depth":1,"timestamp":100.6}
{"seq":3,"type":"action_completed","action":"inner","output":null,"depth":1,"duration":50.0,"timestamp":100.65}
{"seq":4,"type":"action_completed","action":"outer","output":null,"depth":0,"duration":200.0,"timestamp":100.7}
{"seq":5,"type":"custom","kind":"t/cycle","data":{"cycle":1,"freq_shift":-1.5},"timestamp":101.0}
not json at all
{"seq":6,"type":"action_started","action":"cut","params":{},"depth":0,"timestamp":102.0}
"#;

    #[test]
    fn parses_records_and_counts_bad_lines() {
        let log = Log::parse(SAMPLE);
        assert_eq!(log.records.len(), 7);
        assert_eq!(log.skipped.len(), 1);
        assert_eq!(log.skipped[0].0, 7, "1-based line number of the bad line");
        assert_eq!(log.header().unwrap().tool, "t");
        assert!(log.finished().is_none(), "the sample was cut off");
        assert!((log.time_s(&log.records[5]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn rebuilds_the_tree_from_depth_and_keeps_cut_off_actions() {
        let log = Log::parse(SAMPLE);
        let tree = log.action_tree();
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].name, "outer");
        assert_eq!(tree[0].duration_ms, Some(200.0));
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].name, "inner");
        assert_eq!(tree[1].name, "cut");
        assert_eq!(tree[1].duration_ms, None);
    }

    #[test]
    fn declared_scalar_fields_come_from_the_schema() {
        let log = Log::parse(SAMPLE);
        let fields = log.header().unwrap().schema.kinds[0].scalar_fields();
        assert_eq!(
            fields,
            vec![
                ("cycle".to_string(), "integer".to_string()),
                ("freq_shift".to_string(), "number".to_string()),
            ],
            "nested objects are not columns; nullable scalars are"
        );
    }

    #[test]
    fn a_legacy_line_without_envelope_fields_still_parses() {
        let log = Log::parse(
            r#"{"type":"action_started","action":"set_bias","params":{},"timestamp":1.0}
{"type":"custom","kind":"tip_prep_state","data":{"phase":"confirming"}}"#,
        );
        assert_eq!(log.records.len(), 2);
        assert!(log.skipped.is_empty());
        assert_eq!(log.records[0].seq, 0);
        assert_eq!(log.records[1].timestamp, 0.0);
    }
}
