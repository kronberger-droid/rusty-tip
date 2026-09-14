//! Reading and writing Nanonis multi-pass configuration files (`.mpas`).
//!
//! Multi-pass makes the Scan Control module scan each line several times, with
//! per-pass overrides for bias, setpoint, speed and lock-in, and with one
//! signal recordable in one pass and playable back (plus a scalar offset) in a
//! later one. The GUI can save that configuration to a `.mpas` file, and the
//! TCP interface can load one back (`MPass.Load`) and switch multi-pass on
//! (`MPass.Activate`). It cannot build a configuration over TCP, which is what
//! this module is for: write the file, then load it.
//!
//! # The format
//!
//! SPECS does not document it; everything here was worked out by saving
//! configurations from the GUI and diffing them. It is a LabVIEW-style INI
//! file with CRLF line endings, a blank line between sections, and **no
//! trailing newline**:
//!
//! ```text
//! [Callback VI]\r\n
//! Selector = 0\r\n
//! Path = ""\r\n
//! \r\n
//! [Pass1]\r\n
//! Acq ch = "\00\00\00\00"\r\n
//! Rec state = TRUE\r\n
//! Rec ch = 30\r\n
//! ...
//! Switch Lock-In = FALSE          <- end of file, no CRLF
//! ```
//!
//! One `[PassN]` section per scan *direction*, numbered from 1, and this is the
//! part most likely to trip you up: a section is not a pass. Sections alternate
//! forward and backward, so `[Pass1]` and `[Pass2]` together are the single
//! pass the GUI lists as "Fwd #1" and "Bwd #1", and a two-pass experiment needs
//! four sections. The manual says the same thing in passing, when it gives the
//! callback VI's arguments for "a scan with N lines and 2 passes" as
//! `(0,0), (0,1), (0,2), (0,3)`. See [`PassDirection`].
//!
//! Booleans are `TRUE` and
//! `FALSE`. Floats are LabVIEW engineering notation: six decimals, exponent a
//! multiple of three, sign always present, no zero padding, so 0.5 is written
//! `500.000000E-3`. The one exception is `End Time`, which is written as plain
//! fixed-point `0.800000`. That inconsistency is Nanonis's, and it is stable:
//! saving, loading and re-saving a configuration reproduces the file byte for
//! byte, which is what [`MultiPassConfig`]'s `Display` is tested against.
//!
//! # What is not in the file
//!
//! - **Scan mode (Normal vs Linefeed).** Ticking Linefeed in the GUI does not
//!   change a single byte, and there is no TCP call for it either. It has to be
//!   set by hand in the GUI, and it is worth re-checking after a restart.
//! - **Forward vs backward.** Not stored as a field, because it is positional:
//!   the section index decides it. There is one record buffer per direction and
//!   they do not mix, so a signal recorded in a forward section can only be
//!   played back in a later forward section. Get this wrong and the GUI shows
//!   "No Channel recorded" against a pass that has Play switched on.
//! - **Acquisition channels.** `Acq ch` is a four-byte blob whose encoding is
//!   still unknown; it stayed `"\00\00\00\00"` across every configuration
//!   saved so far. It is carried through verbatim rather than interpreted.
//!
//! # Units and precision
//!
//! Metres, volts, seconds and amps, matching the rest of the Nanonis
//! interface. Confirmed for the play offset: loading `210.000000E-12` shows as
//! `210p` in the Multi Pass window, and a Z-recording pass displays its offset
//! in `m`. What is still open is the offset's **sign**, i.e. whether a positive
//! value approaches the surface or retracts from it.
//!
//! The controller narrows to `f32`. Loading an offset of 210 pm and asking the
//! controller to save its active configuration returns `210.000003E-12`, which
//! is exactly `2.1e-10_f32` in this notation. So a round trip through the file
//! is exact, a round trip through the machine is not, and comparing the two
//! byte for byte will fail on any value `f32` cannot hold.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::signal_registry::SignalIndex;

/// The default `Acq ch` blob: four NUL bytes in LabVIEW's hex escaping.
const ACQ_CH_DEFAULT: &str = r"\00\00\00\00";

/// A whole multi-pass configuration: the callback VI plus the passes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiPassConfig {
    /// The `[Callback VI]` section, shared by every pass.
    pub callback_vi: CallbackVi,
    /// The passes, in file order. `passes[0]` is `[Pass1]`.
    pub passes: Vec<Pass>,
}

/// The LabVIEW VI called at the beginning of each line, if any.
///
/// Using one needs LabVIEW running on the host with VI Server enabled, so it is
/// off in everything written from here; the fields exist so a GUI-authored file
/// survives a round trip.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CallbackVi {
    /// `0` is None. `1` is presumed to be "User VI"; not yet confirmed.
    pub selector: i32,
    /// Path to the VI, empty when `selector` is 0.
    pub path: String,
}

/// One pass of a multi-pass scan.
///
/// The fields mirror the file one for one, including the pairs where a boolean
/// gates a value (`rec_state`/`rec_ch`, `bias_override`/`bias_override_value`).
/// Nanonis keeps the value when the flag goes off, so collapsing the pair into
/// an `Option` would quietly rewrite files it merely meant to read. Use
/// [`Pass::recorded`], [`Pass::played`] and friends for the tidier view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pass {
    /// Opaque four-byte acquisition-channel blob, carried through verbatim.
    pub acq_ch: String,
    /// Record a signal during this pass.
    pub rec_state: bool,
    /// RT signal slot to record, in the same numbering as the rest of the
    /// Nanonis interface (Current is 0, Z (m) is 30 on a stock assignment).
    pub rec_ch: i32,
    /// Play back the recorded signal during this pass.
    pub play_state: bool,
    /// Offset added to the played-back signal; metres, for a Z playback. The
    /// sign convention is not yet confirmed against hardware.
    pub play_offset: f64,
    /// Rate at which the playback offset is ramped in. 0 or infinity applies it
    /// instantly.
    pub play_slew_rate: f64,
    /// "Initial Time" in the GUI: the wait at the beginning of the line, in
    /// seconds. With a playback signal it is the wait *before* the offset is
    /// applied.
    pub delay: f64,
    /// Wait after applying the playback offset, in seconds. Only meaningful
    /// when playing a signal back.
    pub end_time: f64,
    /// Enum behind the speed control. `2` is what the GUI wrote for a custom
    /// speed; the other values are not yet known, `0` included.
    pub speed_sel: i32,
    /// Speed value for this pass. The GUI labels it "Speed Ratio", so it most
    /// likely multiplies the regular scan speed, but that is not confirmed and
    /// it is not known how `speed_sel` changes the reading. See the warning on
    /// [`Pass::default`].
    pub speed_value: f64,
    /// Override the bias for this pass.
    pub bias_override: bool,
    /// Bias to use when `bias_override` is set, in volts.
    pub bias_override_value: f64,
    /// Override the Z controller setpoint for this pass.
    pub z_setp_override: bool,
    /// Setpoint to use when `z_setp_override` is set.
    pub z_setp_override_value: f64,
    /// Run the shared callback VI at the beginning of this pass.
    pub run_callback_vi: bool,
    /// How long to wait for the callback VI before scanning anyway, in seconds.
    pub callback_vi_timeout: f64,
    /// Run a real-time script at the beginning of this pass.
    pub run_script: bool,
    /// Name of that script, as deployed in the Scripting Tool.
    pub script_name: String,
    /// Wait `delay` only after the callback VI or script has finished, rather
    /// than at the very start of the line.
    pub apply_offset_after_callback_vi: bool,
    /// Switch the lock-in on for this pass.
    pub switch_lock_in: bool,
}

impl Default for Pass {
    /// A pass that does nothing beyond scanning the line.
    ///
    /// These are our defaults, not Nanonis's: the GUI's own idea of a fresh
    /// pass has never been saved to a file, so the only values here taken from
    /// an observed file are `acq_ch` and the 10 s callback timeout.
    ///
    /// `speed_sel: 0` with `speed_value: 1.0` displays as "Speed Ratio 1" and
    /// is the reason to be careful: if `0` ever selects an *absolute* speed
    /// rather than a ratio, 1 m/s is a catastrophic scan speed. Save a fresh
    /// default pass from the GUI and check before running this on hardware.
    fn default() -> Self {
        Self {
            acq_ch: ACQ_CH_DEFAULT.to_string(),
            rec_state: false,
            rec_ch: 0,
            play_state: false,
            play_offset: 0.0,
            play_slew_rate: 0.0,
            delay: 0.0,
            end_time: 0.0,
            speed_sel: 0,
            speed_value: 1.0,
            bias_override: false,
            bias_override_value: 0.0,
            z_setp_override: false,
            z_setp_override_value: 0.0,
            run_callback_vi: false,
            callback_vi_timeout: 10.0,
            run_script: false,
            script_name: String::new(),
            apply_offset_after_callback_vi: false,
            switch_lock_in: false,
        }
    }
}

impl Pass {
    /// Record `signal` during this pass.
    pub fn record(mut self, signal: SignalIndex) -> Self {
        self.rec_state = true;
        self.rec_ch = signal.0 as i32;
        self
    }

    /// Play the recorded signal back during this pass, offset by `offset`
    /// (metres, for Z) and ramped in at `slew_rate` per second. A slew rate of
    /// zero applies the offset instantly.
    pub fn play(mut self, offset: f64, slew_rate: f64) -> Self {
        self.play_state = true;
        self.play_offset = offset;
        self.play_slew_rate = slew_rate;
        self
    }

    /// Which signal this pass records, if any.
    pub fn recorded(&self) -> Option<SignalIndex> {
        match self.rec_state && self.rec_ch >= 0 {
            true => Some(SignalIndex(self.rec_ch as u32)),
            false => None,
        }
    }

    /// The playback offset and slew rate, if this pass plays a signal back.
    pub fn played(&self) -> Option<(f64, f64)> {
        self.play_state
            .then_some((self.play_offset, self.play_slew_rate))
    }

    /// The bias override, if set.
    pub fn bias(&self) -> Option<f64> {
        self.bias_override.then_some(self.bias_override_value)
    }

    /// The Z controller setpoint override, if set.
    pub fn z_setpoint(&self) -> Option<f64> {
        self.z_setp_override.then_some(self.z_setp_override_value)
    }
}

/// Which scan direction a `[PassN]` section drives.
///
/// Sections alternate, starting forward: `[Pass1]` is forward, `[Pass2]`
/// backward, `[Pass3]` forward again. Record buffers are per direction and do
/// not mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PassDirection {
    Forward,
    Backward,
}

impl MultiPassConfig {
    /// Direction of the section at `index` (0-based, so `index` 0 is
    /// `[Pass1]`).
    pub fn direction(index: usize) -> PassDirection {
        match index.is_multiple_of(2) {
            true => PassDirection::Forward,
            false => PassDirection::Backward,
        }
    }

    /// The label the GUI shows for the section at `index`, e.g. `"Bwd #2"`.
    /// Handy for printing a configuration back to someone who is looking at the
    /// Multi Pass window.
    pub fn label(index: usize) -> String {
        let side = match Self::direction(index) {
            PassDirection::Forward => "Fwd",
            PassDirection::Backward => "Bwd",
        };
        format!("{side} #{}", index / 2 + 1)
    }

    /// A configuration with `passes` sections and no callback VI.
    pub fn new(passes: Vec<Pass>) -> Self {
        Self {
            callback_vi: CallbackVi::default(),
            passes,
        }
    }

    /// The two-pass constant-lift configuration: record `signal` during the
    /// first pass, play it back during the second raised by `lift` metres.
    ///
    /// That is four sections, not two. Both directions of pass 1 record, both
    /// directions of pass 2 play, because the record buffers are per direction
    /// and a backward section cannot play what a forward section recorded.
    ///
    /// This is the whole of what multi-pass can do towards constant-distance
    /// scanning on its own, and it is worth having as a baseline: it is
    /// `Z_tip = Z + lift`, a plain vertical shift, whereas the trajectory from
    /// [`crate::analyzer::rolling_ellipsoid`] stands the tip further off at
    /// step edges. Comparing the two on the same area is the point.
    ///
    /// The sign of `lift` is not yet pinned down against hardware. The
    /// published method (Moreno et al., Nano Lett. 2015) approaches the surface
    /// by 0.21 to 0.32 nm during the second pass.
    pub fn constant_lift(signal: SignalIndex, lift: f64) -> Self {
        Self::new(vec![
            // Fwd #1, Bwd #1: record.
            Pass::default().record(signal),
            Pass::default().record(signal),
            // Fwd #2, Bwd #2: play it back, offset.
            Pass::default().play(lift, 0.0),
            Pass::default().play(lift, 0.0),
        ])
    }

    /// Sections that play a signal back with nothing to play.
    ///
    /// Record buffers are per direction and do not mix, so a playback section
    /// needs an earlier section of the *same* direction that records. Nanonis
    /// does not reject the configuration: the Multi Pass window shows "No
    /// Channel recorded" and the pass plays nothing, which is exactly how our
    /// first two-section attempt looked correct until the GUI was opened.
    ///
    /// Returns section indices, to be named with [`Self::label`]. This is a
    /// query rather than an error in [`Self::write`] because a GUI-authored
    /// file can trip it legitimately: the buffer survives from an earlier scan,
    /// so a configuration that plays before it records is odd but not invalid.
    pub fn unsourced_playbacks(&self) -> Vec<usize> {
        (0..self.passes.len())
            .filter(|&i| self.passes[i].play_state)
            .filter(|&i| {
                !self.passes[..i]
                    .iter()
                    .enumerate()
                    .any(|(j, p)| Self::direction(j) == Self::direction(i) && p.rec_state)
            })
            .collect()
    }

    /// Read a `.mpas` file.
    pub fn read(path: impl AsRef<Path>) -> io::Result<Self> {
        fs::read_to_string(path)?.parse()
    }

    /// Write a `.mpas` file, in the byte-exact form Nanonis itself writes.
    pub fn write(&self, path: impl AsRef<Path>) -> io::Result<()> {
        if self.passes.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mpas: refusing to write a configuration with no passes",
            ));
        }
        fs::write(path, self.to_string())
    }
}

impl fmt::Display for MultiPassConfig {
    /// Renders the file, CRLF endings and all, without a trailing newline.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut lines: Vec<String> = vec![
            "[Callback VI]".to_string(),
            format!("Selector = {}", self.callback_vi.selector),
            format!("Path = \"{}\"", self.callback_vi.path),
        ];

        for (n, p) in self.passes.iter().enumerate() {
            lines.push(String::new());
            lines.push(format!("[Pass{}]", n + 1));
            // Key order is not something a parser cares about, but matching the
            // GUI's makes our files diffable against GUI-saved ones.
            for (key, value) in [
                ("Acq ch", format!("\"{}\"", p.acq_ch)),
                ("Rec state", fmt_bool(p.rec_state)),
                ("Rec ch", p.rec_ch.to_string()),
                ("Play state", fmt_bool(p.play_state)),
                ("Play offset", fmt_eng(p.play_offset)),
                ("Delay", fmt_eng(p.delay)),
                ("Speed sel", p.speed_sel.to_string()),
                ("Speed value", fmt_eng(p.speed_value)),
                ("Bias override", fmt_bool(p.bias_override)),
                ("Bias override value", fmt_eng(p.bias_override_value)),
                ("Z Setp override", fmt_bool(p.z_setp_override)),
                ("Z Setp override value", fmt_eng(p.z_setp_override_value)),
                ("Run Callback VI", fmt_bool(p.run_callback_vi)),
                ("Callback VI Timeout", fmt_eng(p.callback_vi_timeout)),
                ("Play slew rate", fmt_eng(p.play_slew_rate)),
                ("Run Script", fmt_bool(p.run_script)),
                ("Script name", format!("\"{}\"", p.script_name)),
                // The one field Nanonis writes in plain fixed-point.
                ("End Time", format!("{:.6}", p.end_time)),
                (
                    "Apply Offset after Callback VI",
                    fmt_bool(p.apply_offset_after_callback_vi),
                ),
                ("Switch Lock-In", fmt_bool(p.switch_lock_in)),
            ] {
                lines.push(format!("{key} = {value}"));
            }
        }

        f.write_str(&lines.join("\r\n"))
    }
}

impl FromStr for MultiPassConfig {
    type Err = io::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let sections = parse_ini(text)?;

        let callback_vi = match sections.get("Callback VI") {
            Some(s) => CallbackVi {
                selector: int(s, "Callback VI", "Selector")?,
                path: string(s, "Path"),
            },
            None => CallbackVi::default(),
        };

        // Passes are numbered from 1 and must be contiguous; a gap means we
        // misread the file rather than that Nanonis skipped a pass.
        let mut numbered: Vec<(u32, &Section)> = sections
            .iter()
            .filter_map(|(name, s)| {
                name.strip_prefix("Pass")
                    .and_then(|n| n.parse::<u32>().ok())
                    .map(|n| (n, s))
            })
            .collect();
        numbered.sort_by_key(|(n, _)| *n);

        if numbered.is_empty() {
            return Err(invalid("no [PassN] sections found; not a .mpas file"));
        }
        for (i, (n, _)) in numbered.iter().enumerate() {
            if *n as usize != i + 1 {
                return Err(invalid(&format!(
                    "passes are numbered {:?}, expected 1..={}",
                    numbered.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
                    numbered.len()
                )));
            }
        }

        let passes = numbered
            .into_iter()
            .map(|(n, s)| parse_pass(s, &format!("Pass{n}")))
            .collect::<io::Result<Vec<_>>>()?;

        Ok(Self {
            callback_vi,
            passes,
        })
    }
}

type Section = BTreeMap<String, String>;

/// Sections keyed by name, each a map of key to unquoted value. Note the
/// keying is lexicographic, so `Pass10` sorts before `Pass2`; the caller sorts
/// numerically before relying on pass order.
fn parse_ini(text: &str) -> io::Result<BTreeMap<String, Section>> {
    let mut sections: BTreeMap<String, Section> = BTreeMap::new();
    let mut current: Option<String> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = Some(name.trim().to_string());
            sections.entry(name.trim().to_string()).or_default();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(invalid(&format!(
                "line is neither section nor key: {line:?}"
            )));
        };
        let Some(name) = &current else {
            return Err(invalid(&format!("key {key:?} before any section")));
        };
        sections
            .get_mut(name)
            .expect("section was inserted above")
            .insert(key.trim().to_string(), unquote(value.trim()).to_string());
    }
    Ok(sections)
}

fn parse_pass(s: &Section, name: &str) -> io::Result<Pass> {
    // Every key is required. The file is machine-written and always complete,
    // so a missing key means a version difference worth knowing about rather
    // than something to paper over with a default: defaulting `Play offset` to
    // zero would silently plan a pass that replays Z with no lift at all.
    Ok(Pass {
        acq_ch: string(s, "Acq ch"),
        rec_state: boolean(s, name, "Rec state")?,
        rec_ch: int(s, name, "Rec ch")?,
        play_state: boolean(s, name, "Play state")?,
        play_offset: float(s, name, "Play offset")?,
        play_slew_rate: float(s, name, "Play slew rate")?,
        delay: float(s, name, "Delay")?,
        end_time: float(s, name, "End Time")?,
        speed_sel: int(s, name, "Speed sel")?,
        speed_value: float(s, name, "Speed value")?,
        bias_override: boolean(s, name, "Bias override")?,
        bias_override_value: float(s, name, "Bias override value")?,
        z_setp_override: boolean(s, name, "Z Setp override")?,
        z_setp_override_value: float(s, name, "Z Setp override value")?,
        run_callback_vi: boolean(s, name, "Run Callback VI")?,
        callback_vi_timeout: float(s, name, "Callback VI Timeout")?,
        run_script: boolean(s, name, "Run Script")?,
        script_name: string(s, "Script name"),
        apply_offset_after_callback_vi: boolean(s, name, "Apply Offset after Callback VI")?,
        switch_lock_in: boolean(s, name, "Switch Lock-In")?,
    })
}

fn get<'a>(s: &'a Section, section: &str, key: &str) -> io::Result<&'a str> {
    s.get(key)
        .map(String::as_str)
        .ok_or_else(|| invalid(&format!("[{section}] has no {key:?}")))
}

fn boolean(s: &Section, section: &str, key: &str) -> io::Result<bool> {
    match get(s, section, key)?.to_ascii_uppercase().as_str() {
        "TRUE" => Ok(true),
        "FALSE" => Ok(false),
        other => Err(invalid(&format!(
            "[{section}] {key} is {other:?}, expected TRUE or FALSE"
        ))),
    }
}

fn int(s: &Section, section: &str, key: &str) -> io::Result<i32> {
    let value = get(s, section, key)?;
    value
        .parse()
        .map_err(|_| invalid(&format!("[{section}] {key} is not an integer: {value:?}")))
}

fn float(s: &Section, section: &str, key: &str) -> io::Result<f64> {
    let value = get(s, section, key)?;
    // Rust's float parser already accepts LabVIEW's `500.000000E-3`.
    value
        .parse()
        .map_err(|_| invalid(&format!("[{section}] {key} is not a number: {value:?}")))
}

/// Strings are optional: an absent one reads the same as an empty one.
fn string(s: &Section, key: &str) -> String {
    s.get(key).cloned().unwrap_or_default()
}

fn unquote(value: &str) -> &str {
    match value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        true => &value[1..value.len() - 1],
        false => value,
    }
}

fn fmt_bool(b: bool) -> String {
    match b {
        true => "TRUE".to_string(),
        false => "FALSE".to_string(),
    }
}

/// Format a number the way LabVIEW does: six decimals, exponent a multiple of
/// three with its sign always shown, so 0.5 becomes `500.000000E-3` and 3
/// becomes `3.000000E+0`.
fn fmt_eng(v: f64) -> String {
    if !v.is_finite() || v == 0.0 {
        return "0.000000E+0".to_string();
    }

    // Stepping by factors of 1000 rather than going through log10 keeps the
    // exact powers (1e-3, 1e-12) off the boundary where rounding picks the
    // wrong decade.
    let (mut mantissa, mut exponent) = (v, 0i32);
    while mantissa.abs() >= 1000.0 {
        mantissa /= 1000.0;
        exponent += 3;
    }
    while mantissa.abs() < 1.0 {
        mantissa *= 1000.0;
        exponent -= 3;
    }
    // Rounding to six decimals can carry 999.9999999 up into the next decade.
    if format!("{:.6}", mantissa.abs()).starts_with("1000") {
        mantissa /= 1000.0;
        exponent += 3;
    }

    format!("{mantissa:.6}E{exponent:+}")
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("mpas: {msg}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A configuration saved from the Nanonis GUI with every option set to
    /// something distinctive, and Z (m) selected as the recorded signal in the
    /// first pass. This is the format specification: reading it and writing it
    /// back has to reproduce it exactly.
    fn golden() -> String {
        const LF: &str = r#"[Callback VI]
Selector = 0
Path = ""

[Pass1]
Acq ch = "\00\00\00\00"
Rec state = TRUE
Rec ch = 30
Play state = TRUE
Play offset = 3.000000E+0
Delay = 8.000000E+0
Speed sel = 2
Speed value = 5.000000E+0
Bias override = TRUE
Bias override value = 3.000000E+0
Z Setp override = TRUE
Z Setp override value = 3.000000E+0
Run Callback VI = FALSE
Callback VI Timeout = 10.000000E+0
Play slew rate = 5.000000E+0
Run Script = FALSE
Script name = ""
End Time = 0.800000
Apply Offset after Callback VI = FALSE
Switch Lock-In = FALSE

[Pass2]
Acq ch = "\00\00\00\00"
Rec state = TRUE
Rec ch = 0
Play state = TRUE
Play offset = 3.000000E+0
Delay = 500.000000E-3
Speed sel = 2
Speed value = 4.000000E+0
Bias override = TRUE
Bias override value = 3.000000E+0
Z Setp override = TRUE
Z Setp override value = 4.000000E+0
Run Callback VI = FALSE
Callback VI Timeout = 10.000000E+0
Play slew rate = 3.000000E+0
Run Script = FALSE
Script name = ""
End Time = 3.000000
Apply Offset after Callback VI = FALSE
Switch Lock-In = FALSE"#;
        LF.replace('\n', "\r\n")
    }

    #[test]
    fn round_trip_is_byte_exact() {
        let text = golden();
        let config: MultiPassConfig = text.parse().expect("golden file parses");
        assert_eq!(config.to_string(), text);
    }

    #[test]
    fn parses_the_fields_the_gui_set() {
        let config: MultiPassConfig = golden().parse().unwrap();

        assert_eq!(config.callback_vi.selector, 0);
        assert_eq!(config.callback_vi.path, "");
        assert_eq!(config.passes.len(), 2);

        let p1 = &config.passes[0];
        // Z (m) is RT signal slot 30 on a stock assignment. The GUI writing 30
        // here is what pins `Rec ch` to the RT slot numbering rather than to an
        // index into the assigned scan signals.
        assert_eq!(p1.recorded(), Some(SignalIndex(30)));
        assert_eq!(p1.played(), Some((3.0, 5.0)));
        assert_eq!(p1.bias(), Some(3.0));
        assert_eq!(p1.z_setpoint(), Some(3.0));
        assert_eq!(p1.delay, 8.0);
        assert_eq!(p1.end_time, 0.8);
        assert_eq!(p1.acq_ch, ACQ_CH_DEFAULT);

        let p2 = &config.passes[1];
        // Only `Rec ch` differed between the two passes in this file.
        assert_eq!(p2.recorded(), Some(SignalIndex(0)));
        assert_eq!(p2.speed_value, 4.0);
        assert_eq!(p2.z_setpoint(), Some(4.0));
        assert_eq!(p2.end_time, 3.0);
    }

    #[test]
    fn tolerates_lf_endings_and_stray_whitespace() {
        // The writer is strict so files match Nanonis byte for byte; the reader
        // should not be, so a hand-edited file still loads.
        let relaxed = golden().replace("\r\n", "\n").replace(" = ", "=");
        assert_eq!(
            relaxed.parse::<MultiPassConfig>().unwrap(),
            golden().parse::<MultiPassConfig>().unwrap()
        );
    }

    #[test]
    fn engineering_notation_matches_labview() {
        // Every case the GUI has been observed to write.
        assert_eq!(fmt_eng(3.0), "3.000000E+0");
        assert_eq!(fmt_eng(10.0), "10.000000E+0");
        assert_eq!(fmt_eng(0.5), "500.000000E-3");
        // Not yet observed, but implied by the rule: mantissa in [1, 1000),
        // exponent a multiple of three.
        assert_eq!(fmt_eng(1e-10), "100.000000E-12");
        assert_eq!(fmt_eng(1234.5), "1.234500E+3");
        assert_eq!(fmt_eng(-0.002), "-2.000000E-3");
        assert_eq!(fmt_eng(0.0), "0.000000E+0");
    }

    #[test]
    fn engineering_notation_survives_a_parse() {
        for value in [3.0, 0.5, 1e-10, -2.5e-9, 1234.5, 42.0] {
            let text = fmt_eng(value);
            let back: f64 = text.parse().unwrap_or_else(|_| panic!("{text} parses"));
            assert!(
                (back - value).abs() <= value.abs() * 1e-6,
                "{value} formatted as {text} came back as {back}"
            );
        }
    }

    #[test]
    fn constant_lift_records_in_both_directions_then_plays_in_both() {
        // A section is one direction of a pass, so two passes is four sections.
        // Recording only in the forward section leaves the backward play with
        // an empty buffer, which the GUI reports as "No Channel recorded".
        let config = MultiPassConfig::constant_lift(SignalIndex(30), 100e-12);
        assert_eq!(config.passes.len(), 4);

        for i in [0, 1] {
            assert_eq!(config.passes[i].recorded(), Some(SignalIndex(30)));
            let name = MultiPassConfig::label(i);
            assert!(!config.passes[i].play_state, "{name} should not play");
        }
        for i in [2, 3] {
            assert_eq!(config.passes[i].played(), Some((100e-12, 0.0)));
            let name = MultiPassConfig::label(i);
            assert!(
                config.passes[i].recorded().is_none(),
                "{name} should not record"
            );
        }

        // Round trips through the file like anything else.
        let text = config.to_string();
        assert_eq!(text.parse::<MultiPassConfig>().unwrap(), config);
        assert!(text.contains("Play offset = 100.000000E-12"));
    }

    #[test]
    fn sections_alternate_forward_and_backward() {
        // The GUI's own labels, which is how this was discovered: our two-section
        // file came back as "Fwd #1" and "Bwd #1", not as two passes.
        assert_eq!(MultiPassConfig::label(0), "Fwd #1");
        assert_eq!(MultiPassConfig::label(1), "Bwd #1");
        assert_eq!(MultiPassConfig::label(2), "Fwd #2");
        assert_eq!(MultiPassConfig::label(3), "Bwd #2");
        assert_eq!(MultiPassConfig::direction(0), PassDirection::Forward);
        assert_eq!(MultiPassConfig::direction(3), PassDirection::Backward);
    }

    #[test]
    fn a_playback_with_nothing_recorded_in_that_direction_is_reported() {
        // The original bug: record forward, play backward. Nanonis accepts it
        // and the backward pass plays nothing at all.
        let config = MultiPassConfig::new(vec![
            Pass::default().record(SignalIndex(30)),
            Pass::default().play(210e-12, 0.0),
        ]);
        assert_eq!(config.unsourced_playbacks(), vec![1]);
        assert_eq!(MultiPassConfig::label(1), "Bwd #1");

        // The four-section version has a source for both playbacks.
        assert!(
            MultiPassConfig::constant_lift(SignalIndex(30), 210e-12)
                .unsourced_playbacks()
                .is_empty()
        );
    }

    #[test]
    fn a_gap_in_the_pass_numbering_is_an_error() {
        let text = golden().replace("[Pass2]", "[Pass3]");
        assert!(text.parse::<MultiPassConfig>().is_err());
    }

    #[test]
    fn a_missing_key_is_an_error_rather_than_a_default() {
        let text = golden().replace("Play offset = 3.000000E+0\r\n", "");
        let err = text.parse::<MultiPassConfig>().unwrap_err();
        assert!(
            err.to_string().contains("Play offset"),
            "error should name the missing key, got: {err}"
        );
    }

    #[test]
    fn writes_and_reads_a_file() {
        let path = crate::utils::temp_path("mpas-round-trip", "mpas");

        let config: MultiPassConfig = golden().parse().unwrap();
        config.write(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), golden());
        assert_eq!(MultiPassConfig::read(&path).unwrap(), config);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn refuses_to_write_a_configuration_with_no_passes() {
        let path = crate::utils::temp_path("mpas-empty", "mpas");
        assert!(MultiPassConfig::new(vec![]).write(&path).is_err());
        assert!(!path.exists());
    }
}
