//! ASCII point clouds and column tables.
//!
//! Where [`super::gsf`] is for *looking* at one map, this is for *comparing*
//! several. A GSF file holds a single channel; a column table holds the
//! surface, the planned trajectory and their difference side by side on one
//! line per sample, which is what makes "is this mapping reasonable?" a
//! question you can answer with a plot or a spreadsheet.
//!
//! # Coordinates
//!
//! Sample `(i, j)` is written at the centre of the area it covers:
//!
//! ```text
//! x = (j + 0.5) * dx        column index, fast scan axis
//! y = (i + 0.5) * dy        row index, slow scan axis
//! ```
//!
//! The half-sample offset is not cosmetic: it is the same convention GSF uses,
//! so a point cloud exported here lands exactly on top of the matching `.gsf`
//! map when both are loaded into Gwyddion. `y` increases with the row index,
//! i.e. downward through the image, matching GSF's top-to-bottom row order.
//!
//! All values are in metres, in scientific notation, with enough digits to
//! survive a round trip through `f64`.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use ndarray::ArrayView2;

use super::super::analyzer::rolling_ellipsoid::GridSpacing;

/// Write a single map as a three-column `x y z` point cloud.
///
/// Deliberately free of comments and headers, because Gwyddion's XYZ import and
/// several other readers want nothing but numbers. Use [`write_table`] when the
/// file is for your own analysis and a header helps.
pub fn write_xyz(path: impl AsRef<Path>, data: ArrayView2<f64>, sp: GridSpacing) -> io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    for ((i, j), value) in data.indexed_iter() {
        let x = (j as f64 + 0.5) * sp.dx;
        let y = (i as f64 + 0.5) * sp.dy;
        writeln!(out, "{x:.9e} {y:.9e} {value:.9e}")?;
    }
    out.flush()
}

/// Write several co-registered maps as one whitespace-separated table.
///
/// Columns are `x`, `y`, then one column per named map, in the order given. The
/// first line is a `#` comment naming the columns, which gnuplot, numpy
/// (`loadtxt` skips `#` by default) and pandas all understand.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] if `maps` is empty or the maps do
/// not all have the same shape, since a table of mismatched maps would silently
/// align the wrong samples.
pub fn write_table(
    path: impl AsRef<Path>,
    sp: GridSpacing,
    maps: &[(&str, ArrayView2<f64>)],
) -> io::Result<()> {
    let Some((_, first)) = maps.first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "write_table needs at least one map",
        ));
    };
    let (ny, nx) = first.dim();
    if maps.iter().any(|(_, m)| m.dim() != (ny, nx)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "all maps in a table must have the same shape",
        ));
    }

    let mut out = BufWriter::new(File::create(path)?);
    write!(out, "# x_m y_m")?;
    for (name, _) in maps {
        write!(out, " {name}")?;
    }
    writeln!(out)?;

    // One formatted line at a time rather than one call per column: at scan
    // sizes the per-value `write!` runs into the millions, and `y` does not
    // change across a row.
    let mut line = String::new();
    for i in 0..ny {
        let y = (i as f64 + 0.5) * sp.dy;
        for j in 0..nx {
            let x = (j as f64 + 0.5) * sp.dx;
            line.clear();
            let _ = write!(line, "{x:.9e} {y:.9e}");
            for (_, m) in maps {
                let _ = write!(line, " {:.9e}", m[(i, j)]);
            }
            writeln!(out, "{line}")?;
        }
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    fn temp(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "rusty-tip-xyz-test-{name}-{}.dat",
            std::process::id()
        ));
        p
    }

    #[test]
    fn xyz_places_samples_at_area_centres() {
        let path = temp("centres");
        let z = array![[1.0, 2.0], [3.0, 4.0]];
        let sp = GridSpacing { dx: 2.0, dy: 10.0 };
        write_xyz(&path, z.view(), sp).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let rows: Vec<Vec<f64>> = text
            .lines()
            .map(|l| l.split_whitespace().map(|t| t.parse().unwrap()).collect())
            .collect();
        assert_eq!(rows.len(), 4);
        // (0,0) -> half a sample in from the origin on both axes.
        assert_eq!(rows[0], vec![1.0, 5.0, 1.0]);
        // Fast axis varies first, matching row-major order.
        assert_eq!(rows[1], vec![3.0, 5.0, 2.0]);
        // Second row is one full dy further along the slow axis.
        assert_eq!(rows[2], vec![1.0, 15.0, 3.0]);
    }

    #[test]
    fn table_keeps_maps_aligned_column_by_column() {
        let path = temp("table");
        let surface = array![[0.0, 1.0]];
        let tip = array![[0.5, 1.5]];
        write_table(
            &path,
            GridSpacing::square(1.0),
            &[("z_surface", surface.view()), ("z_tip", tip.view())],
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let mut lines = text.lines();
        assert_eq!(lines.next().unwrap(), "# x_m y_m z_surface z_tip");
        let first: Vec<f64> = lines
            .next()
            .unwrap()
            .split_whitespace()
            .map(|t| t.parse().unwrap())
            .collect();
        assert_eq!(first, vec![0.5, 0.5, 0.0, 0.5]);
    }

    #[test]
    fn table_rejects_mismatched_maps() {
        let path = temp("mismatch");
        let a = array![[0.0, 1.0]];
        let b = array![[0.0]];
        let err = write_table(
            &path,
            GridSpacing::square(1.0),
            &[("a", a.view()), ("b", b.view())],
        )
        .unwrap_err();
        std::fs::remove_file(&path).ok();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
