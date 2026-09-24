//! `rt-log`: inspect experiment logs from the terminal.
//!
//! Works on any log with a `run_started` header, whichever tool wrote it:
//! the columns `export` writes and the series `plot` offers come from the
//! schema the tool declared in that header, not from anything here.
//!
//! ```text
//! rt-log ls experiments/                 # every run, newest first
//! rt-log summary <file>                  # outcome, timing, what ran, what was measured
//! rt-log timeline <file>                 # the action tree with durations
//! rt-log plot <file> tip_prep/cycle.freq_shift
//! rt-log export <file> --out <dir>       # flat CSV tables
//! ```

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use serde_json::Value;
use textplots::{Chart, Plot, Shape};

use rusty_tip::experiment_log::reader::{ActionNode, Body, Log, Record};

/// `println!` that treats a closed pipe as the reader being done, so
/// `rt-log timeline … | head` ends quietly instead of panicking.
macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let mut out = std::io::stdout().lock();
        if writeln!(out, $($arg)*).is_err() {
            std::process::exit(0);
        }
    }};
}

#[derive(Parser)]
#[command(name = "rt-log", about = "Inspect rusty-tip experiment logs")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the runs in a directory, newest first.
    Ls {
        /// Directory of `.jsonl` logs.
        #[arg(default_value = "./experiments")]
        dir: PathBuf,
    },
    /// What a run was, how it ended, what it did and what it measured.
    Summary(FileArg),
    /// The action tree with timings, one line per action.
    Timeline(TimelineArgs),
    /// Plot a series against time in the terminal.
    Plot(PlotArgs),
    /// Write flat CSV tables: actions, measurements, one per custom kind.
    Export(ExportArgs),
}

#[derive(Args)]
struct FileArg {
    file: PathBuf,
}

#[derive(Args)]
struct TimelineArgs {
    file: PathBuf,
    /// Only actions with this name, and everything under them.
    #[arg(long)]
    action: Option<String>,
    /// Deepest level to print; 0 shows only what the routine ran directly.
    #[arg(long)]
    max_depth: Option<usize>,
    /// Only failed or cut-off actions.
    #[arg(long)]
    failed: bool,
}

#[derive(Args)]
struct PlotArgs {
    file: PathBuf,
    /// `kind.field` for a custom event (`tip_prep/cycle.freq_shift`) or
    /// `label.field` for a measurement (`stable_read.value`). Omit to list
    /// what the log offers.
    series: Option<String>,
    #[arg(long, default_value_t = 120)]
    width: u32,
    #[arg(long, default_value_t = 40)]
    height: u32,
}

#[derive(Args)]
struct ExportArgs {
    file: PathBuf,
    /// Directory for the tables. Default: next to the log, named after it.
    #[arg(long)]
    out: Option<PathBuf>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Ls { dir } => ls(&dir),
        Command::Summary(a) => summary(&Log::read(&a.file)?, &a.file),
        Command::Timeline(a) => timeline(&Log::read(&a.file)?, &a),
        Command::Plot(a) => plot(&Log::read(&a.file)?, &a),
        Command::Export(a) => export(&Log::read(&a.file)?, &a),
    }
}

// ============================================================================
// ls
// ============================================================================

fn ls(dir: &Path) -> Result<(), Box<dyn Error>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    files.reverse();
    if files.is_empty() {
        outln!("no .jsonl logs in {}", dir.display());
        return Ok(());
    }
    outln!(
        "{:<44} {:<15} {:<20} {:>9}  {}",
        "file",
        "tool",
        "started (UTC)",
        "duration",
        "outcome"
    );
    for path in files {
        let log = Log::read(&path)?;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let tool = log
            .header()
            .map(|h| h.tool.as_str())
            .unwrap_or("(no header)");
        let started = log.started_at().map(fmt_utc).unwrap_or_default();
        let (duration, outcome) = match log.finished() {
            Some((outcome, detail, ms)) => (
                fmt_duration_ms(ms),
                match detail {
                    Some(d) => format!("{outcome} ({d})"),
                    None => outcome.to_string(),
                },
            ),
            None => (String::new(), "cut off".to_string()),
        };
        outln!("{name:<44} {tool:<15} {started:<20} {duration:>9}  {outcome}");
    }
    Ok(())
}

// ============================================================================
// summary
// ============================================================================

fn summary(log: &Log, path: &Path) -> Result<(), Box<dyn Error>> {
    match log.header() {
        Some(h) => {
            let commit = h.git_commit.as_deref().unwrap_or("no commit");
            outln!("{} {} ({commit})   {}", h.tool, h.version, path.display());
        }
        None => outln!("(no run header)   {}", path.display()),
    }
    let started = log.started_at().map(fmt_utc).unwrap_or_default();
    match log.finished() {
        Some((outcome, detail, ms)) => {
            let detail = detail.map(|d| format!(" ({d})")).unwrap_or_default();
            outln!(
                "started {started}   duration {}   outcome {outcome}{detail}",
                fmt_duration_ms(ms)
            );
        }
        None => outln!("started {started}   cut off: no run_finished line"),
    }
    outln!(
        "{} lines, {} unparsable",
        log.records.len(),
        log.skipped.len()
    );
    if let Some(h) = log.header()
        && let Some(rate) = h.controller.get("stream_rate_hz").and_then(Value::as_f64)
    {
        outln!("stream {rate:.0} Hz");
    }

    // Actions at depth 0: what the routine itself did, with time spent.
    #[derive(Default)]
    struct Agg {
        n: usize,
        total_ms: f64,
        max_ms: f64,
        failed: usize,
        cut: usize,
    }
    let mut actions: BTreeMap<String, Agg> = BTreeMap::new();
    for node in log.action_tree() {
        let a = actions.entry(node.name.clone()).or_default();
        a.n += 1;
        match node.duration_ms {
            Some(ms) => {
                a.total_ms += ms;
                a.max_ms = a.max_ms.max(ms);
            }
            None => a.cut += 1,
        }
        if node.failed() {
            a.failed += 1;
        }
    }
    if !actions.is_empty() {
        outln!();
        outln!(
            "{:<28} {:>5} {:>10} {:>10} {:>7}",
            "actions (depth 0)",
            "n",
            "total",
            "max",
            "failed"
        );
        let mut rows: Vec<_> = actions.into_iter().collect();
        rows.sort_by(|a, b| b.1.total_ms.total_cmp(&a.1.total_ms));
        for (name, a) in rows {
            let cut = if a.cut > 0 {
                format!("  ({} cut off)", a.cut)
            } else {
                String::new()
            };
            outln!(
                "  {:<26} {:>5} {:>10} {:>10} {:>7}{cut}",
                name,
                a.n,
                fmt_duration_ms(a.total_ms),
                fmt_duration_ms(a.max_ms),
                a.failed
            );
        }
    }

    // Measurements by label, with the spread of `value` when it is numeric.
    struct Meas {
        n: usize,
        values: Vec<f64>,
        unstable: usize,
    }
    let mut measurements: BTreeMap<String, Meas> = BTreeMap::new();
    for r in &log.records {
        if let Body::DataCollected { label, value } = &r.body {
            let m = measurements.entry(label.clone()).or_insert(Meas {
                n: 0,
                values: Vec::new(),
                unstable: 0,
            });
            m.n += 1;
            if let Some(v) = value.get("value").and_then(Value::as_f64) {
                m.values.push(v);
            }
            if value.get("stable") == Some(&Value::Bool(false)) {
                m.unstable += 1;
            }
        }
    }
    if !measurements.is_empty() {
        outln!();
        outln!("measurements");
        for (label, m) in measurements {
            let mut line = format!("  {:<26} {:>5}", label, m.n);
            if !m.values.is_empty() {
                let n = m.values.len() as f64;
                let mean = m.values.iter().sum::<f64>() / n;
                let min = m.values.iter().cloned().fold(f64::INFINITY, f64::min);
                let max = m.values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let _ = write!(line, "   value mean {mean:.3}  min {min:.3}  max {max:.3}");
            }
            if m.unstable > 0 {
                let _ = write!(line, "   {} failed the gates", m.unstable);
            }
            outln!("{line}");
        }
    }

    // Custom events by kind; phase-like events list their values.
    let mut kinds: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for r in &log.records {
        if let Body::Custom { kind, data } = &r.body {
            kinds.entry(kind.clone()).or_default().push(data);
        }
    }
    if !kinds.is_empty() {
        outln!();
        outln!("events");
        for (kind, datas) in kinds {
            let mut line = format!("  {:<26} {:>5}", kind, datas.len());
            // A kind whose data has a "phase" tag reads best as a sequence.
            let phases: Vec<&str> = datas
                .iter()
                .filter_map(|d| d.get("phase").and_then(Value::as_str))
                .collect();
            if !phases.is_empty() && phases.len() == datas.len() {
                let _ = write!(line, "   {}", phases.join(" > "));
            }
            outln!("{line}");
        }
    }
    Ok(())
}

// ============================================================================
// timeline
// ============================================================================

fn timeline(log: &Log, args: &TimelineArgs) -> Result<(), Box<dyn Error>> {
    let tree = log.action_tree();
    let mut roots: Vec<&ActionNode> = Vec::new();
    match &args.action {
        Some(name) => {
            for root in &tree {
                root.walk(&mut |n| {
                    if n.name == *name {
                        roots.push(n);
                    }
                });
            }
        }
        None => roots.extend(tree.iter()),
    }
    let base_depth = roots.first().map(|n| n.depth).unwrap_or(0);
    for root in roots {
        root.walk(&mut |n| {
            let rel = n.depth - base_depth;
            if args.max_depth.is_some_and(|m| rel > m) {
                return;
            }
            if args.failed && !(n.failed() || n.duration_ms.is_none()) {
                return;
            }
            let status = match (&n.error, n.duration_ms) {
                (Some(e), Some(ms)) => format!("{:>10}  FAILED: {e}", fmt_duration_ms(ms)),
                (Some(e), None) => format!("{:>10}  FAILED: {e}", ""),
                (None, Some(ms)) => format!("{:>10}", fmt_duration_ms(ms)),
                (None, None) => format!("{:>10}", "cut off"),
            };
            outln!(
                "{:>10.3}s  {}{:<28} {}  {}",
                n.time_s,
                "  ".repeat(rel),
                n.name,
                status,
                fmt_params(&n.params)
            );
        });
    }
    if !log.skipped.is_empty() {
        eprintln!("({} unparsable lines skipped)", log.skipped.len());
    }
    Ok(())
}

/// Scalar params as `k=v` pairs; nested values are elided.
fn fmt_params(params: &Value) -> String {
    let Some(map) = params.as_object() else {
        return String::new();
    };
    map.iter()
        .filter_map(|(k, v)| match v {
            Value::Null | Value::Object(_) | Value::Array(_) => None,
            Value::String(s) => Some(format!("{k}={s}")),
            other => Some(format!("{k}={other}")),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ============================================================================
// plot
// ============================================================================

/// The series a log offers: declared custom kinds with their numeric
/// fields, and measurement labels with the numeric fields seen.
fn available_series(log: &Log) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(h) = log.header() {
        for kind in &h.schema.kinds {
            for (field, ty) in kind.scalar_fields() {
                if ty == "number" || ty == "integer" {
                    out.push(format!("{}.{field}", kind.kind));
                }
            }
        }
    }
    let mut labels: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in &log.records {
        if let Body::DataCollected { label, value } = &r.body
            && let Some(obj) = value.as_object()
        {
            let fields = labels.entry(label.clone()).or_default();
            for (k, v) in obj {
                if v.is_number() && !fields.contains(k) {
                    fields.push(k.clone());
                }
            }
        }
    }
    for (label, fields) in labels {
        for f in fields {
            out.push(format!("{label}.{f}"));
        }
    }
    out
}

/// `(time_s, value)` points for a series selector.
fn series_points(log: &Log, selector: &str) -> Result<Vec<(f64, f64)>, Box<dyn Error>> {
    let (source, field) = selector
        .rsplit_once('.')
        .ok_or("series must be kind.field or label.field")?;
    let points: Vec<(f64, f64)> = log
        .records
        .iter()
        .filter_map(|r| {
            let data = match &r.body {
                Body::Custom { kind, data } if kind == source => data,
                Body::DataCollected { label, value } if label == source => value,
                _ => return None,
            };
            let v = data.get(field)?.as_f64()?;
            Some((log.time_s(r), v))
        })
        .collect();
    if points.is_empty() {
        return Err(format!(
            "no numeric values for {selector}; available:\n  {}",
            available_series(log).join("\n  ")
        )
        .into());
    }
    Ok(points)
}

fn plot(log: &Log, args: &PlotArgs) -> Result<(), Box<dyn Error>> {
    let Some(selector) = &args.series else {
        let series = available_series(log);
        if series.is_empty() {
            outln!("nothing numeric to plot");
        } else {
            outln!("series in this log:");
            for s in series {
                outln!("  {s}");
            }
        }
        return Ok(());
    };
    let points = series_points(log, selector)?;
    let (x0, x1) = (points[0].0 as f32, points[points.len() - 1].0 as f32);
    let pts: Vec<(f32, f32)> = points.iter().map(|(t, v)| (*t as f32, *v as f32)).collect();
    let (min, max) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, v)| {
            (lo.min(*v), hi.max(*v))
        });
    outln!(
        "{selector}: {} points over {}, min {min:.3}, max {max:.3}",
        points.len(),
        fmt_duration_ms((points[points.len() - 1].0 - points[0].0) * 1000.0)
    );
    Chart::new(args.width, args.height, x0, x1.max(x0 + 1.0))
        .lineplot(&Shape::Lines(&pts))
        .lineplot(&Shape::Points(&pts))
        .display();
    outln!("time (s) →");
    Ok(())
}

// ============================================================================
// export
// ============================================================================

fn export(log: &Log, args: &ExportArgs) -> Result<(), Box<dyn Error>> {
    let stem = args
        .file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "log".into());
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| args.file.with_file_name(&stem));
    fs::create_dir_all(&out)?;

    // run.json: the header, so the tables stay attributable.
    if let Some(h) = log.header() {
        let run = serde_json::json!({
            "tool": h.tool, "version": h.version, "git_commit": h.git_commit,
            "started_utc": log.started_at().map(fmt_utc),
            "outcome": log.finished().map(|(o, _, _)| o),
            "detail": log.finished().and_then(|(_, d, _)| d),
            "duration_ms": log.finished().map(|(_, _, ms)| ms),
            "config": h.config, "controller": h.controller,
        });
        fs::write(out.join("run.json"), serde_json::to_string_pretty(&run)?)?;
    }

    // actions.csv: every action at every depth, one row each.
    let mut rows: Vec<Vec<String>> = Vec::new();
    for root in log.action_tree() {
        root.walk(&mut |n| {
            rows.push(vec![
                n.seq.to_string(),
                fmt_num(n.time_s),
                n.depth.to_string(),
                n.name.clone(),
                n.duration_ms.map(fmt_num).unwrap_or_default(),
                if n.failed() {
                    "failed"
                } else if n.duration_ms.is_some() {
                    "ok"
                } else {
                    "cut_off"
                }
                .to_string(),
                n.error.clone().unwrap_or_default(),
                n.params.to_string(),
            ]);
        });
    }
    rows.sort_by_key(|r| r[0].parse::<u64>().unwrap_or(0));
    write_csv(
        &out.join("actions.csv"),
        &[
            "seq",
            "time_s",
            "depth",
            "action",
            "duration_ms",
            "status",
            "error",
            "params_json",
        ],
        &rows,
    )?;

    // measurements.csv: one table per label, columns from the values seen.
    let mut by_label: BTreeMap<String, Vec<&Record>> = BTreeMap::new();
    for r in &log.records {
        if let Body::DataCollected { label, .. } = &r.body {
            by_label.entry(label.clone()).or_default().push(r);
        }
    }
    for (label, records) in &by_label {
        let mut fields: Vec<String> = Vec::new();
        for r in records {
            if let Body::DataCollected { value, .. } = &r.body
                && let Some(obj) = value.as_object()
            {
                for (k, v) in obj {
                    if !v.is_object() && !v.is_array() && !fields.contains(k) {
                        fields.push(k.clone());
                    }
                }
            }
        }
        let rows = records
            .iter()
            .map(|r| {
                let Body::DataCollected { value, .. } = &r.body else {
                    unreachable!()
                };
                let mut row = vec![r.seq.to_string(), fmt_num(log.time_s(r))];
                row.extend(fields.iter().map(|f| cell(value.get(f))));
                row
            })
            .collect::<Vec<_>>();
        let header: Vec<&str> = ["seq", "time_s"]
            .into_iter()
            .chain(fields.iter().map(String::as_str))
            .collect();
        write_csv(&out.join(format!("{label}.csv")), &header, &rows)?;
    }

    // A stream dump is a time series, not a row per event: one file each,
    // time down the side, a column per signal.
    for r in &log.records {
        if let Body::Custom { kind, data } = &r.body
            && kind == STREAM_DUMP
        {
            write_stream_dump(&out.join(format!("stream_dump_{}.csv", r.seq)), log, data)?;
        }
    }

    // One table per declared custom kind, columns from its schema.
    let mut by_kind: BTreeMap<String, Vec<&Record>> = BTreeMap::new();
    for r in &log.records {
        if let Body::Custom { kind, .. } = &r.body
            && kind != STREAM_DUMP
        {
            by_kind.entry(kind.clone()).or_default().push(r);
        }
    }
    for (kind, records) in &by_kind {
        let declared: Option<Vec<String>> = log.header().and_then(|h| {
            h.schema
                .kinds
                .iter()
                .find(|k| k.kind == *kind)
                .map(|k| k.scalar_fields().into_iter().map(|(f, _)| f).collect())
        });
        // Undeclared kinds (legacy logs) get the union of scalar keys seen.
        let fields = declared.unwrap_or_else(|| {
            let mut fields = Vec::new();
            for r in records {
                if let Body::Custom { data, .. } = &r.body
                    && let Some(obj) = data.as_object()
                {
                    for (k, v) in obj {
                        if !v.is_object() && !v.is_array() && !fields.contains(k) {
                            fields.push(k.clone());
                        }
                    }
                }
            }
            fields
        });
        let rows = records
            .iter()
            .map(|r| {
                let Body::Custom { data, .. } = &r.body else {
                    unreachable!()
                };
                let mut row = vec![r.seq.to_string(), fmt_num(log.time_s(r))];
                row.extend(fields.iter().map(|f| cell(data.get(f))));
                row
            })
            .collect::<Vec<_>>();
        let header: Vec<&str> = ["seq", "time_s"]
            .into_iter()
            .chain(fields.iter().map(String::as_str))
            .collect();
        write_csv(
            &out.join(format!("{}.csv", kind.replace('/', "_"))),
            &header,
            &rows,
        )?;
    }

    outln!("wrote {}", out.display());
    let mut names: Vec<_> = fs::read_dir(&out)?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    names.sort();
    for n in names {
        outln!("  {n}");
    }
    Ok(())
}

fn cell(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(other) => other.to_string(),
    }
}

const STREAM_DUMP: &str = "routine/stream_dump";

/// Write a `routine/stream_dump` as `t_s` plus one column per signal, named
/// from the header's signal list. Values are written in exponent form, since
/// a current in amperes does not survive `fmt_num`'s six decimals.
fn write_stream_dump(path: &Path, log: &Log, data: &Value) -> std::io::Result<()> {
    let stream = &data["stream"];
    let as_f64s = |v: &Value| -> Vec<f64> {
        v.as_array()
            .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect())
            .unwrap_or_default()
    };
    let t_s = as_f64s(&stream["t_s"]);
    let columns: Vec<Vec<f64>> = stream["columns"]
        .as_array()
        .map(|cols| cols.iter().map(as_f64s).collect())
        .unwrap_or_default();
    let names: Vec<String> = stream["signals"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_u64)
                .map(|i| signal_name(log, i))
                .collect()
        })
        .unwrap_or_default();

    let header: Vec<&str> = std::iter::once("t_s")
        .chain(names.iter().map(String::as_str))
        .collect();
    let rows: Vec<Vec<String>> = t_s
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut row = vec![fmt_num(*t)];
            row.extend(
                columns
                    .iter()
                    // The stream is f32; narrowing drops the digits the
                    // f64 widening in the JSON added.
                    .map(|c| {
                        c.get(i)
                            .map(|&v| format!("{:e}", v as f32))
                            .unwrap_or_default()
                    }),
            );
            row
        })
        .collect();
    write_csv(path, &header, &rows)
}

/// The header's first name for a signal index, or `signal_<index>`.
fn signal_name(log: &Log, index: u64) -> String {
    log.header()
        .and_then(|h| h.controller["signals"].as_array())
        .and_then(|sigs| sigs.iter().find(|s| s["index"].as_u64() == Some(index)))
        .and_then(|s| s["name"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("signal_{index}"))
}

fn write_csv(path: &Path, header: &[&str], rows: &[Vec<String>]) -> std::io::Result<()> {
    let mut w = std::io::BufWriter::new(fs::File::create(path)?);
    writeln!(
        w,
        "{}",
        header
            .iter()
            .map(|h| csv_quote(h))
            .collect::<Vec<_>>()
            .join(",")
    )?;
    for row in rows {
        writeln!(
            w,
            "{}",
            row.iter()
                .map(|c| csv_quote(c))
                .collect::<Vec<_>>()
                .join(",")
        )?;
    }
    w.flush()
}

fn csv_quote(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ============================================================================
// formatting
// ============================================================================

/// Microsecond precision, trailing zeros trimmed: timestamps carry no more,
/// and durations are already in milliseconds.
fn fmt_num(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0');
    s.trim_end_matches('.').to_string()
}

fn fmt_utc(epoch_s: f64) -> String {
    DateTime::<Utc>::from_timestamp(epoch_s as i64, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

fn fmt_duration_ms(ms: f64) -> String {
    let s = ms / 1000.0;
    if s < 1.0 {
        format!("{ms:.1} ms")
    } else if s < 60.0 {
        format!("{s:.1} s")
    } else if s < 3600.0 {
        format!("{}m {:02}s", (s / 60.0) as u64, (s % 60.0) as u64)
    } else {
        format!(
            "{}h {:02}m",
            (s / 3600.0) as u64,
            ((s % 3600.0) / 60.0) as u64
        )
    }
}
