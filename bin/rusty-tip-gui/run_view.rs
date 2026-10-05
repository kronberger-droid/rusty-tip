//! A run as a fold over its log records.
//!
//! No egui in here. A live run feeds [`RunView::apply_event`] from the
//! session's event channel, a replayed log feeds [`RunView::apply`] from
//! `experiment_log::reader`; both go through the same [`Record`] fold, so the
//! view of a run cannot differ between watching it and reading it back.
//!
//! What is kept is schema-free: every numeric field of every `DataCollected`
//! value and every custom event becomes a series keyed by `label.field` or
//! `kind.field`, a `phase` string becomes a phase transition, and the last
//! records are kept as one-line summaries for a log tail. A tool's panel
//! reads the series it knows by name.

use std::collections::{BTreeMap, VecDeque};

use rusty_tip::event::Event;
use rusty_tip::experiment_log::reader::{Body, Header, Record};

use crate::units::number;

/// Upper bound on points per series; the oldest go first.
const MAX_SERIES_POINTS: usize = 20_000;

/// Records kept for the log tail.
const TAIL_LINES: usize = 500;

/// Custom events kept per kind, as rows; the oldest go first.
const MAX_ROWS_PER_KIND: usize = 2_000;

/// One numeric series, as `[time_s, value]` pairs in arrival order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Series {
    pub points: Vec<[f64; 2]>,
}

impl Series {
    pub fn latest(&self) -> Option<f64> {
        self.points.last().map(|p| p[1])
    }

    fn push(&mut self, time_s: f64, value: f64) {
        self.points.push([time_s, value]);
        trim_front(&mut self.points, MAX_SERIES_POINTS);
    }
}

/// How the run ended, from its `run_finished` line.
#[derive(Debug, Clone, PartialEq)]
pub struct Finish {
    pub outcome: String,
    pub detail: Option<String>,
    pub duration_ms: f64,
}

#[derive(Debug, Clone, Default)]
pub struct RunView {
    pub header: Option<Header>,
    /// Timestamp of the first record, the origin of every `time_s`.
    started_at: Option<f64>,
    /// Timestamp of the last record.
    last_at: Option<f64>,
    /// The innermost action that has started and not finished, with its depth.
    pub current_action: Option<(String, usize)>,
    /// The last start of each action, by name, as `(time_s, params)`.
    pub last_started: BTreeMap<String, (f64, serde_json::Value)>,
    pub series: BTreeMap<String, Series>,
    /// Every custom event by kind, as `(time_s, data)` rows, for panels that
    /// want the record rather than a number out of it.
    pub custom: BTreeMap<String, Vec<(f64, serde_json::Value)>>,
    /// Every `phase` seen, with when.
    pub phases: Vec<(f64, String)>,
    pub finish: Option<Finish>,
    pub records: usize,
    pub tail: VecDeque<String>,
}

impl RunView {
    /// Fold one record in.
    pub fn apply(&mut self, record: Record) {
        self.records += 1;
        if record.timestamp > 0.0 {
            if self.started_at.is_none() {
                self.started_at = Some(record.timestamp);
            }
            self.last_at = Some(record.timestamp);
        }
        let t = self.time_of(record.timestamp);

        self.tail.push_back(summarize(t, &record.body));
        if self.tail.len() > TAIL_LINES {
            self.tail.pop_front();
        }

        match record.body {
            Body::RunStarted { header } => {
                self.header = Some(header);
            }
            Body::RunFinished {
                outcome,
                detail,
                duration,
            } => {
                self.finish = Some(Finish {
                    outcome,
                    detail,
                    duration_ms: duration,
                });
                self.current_action = None;
            }
            Body::ActionStarted {
                action,
                params,
                depth,
            } => {
                self.last_started.insert(action.clone(), (t, params));
                self.current_action = Some((action, depth));
            }
            Body::ActionCompleted { action, depth, .. }
            | Body::ActionFailed { action, depth, .. } => {
                if self
                    .current_action
                    .as_ref()
                    .is_some_and(|(a, d)| *a == action && *d == depth)
                {
                    self.current_action = None;
                }
            }
            Body::DataCollected { label, value } => {
                self.collect(&label, &value, t);
            }
            Body::Custom { kind, data } => {
                if let Some(phase) = data.get("phase").and_then(|p| p.as_str()) {
                    self.phases.push((t, phase.to_string()));
                }
                self.collect(&kind, &data, t);
                let rows = self.custom.entry(kind).or_default();
                rows.push((t, data));
                trim_front(rows, MAX_ROWS_PER_KIND);
            }
        }
    }

    /// Fold a live event in, through the same path a log line takes.
    ///
    /// `Event` serializes to exactly what `FileLogger` writes, so decoding
    /// it as a [`Record`] is the same conversion the reader does. A custom
    /// event is moved across rather than decoded, since a stream dump is
    /// megabytes. An event that does not decode is dropped and counted;
    /// the writer and reader are in this crate, so that is a bug, not a
    /// runtime condition.
    pub fn apply_event(&mut self, event: Event) {
        let record = match event {
            Event::Custom {
                kind,
                data,
                timestamp,
            } => Record {
                seq: 0,
                timestamp: timestamp
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0),
                body: Body::Custom { kind, data },
            },
            other => match serde_json::to_value(&other).and_then(serde_json::from_value) {
                Ok(record) => record,
                Err(e) => {
                    log::warn!("RunView: event does not decode as a record: {e}");
                    return;
                }
            },
        };
        self.apply(record);
    }

    /// Seconds from the first record to `timestamp`.
    fn time_of(&self, timestamp: f64) -> f64 {
        match self.started_at {
            Some(t0) if timestamp > 0.0 => timestamp - t0,
            _ => 0.0,
        }
    }

    /// Seconds from the first record to the last.
    pub fn elapsed_s(&self) -> Option<f64> {
        Some(self.last_at? - self.started_at?)
    }

    /// When the run's first record was, which tells one run from the next.
    pub fn started_at(&self) -> Option<f64> {
        self.started_at
    }

    pub fn latest(&self, key: &str) -> Option<f64> {
        self.series.get(key)?.latest()
    }

    pub fn points(&self, key: &str) -> &[[f64; 2]] {
        self.series
            .get(key)
            .map(|s| s.points.as_slice())
            .unwrap_or(&[])
    }

    /// The rows of one custom kind, oldest first.
    pub fn custom(&self, kind: &str) -> &[(f64, serde_json::Value)] {
        self.custom.get(kind).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The last phase seen.
    pub fn phase(&self) -> Option<&str> {
        self.phases.last().map(|(_, p)| p.as_str())
    }

    /// A plain number goes under `key`; an object's numeric and boolean
    /// fields go under `key.field`. Booleans plot as 0 and 1.
    fn collect(&mut self, key: &str, value: &serde_json::Value, t: f64) {
        match value {
            serde_json::Value::Number(n) => {
                if let Some(v) = n.as_f64() {
                    self.series.entry(key.to_string()).or_default().push(t, v);
                }
            }
            serde_json::Value::Object(fields) => {
                for (name, field) in fields {
                    let v = match field {
                        serde_json::Value::Number(n) => n.as_f64(),
                        serde_json::Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                        _ => None,
                    };
                    if let Some(v) = v {
                        self.series
                            .entry(format!("{key}.{name}"))
                            .or_default()
                            .push(t, v);
                    }
                }
            }
            _ => {}
        }
    }
}

/// One line per record for the log tail.
fn summarize(t: f64, body: &Body) -> String {
    let what = match body {
        Body::RunStarted { header } => format!("run started: {} {}", header.tool, header.version),
        Body::RunFinished {
            outcome, detail, ..
        } => match detail {
            Some(d) => format!("run finished: {outcome} ({d})"),
            None => format!("run finished: {outcome}"),
        },
        Body::ActionStarted {
            action,
            params,
            depth,
        } => {
            let params = compact(params);
            format!("{}{action} {params}", "  ".repeat(*depth))
        }
        Body::ActionCompleted {
            action,
            depth,
            duration,
            ..
        } => format!("{}{action} done ({duration:.0} ms)", "  ".repeat(*depth)),
        Body::ActionFailed {
            action,
            error,
            depth,
            ..
        } => format!("{}{action} FAILED: {error}", "  ".repeat(*depth)),
        Body::DataCollected { label, value } => format!("{label} = {}", compact(value)),
        Body::Custom { kind, data } => format!("{kind} {}", compact(data)),
    };
    format!("{t:8.1}s  {what}")
}

/// Drop the oldest tenth once `v` grows past `cap`, so a full buffer does
/// not shift on every push.
fn trim_front<T>(v: &mut Vec<T>, cap: usize) {
    if v.len() > cap {
        let excess = v.len() - cap + cap / 10;
        v.drain(0..excess);
    }
}

/// A short rendering of a value: objects without braces, arrays by their
/// length, long ones cut.
fn compact(value: &serde_json::Value) -> String {
    let s = match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Array(items) => format!("[{} items]", items.len()),
        serde_json::Value::Object(fields) => fields
            .iter()
            .map(|(k, v)| format!("{k}={}", compact(v)))
            .collect::<Vec<_>>()
            .join(" "),
        serde_json::Value::Number(n) => n.as_f64().map(number).unwrap_or_else(|| n.to_string()),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.len() > 120 {
        format!(
            "{}…",
            &s[..s.char_indices().nth(117).map(|(i, _)| i).unwrap_or(117)]
        )
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tip::experiment_log::reader::Log;

    const SAMPLE: &str = r#"{"seq":0,"type":"run_started","header":{"tool":"tip_prep","version":"1","schema":{"tool":"tip_prep","kinds":[]}},"timestamp":100.0}
{"seq":1,"type":"action_started","action":"reposition","params":{},"depth":0,"timestamp":101.0}
{"seq":2,"type":"action_started","action":"withdraw","params":{},"depth":1,"timestamp":101.2}
{"seq":3,"type":"action_completed","action":"withdraw","output":null,"depth":1,"duration":50.0,"timestamp":101.3}
{"seq":4,"type":"data_collected","label":"stable_read","value":{"value":-1.5,"std_dev":0.1,"n":16},"timestamp":102.0}
{"seq":5,"type":"custom","kind":"tip_prep/phase","data":{"phase":"confirming"},"timestamp":103.0}
{"seq":6,"type":"custom","kind":"tip_prep/cycle","data":{"cycle":1,"freq_shift":-1.5,"pulse_voltage":3.0,"is_sharp":true},"timestamp":104.0}
{"seq":7,"type":"run_finished","outcome":"completed","duration":4000.0,"timestamp":104.0}
"#;

    #[test]
    fn folds_series_phases_actions_and_the_outcome() {
        let log = Log::parse(SAMPLE);
        let mut view = RunView::default();
        for r in log.records {
            view.apply(r);
        }
        assert_eq!(view.header.as_ref().unwrap().tool, "tip_prep");
        assert_eq!(view.points("stable_read.value"), &[[2.0, -1.5]]);
        assert_eq!(view.points("stable_read.n"), &[[2.0, 16.0]]);
        assert_eq!(view.latest("tip_prep/cycle.pulse_voltage"), Some(3.0));
        assert_eq!(view.latest("tip_prep/cycle.is_sharp"), Some(1.0));
        assert_eq!(view.phase(), Some("confirming"));
        assert_eq!(view.custom("tip_prep/cycle").len(), 1);
        assert_eq!(view.custom("tip_prep/cycle")[0].1["cycle"], 1);
        assert_eq!(view.phases, vec![(3.0, "confirming".to_string())]);
        assert_eq!(view.finish.as_ref().unwrap().outcome, "completed");
        assert_eq!(view.elapsed_s(), Some(4.0));
        assert_eq!(view.records, 8);
        assert!(
            view.current_action.is_none(),
            "finished runs have no open action"
        );
    }

    #[test]
    fn the_current_action_is_the_innermost_open_one() {
        let log = Log::parse(SAMPLE);
        let mut view = RunView::default();
        for r in &log.records[..3] {
            view.apply(r.clone());
        }
        assert_eq!(
            view.current_action,
            Some(("withdraw".to_string(), 1)),
            "the child is open"
        );
        view.apply(log.records[3].clone());
        assert!(
            view.current_action.is_none(),
            "closing the child leaves nothing; the parent's start is not replayed"
        );
    }

    #[test]
    fn a_live_event_folds_the_same_as_its_log_line() {
        let event = Event::data_collected("stable_read", serde_json::json!({"value": -2.0}));
        let line = serde_json::to_string(&event).unwrap();
        let mut live = RunView::default();
        live.apply_event(event);

        let log = Log::parse(&line);
        let mut replayed = RunView::default();
        replayed.apply(log.records[0].clone());

        assert_eq!(live.series, replayed.series);
        assert_eq!(live.tail, replayed.tail);
    }

    #[test]
    fn a_run_started_event_carries_its_header_through() {
        use rusty_tip::experiment_log::RunHeader;
        let event = Event::run_started(RunHeader::new(
            rusty_tip::tip_prep::log_schema(),
            serde_json::Value::Null,
            serde_json::Value::Null,
        ));
        let mut view = RunView::default();
        view.apply_event(event);
        assert_eq!(view.header.unwrap().tool, "tip_prep");
    }

    #[test]
    fn series_are_capped() {
        let mut s = Series::default();
        for i in 0..(MAX_SERIES_POINTS + 1) {
            s.push(i as f64, 0.0);
        }
        let kept = MAX_SERIES_POINTS - MAX_SERIES_POINTS / 10;
        assert_eq!(s.points.len(), kept, "a tenth goes at once");
        assert_eq!(
            s.points[0][0],
            (MAX_SERIES_POINTS + 1 - kept) as f64,
            "the oldest went first"
        );
        assert_eq!(s.latest(), Some(0.0));
        for i in 0..(MAX_SERIES_POINTS / 10) {
            s.push(i as f64, 1.0);
        }
        assert_eq!(s.points.len(), MAX_SERIES_POINTS, "full again, untrimmed");
    }
}
