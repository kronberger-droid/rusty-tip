//! The reads for tuning: the feedback loops, the scan, a scan frame, and a
//! streamed signal's spectrum.
//!
//! Each only asks; none changes anything on the instrument. A frame's
//! pixels go to a file in a directory the server picks, never one the
//! request names, so a read-only server cannot be made to write where a
//! client says; the reply names the file and carries per-line statistics,
//! which is what tells a slow loop from a ringing one without reading
//! megabytes of pixels.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};

use super::{ErrorKind, Reply, spectrum};
use crate::session::{Session, unit_of};
use crate::signal_registry::SignalRegistry;
use crate::spm_error::SpmError;

/// Every feedback loop the controller exposes, read once each.
pub(super) fn controllers(session: &mut Session) -> Reply {
    let readings = session.query(|controller, _| {
        controller
            .controllers()?
            .into_iter()
            .map(|id| controller.read_controller(id))
            .collect::<Result<Vec<_>, _>>()
    });
    Reply::of(readings.map(|readings| json!({ "controllers": readings })))
}

/// The scan: where the frame sits, what the buffer records, how fast,
/// and whether it runs.
pub(super) fn scan(session: &mut Session) -> Reply {
    let scan = session.query(|controller, registry| {
        let frame = controller.scan_frame_get()?;
        let buffer = controller.scan_buffer_get()?;
        let speed = controller.scan_speed_get()?;
        let props = controller.scan_props_get()?;
        let running = controller.scan_status()?;
        let signals: Vec<Value> = buffer
            .channels
            .iter()
            .map(|c| json!({ "index": c.0, "name": signal_name(registry, c.0) }))
            .collect();
        Ok(json!({
            "running": running,
            "frame": {
                "center_m": [short(frame.center.x as f32), short(frame.center.y as f32)],
                "width_m": short(frame.width_m),
                "height_m": short(frame.height_m),
                "angle_deg": short(frame.angle_deg),
            },
            "buffer": {
                "signals": signals,
                "pixels": buffer.pixels,
                "lines": buffer.lines,
            },
            "speed": {
                "forward_m_s": short(speed.forward_linear_speed_m_s),
                "backward_m_s": short(speed.backward_linear_speed_m_s),
                "forward_time_per_line_s": short(speed.forward_time_per_line_s),
                "backward_time_per_line_s": short(speed.backward_time_per_line_s),
                "keep_constant": match speed.keep_parameter_constant {
                    0 => "linear_speed",
                    _ => "time_per_line",
                },
                "backward_to_forward_ratio": short(speed.speed_ratio),
            },
            "continuous": props.continuous_scan,
            "bouncy": props.bouncy_scan,
        }))
    });
    Reply::of(scan)
}

/// An f32 from the controller as the f64 its shortest decimal form
/// names: `5e-8`, not the `5.000000058430487e-8` widening gives.
fn short(value: f32) -> f64 {
    value.to_string().parse().unwrap_or(f64::from(value))
}

fn signal_name(registry: &SignalRegistry, index: u32) -> Option<String> {
    u8::try_from(index)
        .ok()
        .and_then(|i| registry.get_by_index(i))
        .map(|s| s.name.clone())
}

/// What a frame request can go wrong with before the controller does.
enum FrameError {
    BadRequest(String),
    Controller(SpmError),
}

impl From<SpmError> for FrameError {
    fn from(e: SpmError) -> Self {
        FrameError::Controller(e)
    }
}

/// The file a frame is written to: the pixels as the controller sent
/// them, both directions.
#[derive(Serialize)]
struct FrameFile<'a> {
    signal: &'a str,
    unit: Option<String>,
    index: u8,
    /// Whether the slow axis ran up. Rows are in the order the controller
    /// sent them.
    scan_up: bool,
    forward: &'a [Vec<f32>],
    backward: &'a [Vec<f32>],
}

/// One signal's current frame, both directions: the pixels to a file, the
/// per-line statistics in the reply.
pub(super) fn frame(session: &mut Session, signal: &str) -> Reply {
    let dir = frames_dir(session.log_dir());
    let grabbed = session.query(|controller, registry| {
        let buffer = controller.scan_buffer_get()?;
        let found = registry
            .get_by_name(signal)
            .filter(|s| buffer.channels.contains(&s.signal_index()))
            .cloned();
        let Some(found) = found else {
            let recorded: Vec<String> = buffer
                .channels
                .iter()
                .map(|c| signal_name(registry, c.0).unwrap_or_else(|| c.0.to_string()))
                .collect();
            return Ok(Err(FrameError::BadRequest(format!(
                "the scan does not record {signal}; it records {}",
                recorded.join(", ")
            ))));
        };
        let channel = found.signal_index();
        let (_, forward, scan_up) = controller.scan_frame_data_grab(channel, true)?;
        let (_, backward, _) = controller.scan_frame_data_grab(channel, false)?;
        Ok(Ok((found, forward, backward, scan_up)))
    });
    let (found, forward, backward, scan_up) = match grabbed {
        Ok(Ok(grabbed)) => grabbed,
        Ok(Err(FrameError::BadRequest(message))) => {
            return Reply::err(ErrorKind::BadRequest, message);
        }
        Ok(Err(FrameError::Controller(e))) | Err(e) => {
            return Reply::err(ErrorKind::of(&e), e.to_string());
        }
    };

    let stats = FrameStats::of(&forward, &backward);
    let unit = unit_of(&found.name);
    let file = FrameFile {
        signal: &found.name,
        unit: unit.clone(),
        index: found.index,
        scan_up,
        forward: &forward,
        backward: &backward,
    };
    let path = match write_frame(&dir, &found.name, &file) {
        Ok(path) => path,
        Err(e) => return Reply::err(ErrorKind::Failed, e),
    };
    Reply::ok(json!({
        "signal": found.name,
        "unit": unit,
        "index": found.index,
        "file": path,
        "pixels": forward.first().map_or(0, Vec::len),
        "lines": forward.len(),
        "scan_up": scan_up,
        "stats": stats,
    }))
}

/// How many peaks a `psd` reply lists.
const PSD_PEAKS: usize = 8;

/// One streamed signal's power spectral density over `samples` evenly
/// spaced stream samples, Welch-averaged in segments of `segment`.
///
/// Only a signal on the data stream will do: polled samples have no time
/// base, so their spectrum would put lines at frequencies that are not
/// there.
pub(super) fn psd(session: &mut Session, signal: &str, samples: usize, segment: usize) -> Reply {
    let taken = session.query(|controller, registry| {
        let Some(found) = registry.get_by_name(signal).cloned() else {
            return Ok(Err(Reply::err(
                ErrorKind::BadRequest,
                format!("no signal called {signal}"),
            )));
        };
        if !controller.streams_signal(found.signal_index()) {
            // The registry lists aliases too; name each streamed signal once.
            let mut streamed: Vec<(u8, String)> = registry
                .tcp_signals()
                .into_iter()
                .filter(|s| controller.streams_signal(s.signal_index()))
                .map(|s| (s.index, s.name.clone()))
                .collect();
            streamed.sort();
            streamed.dedup_by_key(|(index, _)| *index);
            let streamed: Vec<String> = streamed.into_iter().map(|(_, name)| name).collect();
            return Ok(Err(Reply::err(
                ErrorKind::BadRequest,
                format!(
                    "{} is not on the data stream, and a spectrum needs evenly spaced \
                     samples; the stream carries {}",
                    found.name,
                    streamed.join(", ")
                ),
            )));
        }
        let Some(rate_hz) = controller.stream_rate_hz() else {
            return Ok(Err(Reply::err(
                ErrorKind::Refused,
                "the data stream reports no sample rate, so the spectrum has no frequency axis",
            )));
        };
        let values = controller.read_signal_samples(found.signal_index(), samples)?;
        Ok(Ok((found, rate_hz, values)))
    });
    let (found, rate_hz, values) = match taken {
        Ok(Ok(taken)) => taken,
        Ok(Err(reply)) => return reply,
        Err(e) => return Reply::of(Err(e)),
    };
    if values.len() < segment {
        return Reply::err(
            ErrorKind::Failed,
            format!(
                "the stream gave {} samples, fewer than one segment of {segment}",
                values.len()
            ),
        );
    }

    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let rms = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt();
    let spectrum = spectrum::welch(&values, rate_hz, segment);
    let peaks = spectrum::peaks(&spectrum, PSD_PEAKS);
    Reply::ok(json!({
        "signal": found.name,
        "unit": unit_of(&found.name),
        "index": found.index,
        "rate_hz": rate_hz,
        "samples": values.len(),
        "rms": rms,
        "peaks": peaks,
        "spectrum": spectrum,
    }))
}

/// Where frame files go: a `frames` directory beside the job logs, or
/// under the system's temporary directory when the session keeps none.
fn frames_dir(log_dir: Option<&Path>) -> PathBuf {
    match log_dir {
        Some(dir) => dir.join("frames"),
        None => std::env::temp_dir().join("rusty-tip").join("frames"),
    }
}

/// Write the frame and return its absolute path. The log directory is
/// often relative (`./experiments`), and a path relative to the server's
/// working directory means nothing to a client started somewhere else.
fn write_frame(dir: &Path, signal: &str, file: &FrameFile<'_>) -> Result<PathBuf, String> {
    let dir =
        std::path::absolute(dir).map_err(|e| format!("cannot resolve {}: {e}", dir.display()))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let stem: String = signal
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = dir.join(format!("frame-{millis}-{stem}.json"));
    let text = serde_json::to_string(file).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// Per-line statistics of a frame, as columns: entry `i` of each list is
/// line `i`, `null` where the line is not scanned yet.
///
/// A line's RMS is taken after removing its mean and slope, so tilt does
/// not count as roughness. The trace-retrace RMS is of the difference
/// between the two directions after removing its mean, which is
/// `retrace_offset`: a loop that lags shows as features shifted between
/// the directions, so as this difference, while the offset alone is
/// mostly hysteresis and creep.
///
/// The directions are compared pixel for pixel as the controller sends
/// them, the same way every frame, so frames stay comparable across a
/// change of gains. Whether Nanonis sends backward rows mirrored has not
/// been checked on hardware; `mean.trace_retrace_rms_mirrored` compares
/// them mirrored too, and on a frame with features, whichever of the two is
/// clearly smaller tells the way.
#[derive(Debug, Serialize)]
pub(crate) struct FrameStats {
    pub lines_scanned: usize,
    /// Means over the scanned lines.
    pub mean: Summary,
    pub lines: Lines,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Summary {
    pub rms_forward: Option<f64>,
    pub rms_backward: Option<f64>,
    pub retrace_offset: Option<f64>,
    pub trace_retrace_rms: Option<f64>,
    /// `trace_retrace_rms` with the backward rows mirrored.
    pub trace_retrace_rms_mirrored: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Lines {
    pub mean_forward: Vec<Option<f64>>,
    pub rms_forward: Vec<Option<f64>>,
    pub rms_backward: Vec<Option<f64>>,
    pub retrace_offset: Vec<Option<f64>>,
    pub trace_retrace_rms: Vec<Option<f64>>,
}

impl FrameStats {
    pub fn of(forward: &[Vec<f32>], backward: &[Vec<f32>]) -> Self {
        let mut lines = Lines::default();
        let mut mirrored = Vec::new();
        for (i, f) in forward.iter().enumerate() {
            let b = backward.get(i).map(Vec::as_slice).unwrap_or(&[]);
            let (offset, rms) = match difference(f, b, false) {
                Some((offset, rms)) => (Some(offset), Some(rms)),
                None => (None, None),
            };
            mirrored.push(difference(f, b, true).map(|(_, rms)| rms));
            lines.mean_forward.push(mean(f));
            lines.rms_forward.push(detrended_rms(f));
            lines.rms_backward.push(detrended_rms(b));
            lines.retrace_offset.push(offset);
            lines.trace_retrace_rms.push(rms);
        }
        let average = |column: &[Option<f64>]| {
            let values: Vec<f64> = column.iter().flatten().copied().collect();
            (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
        };
        Self {
            lines_scanned: lines.rms_forward.iter().flatten().count(),
            mean: Summary {
                rms_forward: average(&lines.rms_forward),
                rms_backward: average(&lines.rms_backward),
                retrace_offset: average(&lines.retrace_offset),
                trace_retrace_rms: average(&lines.trace_retrace_rms),
                trace_retrace_rms_mirrored: average(&mirrored),
            },
            lines,
        }
    }
}

/// The finite pixels of a line with their positions along it.
fn finite(line: &[f32]) -> impl Iterator<Item = (f64, f64)> + '_ {
    line.iter()
        .enumerate()
        .filter(|(_, v)| v.is_finite())
        .map(|(x, &v)| (x as f64, f64::from(v)))
}

fn mean(line: &[f32]) -> Option<f64> {
    let (n, sum) = finite(line).fold((0usize, 0.0), |(n, s), (_, v)| (n + 1, s + v));
    (n > 0).then(|| sum / n as f64)
}

/// RMS about the least-squares line through the finite pixels; `None`
/// with fewer than two.
fn detrended_rms(line: &[f32]) -> Option<f64> {
    let points: Vec<(f64, f64)> = finite(line).collect();
    if points.len() < 2 {
        return None;
    }
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    let ss: f64 = points
        .iter()
        .map(|p| (p.1 - my - slope * (p.0 - mx)).powi(2))
        .sum();
    Some((ss / n).sqrt())
}

/// The mean of backward minus forward over the pixels both have, and the
/// RMS of that difference about its mean; `None` with fewer than two such
/// pixels.
fn difference(forward: &[f32], backward: &[f32], mirrored: bool) -> Option<(f64, f64)> {
    if forward.len() != backward.len() {
        return None;
    }
    let len = forward.len();
    let diffs: Vec<f64> = (0..len)
        .filter_map(|x| {
            let b = backward[if mirrored { len - 1 - x } else { x }];
            let f = forward[x];
            (f.is_finite() && b.is_finite()).then(|| f64::from(b) - f64::from(f))
        })
        .collect();
    if diffs.len() < 2 {
        return None;
    }
    let n = diffs.len() as f64;
    let offset = diffs.iter().sum::<f64>() / n;
    let rms = (diffs.iter().map(|d| (d - offset).powi(2)).sum::<f64>() / n).sqrt();
    Some((offset, rms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-9 * b.abs().max(1.0))
    }

    #[test]
    fn a_tilt_is_not_roughness() {
        let line: Vec<f32> = (0..64).map(|x| 0.5 * x as f32 + 3.0).collect();
        assert!(close(detrended_rms(&line), 0.0));
        let bumpy: Vec<f32> = (0..64)
            .map(|x| if x % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let rms = detrended_rms(&bumpy).unwrap();
        assert!((rms - 1.0).abs() < 0.01, "{rms}");
    }

    /// A partial frame reads NaN past the last scanned line; those lines
    /// come back `null` and do not enter the means.
    #[test]
    fn unscanned_lines_are_null_and_left_out() {
        let scanned: Vec<f32> = (0..16).map(|x| (x % 2) as f32).collect();
        let unscanned = vec![f32::NAN; 16];
        let forward = vec![scanned.clone(), unscanned.clone()];
        let backward = vec![scanned, unscanned];
        let stats = FrameStats::of(&forward, &backward);
        assert_eq!(stats.lines_scanned, 1);
        assert_eq!(stats.lines.rms_forward[1], None);
        let rms = stats.mean.rms_forward.unwrap();
        assert!((rms - 0.5).abs() < 0.02, "{rms}");
        assert!(close(stats.mean.trace_retrace_rms, 0.0));
    }

    /// A feature shifted between the directions is what a lagging loop
    /// leaves; a constant offset between them is not.
    #[test]
    fn a_shift_shows_as_trace_retrace_rms_and_an_offset_does_not() {
        let step = |at: usize| -> Vec<f32> { (0..32).map(|x| (x >= at) as u8 as f32).collect() };
        let offset: Vec<f32> = step(16).iter().map(|v| v + 2.0).collect();
        let same = FrameStats::of(&[step(16)], &[offset]);
        assert!(close(same.mean.retrace_offset, 2.0));
        assert!(close(same.mean.trace_retrace_rms, 0.0));

        let shifted = FrameStats::of(&[step(16)], &[step(20)]);
        assert!(shifted.mean.trace_retrace_rms.unwrap() > 0.1);
    }

    /// The comparison is always as sent, so it cannot change between
    /// frames; the mirrored figure is there to tell which way is right.
    #[test]
    fn rows_are_compared_as_sent_with_the_mirrored_figure_beside() {
        let forward = vec![(0..32).map(|x| x as f32).collect::<Vec<f32>>()];
        let mirrored = vec![forward[0].iter().rev().copied().collect::<Vec<f32>>()];
        let stats = FrameStats::of(&forward, &mirrored);
        assert!(stats.mean.trace_retrace_rms.unwrap() > 1.0);
        assert!(close(stats.mean.trace_retrace_rms_mirrored, 0.0));
        let same = FrameStats::of(&forward, &forward);
        assert!(close(same.mean.trace_retrace_rms, 0.0));
        assert!(same.mean.trace_retrace_rms_mirrored.unwrap() > 1.0);
    }

    #[test]
    fn frames_go_beside_the_logs_or_to_the_temp_dir() {
        assert_eq!(
            frames_dir(Some(Path::new("/logs"))),
            Path::new("/logs/frames")
        );
        assert!(frames_dir(None).starts_with(std::env::temp_dir()));
    }
}
