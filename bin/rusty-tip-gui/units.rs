//! Units, in one place: values are SI everywhere in the code and the logs,
//! and only the screen sees `pA`, `nm` or `mV`.
//!
//! Two ways to show a value. A form field has a fixed display unit, so the
//! number does not change its prefix under the cursor while it is dragged:
//! [`display_unit_for`] picks that unit from the base unit when the schema
//! does not, and [`prefix_scale`] says how to scale into it. A readout or a
//! status line takes the prefix that fits the value ([`format_si`]), so a
//! current reads `120.0 pA` and a bias `-500.0 mV`.

/// How many of `display_unit` make one base unit, from the SI prefix it
/// starts with: `pA` gives 1e12, `nm/s` 1e9. `None` for a bare unit,
/// including ones whose first letter happens to be a prefix (`m/s`, `Hz`).
pub fn prefix_scale(display_unit: &str) -> Option<f64> {
    let first = display_unit.chars().next()?;
    // Only when the rest is a plain base unit, so `m/s` is not "milli".
    let rest = &display_unit[first.len_utf8()..];
    if rest.is_empty() || !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    PREFIXES
        .iter()
        .find(|(_, p)| p.starts_with(first) || (first == 'u' && *p == "µ"))
        .map(|(scale, _)| *scale)
}

/// The unit a form field shows a base unit in when its schema names none:
/// currents in picoamperes, lengths and speeds in nanometres, the rest as
/// they are.
pub fn display_unit_for(base: &str) -> &str {
    match base {
        "A" => "pA",
        "m" => "nm",
        "m/s" => "nm/s",
        other => other,
    }
}

/// A value in its base unit, shown with the SI prefix that fits it and four
/// significant digits: `1.2e-10 A` reads `120.0 pA`. Units that do not take
/// a prefix (`steps`, `ms`, `%`) are shown as they are.
pub fn format_si(value: f64, base: &str) -> String {
    if !takes_prefix(base) || value == 0.0 || !value.is_finite() {
        return format!("{} {base}", significant(value));
    }
    let magnitude = value.abs();
    let (scale, prefix) = PREFIXES
        .iter()
        .rev()
        .find(|(scale, _)| magnitude * scale >= 1.0)
        .unwrap_or(&PREFIXES[0]);
    format!("{} {prefix}{base}", significant(value * scale))
}

/// An axis tick: like [`format_si`], with the trailing zeros dropped, so
/// the ticks of one axis read `0 Hz`, `-5 Hz`, `-10 Hz` rather than
/// `0.000 Hz` beside `-10.00 Hz`.
pub fn format_tick(value: f64, base: &str) -> String {
    let text = format_si(value, base);
    let Some((num, unit)) = text.split_once(' ') else {
        return text;
    };
    let num = if num.contains('.') {
        num.trim_end_matches('0').trim_end_matches('.')
    } else {
        num
    };
    let num = if num == "-0" { "0" } else { num };
    format!("{num} {unit}")
}

/// Four significant digits, fixed decimals for a given magnitude so a
/// readout does not jitter in width as it changes.
pub fn significant(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return format!("{value:.3}");
    }
    let digits = value.abs().log10().floor() as i32;
    let decimals = (3 - digits).clamp(0, 6) as usize;
    format!("{value:.decimals$}")
}

/// A number with no unit, for a log line: whole numbers as they are, the
/// rest to four decimals with the trailing zeros dropped, and anything
/// tiny or huge in scientific notation rather than as a row of zeros.
pub fn number(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{value:.0}");
    }
    let magnitude = value.abs();
    if !(1e-3..1e6).contains(&magnitude) {
        return format!("{value:.3e}");
    }
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Each prefix with the factor that takes a base value into it, from the
/// smallest unit up. Multipliers rather than divisors, since `1.0 / 1e-9`
/// is not exactly `1e9` and the form compares scaled values for equality.
const PREFIXES: &[(f64, &str)] = &[
    (1e12, "p"),
    (1e9, "n"),
    (1e6, "µ"),
    (1e3, "m"),
    (1.0, ""),
    (1e-3, "k"),
    (1e-6, "M"),
];

/// Whether a base unit takes an SI prefix in this app. `ms` and `steps`
/// do not; `Hz` does, since a sample rate reads better as `1.000 kHz`.
fn takes_prefix(base: &str) -> bool {
    matches!(
        base,
        "A" | "V" | "m" | "Hz" | "s" | "W" | "m/s" | "Hz/s" | "V/s"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_drop_trailing_zeros() {
        assert_eq!(format_tick(0.0, "Hz"), "0 Hz");
        assert_eq!(format_tick(-10.0, "Hz"), "-10 Hz");
        assert_eq!(format_tick(-2.5, "Hz"), "-2.5 Hz");
        assert_eq!(format_tick(4.0, "V"), "4 V");
        assert_eq!(format_tick(1.2e-10, "A"), "120 pA");
    }

    #[test]
    fn prefixes_scale_and_bare_units_do_not() {
        assert_eq!(prefix_scale("pA"), Some(1e12));
        assert_eq!(prefix_scale("mV"), Some(1e3));
        assert_eq!(prefix_scale("nm/s"), Some(1e9));
        assert_eq!(prefix_scale("uA"), Some(1e6), "u stands in for µ");
        assert_eq!(prefix_scale("m/s"), None, "metres per second is not milli");
        assert_eq!(prefix_scale("Hz"), None);
        assert_eq!(prefix_scale("V"), None);
    }

    #[test]
    fn default_display_units() {
        assert_eq!(display_unit_for("A"), "pA");
        assert_eq!(display_unit_for("m/s"), "nm/s");
        assert_eq!(display_unit_for("Hz"), "Hz");
        assert_eq!(display_unit_for("steps"), "steps");
    }

    #[test]
    fn values_take_the_prefix_that_fits() {
        assert_eq!(format_si(1.2e-10, "A"), "120.0 pA");
        assert_eq!(format_si(-0.5, "V"), "-500.0 mV");
        assert_eq!(format_si(-1.2345e-9, "m"), "-1.234 nm");
        assert_eq!(format_si(-12.345, "Hz"), "-12.35 Hz");
        assert_eq!(format_si(1000.0, "Hz"), "1.000 kHz");
        assert_eq!(format_si(5e-9, "m/s"), "5.000 nm/s");
        assert_eq!(format_si(0.0, "A"), "0.000 A");
        assert_eq!(format_si(50.0, "ms"), "50.00 ms", "ms takes no prefix");
        assert_eq!(format_si(2.0, "steps"), "2.000 steps");
    }

    #[test]
    fn log_numbers_are_short() {
        assert_eq!(number(3.0), "3");
        assert_eq!(number(-1.5), "-1.5");
        assert_eq!(number(0.123456), "0.1235");
        assert_eq!(number(1e-10), "1.000e-10");
        assert_eq!(number(12345678.9), "1.235e7");
        assert_eq!(number(2.5e7), "25000000", "a whole number is a count");
    }
}
