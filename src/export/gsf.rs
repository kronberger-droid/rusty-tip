//! Gwyddion Simple Field (`.gsf`) reader and writer.
//!
//! GSF is Gwyddion's deliberately minimal interchange format, and it is a good
//! fit here because it carries physical dimensions: the file says "this image
//! is 20 nm across and the values are metres", so nothing has to be re-entered
//! by hand on import.
//!
//! The layout, per the format specification:
//!
//! ```text
//! Gwyddion Simple Field 1.0\n     <- magic line, LF-terminated
//! XRes = 256\n                    <- text header, `key = value` lines
//! YRes = 256\n
//! XReal = 2e-8\n                  <- optional, defaults to 1.0
//! ...
//! \0\0\0                          <- 1 to 4 NUL bytes, aligning data to a
//!                                    multiple of 4 bytes from file start
//! <XRes*YRes little-endian f32>   <- rows top to bottom, left to right
//! ```
//!
//! Note the padding rule: it is one *to* four NULs, never zero. When the
//! header already lands on a multiple of four, a full four NULs are added.
//!
//! Values are stored as `f32`. That is the format's choice, not ours, and it is
//! worth knowing what it costs: 24 bits of mantissa across a Z range of, say,
//! 20 nm resolves about 1e-15 m. Far below anything an SPM measures, so the
//! narrowing is harmless here, but the round trip is lossy and the tests below
//! only assert agreement to `f32` precision.

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;

use ndarray::{Array2, ArrayView2};

use super::super::analyzer::rolling_ellipsoid::GridSpacing;

const MAGIC: &str = "Gwyddion Simple Field 1.0";

/// A 2D field with the physical metadata GSF can carry.
#[derive(Debug, Clone)]
pub struct GsfField {
    /// Values, row-major `[row, column]`, in the units named by `z_units`.
    pub data: Array2<f64>,
    /// Physical width of the field (columns axis), in `xy_units`.
    pub x_real: f64,
    /// Physical height of the field (rows axis), in `xy_units`.
    pub y_real: f64,
    /// Physical offset of the field origin along x.
    pub x_offset: f64,
    /// Physical offset of the field origin along y.
    pub y_offset: f64,
    /// Unit of the lateral axes, e.g. `"m"`. Empty means dimensionless.
    pub xy_units: String,
    /// Unit of the values, e.g. `"m"`. Empty means dimensionless.
    pub z_units: String,
    /// Channel title shown in Gwyddion's data browser.
    pub title: Option<String>,
}

impl GsfField {
    /// Build a field in metres from a map and its sample spacing.
    ///
    /// The physical extent is `n * d` rather than `(n - 1) * d`, because a
    /// sample represents the area it covers, not a dimensionless point. This
    /// matches how a scan frame size relates to its sample count.
    pub fn from_grid(data: Array2<f64>, sp: GridSpacing, title: impl Into<String>) -> Self {
        let (ny, nx) = data.dim();
        Self {
            x_real: nx as f64 * sp.dx,
            y_real: ny as f64 * sp.dy,
            x_offset: 0.0,
            y_offset: 0.0,
            xy_units: "m".into(),
            z_units: "m".into(),
            title: Some(title.into()),
            data,
        }
    }

    /// Sample spacing implied by the stored extent and resolution.
    pub fn spacing(&self) -> GridSpacing {
        let (ny, nx) = self.data.dim();
        GridSpacing {
            dx: self.x_real / nx as f64,
            dy: self.y_real / ny as f64,
        }
    }
}

/// Everything a GSF file carries except the samples themselves.
///
/// Split out so the writer can take a borrowed view of the data: at scan sizes
/// the map is several megabytes, and copying it just to hand it over is waste
/// the caller cannot avoid otherwise.
struct Header<'a> {
    x_real: f64,
    y_real: f64,
    x_offset: f64,
    y_offset: f64,
    xy_units: &'a str,
    z_units: &'a str,
    title: Option<&'a str>,
}

/// Write a field as a `.gsf` file.
pub fn write_gsf(path: impl AsRef<Path>, field: &GsfField) -> io::Result<()> {
    write_view(
        path,
        field.data.view(),
        &Header {
            x_real: field.x_real,
            y_real: field.y_real,
            x_offset: field.x_offset,
            y_offset: field.y_offset,
            xy_units: &field.xy_units,
            z_units: &field.z_units,
            title: field.title.as_deref(),
        },
    )
}

fn write_view(path: impl AsRef<Path>, data: ArrayView2<f64>, field: &Header) -> io::Result<()> {
    let (ny, nx) = data.dim();
    if ny == 0 || nx == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to write an empty GSF field",
        ));
    }

    let mut header = format!("{MAGIC}\nXRes = {nx}\nYRes = {ny}\n");
    header.push_str(&format!(
        "XReal = {:e}\nYReal = {:e}\n",
        field.x_real, field.y_real
    ));
    header.push_str(&format!(
        "XOffset = {:e}\nYOffset = {:e}\n",
        field.x_offset, field.y_offset
    ));
    if !field.xy_units.is_empty() {
        header.push_str(&format!("XYUnits = {}\n", field.xy_units));
    }
    if !field.z_units.is_empty() {
        header.push_str(&format!("ZUnits = {}\n", field.z_units));
    }
    if let Some(title) = field.title {
        header.push_str(&format!("Title = {title}\n"));
    }

    let mut out = BufWriter::new(File::create(path)?);
    out.write_all(header.as_bytes())?;
    // One to four NULs, never zero: a header already aligned to 4 still gets a
    // full padding block.
    let pad = 4 - (header.len() % 4);
    out.write_all(&[0u8; 4][..pad])?;

    // One write for the whole payload. Per-sample writes cost a BufWriter call
    // each, which at scan sizes is a million of them for no benefit.
    let mut payload = Vec::with_capacity(ny * nx * 4);
    for value in data.iter() {
        payload.extend_from_slice(&(*value as f32).to_le_bytes());
    }
    out.write_all(&payload)?;
    out.flush()
}

/// Read a `.gsf` file back into a field.
///
/// Mainly here so the writer can be round-trip tested, but it also means a map
/// exported earlier can be fed back in as the input to a new plan.
pub fn read_gsf(path: impl AsRef<Path>) -> io::Result<GsfField> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;

    // The header is ASCII up to the first NUL; the payload starts after the
    // padding run.
    let nul = bytes
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| invalid("no NUL padding found; not a GSF file"))?;
    let header = std::str::from_utf8(&bytes[..nul]).map_err(|_| invalid("header is not UTF-8"))?;

    let mut lines = header.lines();
    if lines.next() != Some(MAGIC) {
        return Err(invalid("missing the GSF magic line"));
    }

    let mut field = GsfField {
        data: Array2::zeros((0, 0)),
        x_real: 1.0,
        y_real: 1.0,
        x_offset: 0.0,
        y_offset: 0.0,
        xy_units: String::new(),
        z_units: String::new(),
        title: None,
    };
    let (mut nx, mut ny) = (None, None);

    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let number = |what: &str| -> io::Result<f64> {
            value
                .parse::<f64>()
                .map_err(|_| invalid(&format!("{what} is not a number: {value:?}")))
        };
        match key {
            "XRes" => nx = Some(value.parse::<usize>().map_err(|_| invalid("bad XRes"))?),
            "YRes" => ny = Some(value.parse::<usize>().map_err(|_| invalid("bad YRes"))?),
            "XReal" => field.x_real = number("XReal")?,
            "YReal" => field.y_real = number("YReal")?,
            "XOffset" => field.x_offset = number("XOffset")?,
            "YOffset" => field.y_offset = number("YOffset")?,
            "XYUnits" => field.xy_units = value.into(),
            "ZUnits" => field.z_units = value.into(),
            "Title" => field.title = Some(value.into()),
            _ => {}
        }
    }

    let nx = nx.ok_or_else(|| invalid("header has no XRes"))?;
    let ny = ny.ok_or_else(|| invalid("header has no YRes"))?;

    // Data starts at the next multiple of 4 at or after the end of the header.
    let start = nul.div_ceil(4) * 4;
    let need = nx * ny * 4;
    if bytes.len() < start + need {
        return Err(invalid(&format!(
            "truncated: need {need} data bytes from offset {start}, file has {}",
            bytes.len().saturating_sub(start)
        )));
    }

    let values: Vec<f64> = bytes[start..start + need]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c) as f64)
        .collect();
    field.data = Array2::from_shape_vec((ny, nx), values)
        .map_err(|e| invalid(&format!("bad shape: {e}")))?;
    Ok(field)
}

/// Convenience: write a map in metres straight to a `.gsf` file.
///
/// Takes a view and never owns the data: at scan sizes the copy
/// [`GsfField::from_grid`] would need is several megabytes, for something the
/// writer only iterates.
pub fn write_map(
    path: impl AsRef<Path>,
    data: ArrayView2<f64>,
    sp: GridSpacing,
    title: &str,
) -> io::Result<()> {
    let (ny, nx) = data.dim();
    write_view(
        path,
        data,
        &Header {
            x_real: nx as f64 * sp.dx,
            y_real: ny as f64 * sp.dy,
            x_offset: 0.0,
            y_offset: 0.0,
            xy_units: "m",
            z_units: "m",
            title: Some(title),
        },
    )
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("GSF: {msg}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn sample_field() -> GsfField {
        let data = Array2::from_shape_fn((5, 7), |(i, j)| (i * 7 + j) as f64 * 1e-11);
        GsfField::from_grid(data, GridSpacing::square(1e-10), "Z tip")
    }

    #[test]
    fn round_trips_through_the_filesystem() {
        let path = crate::utils::temp_path("gsf-test-roundtrip", "gsf");
        let original = sample_field();
        write_gsf(&path, &original).unwrap();
        let back = read_gsf(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(back.data.dim(), original.data.dim());
        assert_eq!(back.xy_units, "m");
        assert_eq!(back.z_units, "m");
        assert_eq!(back.title.as_deref(), Some("Z tip"));
        assert!((back.x_real - original.x_real).abs() < 1e-20);
        assert!((back.y_real - original.y_real).abs() < 1e-20);
        for (got, want) in back.data.iter().zip(original.data.iter()) {
            // f32 payload: relative agreement is all that is on offer.
            assert!((got - want).abs() <= want.abs() * 1e-6 + 1e-18);
        }
        // The spacing survives the trip, which is the whole reason for GSF.
        assert!((back.spacing().dx - 1e-10).abs() < 1e-16);
    }

    #[test]
    fn layout_matches_the_specification() {
        let path = crate::utils::temp_path("gsf-test-layout", "gsf");
        write_gsf(&path, &sample_field()).unwrap();
        let mut bytes = Vec::new();
        File::open(&path).unwrap().read_to_end(&mut bytes).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(bytes.starts_with(MAGIC.as_bytes()));
        assert_eq!(bytes[MAGIC.len()], b'\n');

        // The payload is a fixed size, so work back from the end. Scanning
        // forward for the first non-NUL would run into the payload itself: a
        // sample of exactly 0.0 is four more zero bytes.
        let data_start = bytes.len() - 5 * 7 * 4;
        assert_eq!(data_start % 4, 0, "data must start on a 4-byte boundary");

        let first_nul = bytes.iter().position(|b| *b == 0).unwrap();
        let pad = data_start - first_nul;
        assert!(
            (1..=4).contains(&pad),
            "padding must be 1 to 4 NULs, got {pad}"
        );
        assert!(bytes[first_nul..data_start].iter().all(|b| *b == 0));
    }

    #[test]
    fn rows_are_stored_top_to_bottom() {
        // GSF stores rows from top to bottom, left to right, which is the same
        // order ndarray iterates a row-major array. If that ever inverts, an
        // exported map shows up mirrored in Gwyddion and every conclusion drawn
        // from it is upside down.
        let path = crate::utils::temp_path("gsf-test-order", "gsf");
        let data = Array2::from_shape_vec((2, 2), vec![1.0, 2.0, 3.0, 4.0]).unwrap();
        write_gsf(
            &path,
            &GsfField::from_grid(data, GridSpacing::square(1.0), "t"),
        )
        .unwrap();
        let mut bytes = Vec::new();
        File::open(&path).unwrap().read_to_end(&mut bytes).unwrap();
        std::fs::remove_file(&path).ok();

        let tail = &bytes[bytes.len() - 16..];
        let vals: Vec<f32> = tail
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        assert_eq!(vals, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn rejects_a_file_that_is_not_gsf() {
        let path = crate::utils::temp_path("gsf-test-bogus", "gsf");
        std::fs::write(&path, b"not a gsf\0\0\0\0").unwrap();
        let err = read_gsf(&path).unwrap_err();
        std::fs::remove_file(&path).ok();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
