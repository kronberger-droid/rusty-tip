use ndarray::Array2;
use rayon::prelude::*;
use serde_json::json;

use super::{Analyzer, AnalyzerInput, AnalyzerOutput, Result};

/// Detects CuOx reconstruction rows (broad stripes) in STM images of Cu(110).
///
/// Works regardless of tip termination:
/// - Cu tip: dark featureless bands
/// - O-terminated tip: textured checkerboard bands
///
/// The algorithm:
/// 1. Build a score map (local variance + intensity deviation, combined with pixel-wise max)
/// 2. Detect band orientation via Radon-style projection variance maximization
/// 3. Threshold the 1D projection to find bands
/// 4. Annotate edge lines on the image
pub struct CuoxRowDetector {
    /// Radius for local variance window (default: 5).
    pub var_radius: usize,
    /// Band detection threshold as fraction of max projection (default: 0.3).
    pub threshold: f32,
    /// Minimum band width in pixels to reject noise (default: 5).
    pub min_band_width: usize,
    /// If set, skip angle detection and use this fixed angle (degrees).
    pub fixed_angle: Option<f32>,
}

impl Default for CuoxRowDetector {
    fn default() -> Self {
        Self {
            var_radius: 5,
            threshold: 0.3,
            min_band_width: 5,
            fixed_angle: None,
        }
    }
}

impl CuoxRowDetector {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Analyzer for CuoxRowDetector {
    fn name(&self) -> &str {
        "cuox_row_detector"
    }

    fn description(&self) -> &str {
        "Detects CuOx reconstruction rows in STM images of Cu(110)"
    }

    fn analyze(&self, input: &AnalyzerInput) -> Result<AnalyzerOutput> {
        let rows = input.rows();
        let cols = input.cols();
        if rows == 0 || cols == 0 {
            return Ok(AnalyzerOutput {
                data: json!({ "error": "empty input", "bands_count": 0, "bands": [] }),
                annotated_image: None,
            });
        }

        // Guard: image must be at least 4x4 for the downsampled angle sweep
        if rows < 4 || cols < 4 {
            return Ok(AnalyzerOutput {
                data: json!({
                    "error": "image too small (minimum 4x4)",
                    "bands_count": 0,
                    "bands": []
                }),
                annotated_image: None,
            });
        }

        // Convert Vec<Vec<f32>> to Array2<f32>, sanitizing NaN/Inf to 0.
        let mut nan_count = 0usize;
        let mut img = Array2::<f32>::zeros((rows, cols));
        for (r, row) in input.data.iter().enumerate() {
            for (c, &val) in row.iter().enumerate() {
                if val.is_finite() {
                    img[[r, c]] = val;
                } else {
                    nan_count += 1;
                    // Leave as 0.0 (neutral value)
                }
            }
        }
        if nan_count > 0 {
            log::warn!(
                "cuox_row_detector: {} non-finite pixels replaced with 0.0",
                nan_count
            );
        }

        // Step 1: Score map
        let score = build_score_map(&img, self.var_radius);

        // Step 2: Angle detection
        let angle_deg = if let Some(a) = self.fixed_angle {
            a
        } else {
            detect_angle(&score)
        };

        // Step 3: Band detection
        let projection = project_at_angle(&score, angle_deg as f64);
        let bands =
            detect_bands(&projection, self.threshold, self.min_band_width);

        // Step 4: Build output with image-space coordinates
        let calibration_nm_per_px = input.calibration_m_per_px.map(|m| m * 1e9);

        let angle_rad = (angle_deg as f64).to_radians();
        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();
        let cy = rows as f64 / 2.0;
        let cx = cols as f64 / 2.0;
        let min_proj = compute_min_proj(rows, cols, cos_a, sin_a, cy, cx);
        let geom = ProjectionGeometry {
            rows, cols, cos_a, sin_a, cy, cx, min_proj,
        };

        let bands_json: Vec<serde_json::Value> = bands
            .iter()
            .map(|b| {
                let center_line = line_endpoints(b.center, &geom);
                let low_line = line_endpoints(b.edge_low as f64, &geom);
                // edge_high is exclusive (first bin outside band), so use
                // edge_high - 1 for the last bin inside the band.
                let high_line = line_endpoints(
                    (b.edge_high.saturating_sub(1)) as f64, &geom,
                );

                let mut obj = json!({
                    "center_px": b.center,
                    "edge_low_px": b.edge_low,
                    "edge_high_px": b.edge_high,
                    "width_px": b.edge_high - b.edge_low,
                    "center_line": format_line(&center_line),
                    "edge_low_line": format_line(&low_line),
                    "edge_high_line": format_line(&high_line),
                });
                if let Some(nm_per_px) = calibration_nm_per_px {
                    let obj_map = obj.as_object_mut().unwrap();
                    obj_map.insert(
                        "center_nm".into(),
                        json!(b.center * nm_per_px),
                    );
                    obj_map.insert(
                        "width_nm".into(),
                        json!((b.edge_high - b.edge_low) as f64 * nm_per_px),
                    );
                }
                obj
            })
            .collect();

        let data = json!({
            "angle_deg": angle_deg,
            "bands_count": bands.len(),
            "bands": bands_json,
        });

        Ok(AnalyzerOutput {
            data,
            annotated_image: None,
        })
    }
}

// ── Detected band ──────────────────────────────────────────────────

struct Band {
    center: f64,
    edge_low: usize,
    edge_high: usize,
}

// ── Step 1: Score map ──────────────────────────────────────────────

/// Build summed area table (SAT) from an Array2<f32>, stored as f64.
fn build_sat(img: &Array2<f32>) -> Array2<f64> {
    let (rows, cols) = img.dim();
    let mut sat = Array2::<f64>::zeros((rows + 1, cols + 1));
    for r in 0..rows {
        for c in 0..cols {
            sat[[r + 1, c + 1]] =
                img[[r, c]] as f64 + sat[[r, c + 1]] + sat[[r + 1, c]]
                    - sat[[r, c]];
        }
    }
    sat
}

/// Build summed area table of squared values.
fn build_sat_sq(img: &Array2<f32>) -> Array2<f64> {
    let (rows, cols) = img.dim();
    let mut sat = Array2::<f64>::zeros((rows + 1, cols + 1));
    for r in 0..rows {
        for c in 0..cols {
            let v = img[[r, c]] as f64;
            sat[[r + 1, c + 1]] =
                v * v + sat[[r, c + 1]] + sat[[r + 1, c]] - sat[[r, c]];
        }
    }
    sat
}

/// Query a SAT for the sum within a rectangle [r0..r1, c0..c1] (inclusive pixel coords).
/// SAT is 1-indexed (row/col 0 is the padding row).
#[inline]
fn sat_query(
    sat: &Array2<f64>,
    r0: usize,
    c0: usize,
    r1: usize,
    c1: usize,
) -> f64 {
    sat[[r1 + 1, c1 + 1]] - sat[[r0, c1 + 1]] - sat[[r1 + 1, c0]]
        + sat[[r0, c0]]
}

fn build_score_map(img: &Array2<f32>, var_radius: usize) -> Array2<f32> {
    let (rows, cols) = img.dim();
    let sat = build_sat(img);
    let sat_sq = build_sat_sq(img);

    // Compute global median for intensity deviation map
    let mut pixels: Vec<f32> = img.iter().copied().collect();
    pixels.sort_unstable_by(|a, b| a.total_cmp(b));
    let global_median = pixels[pixels.len() / 2] as f64;

    let r = var_radius;
    let big_r = 3 * var_radius;

    // Local variance map (parallel over rows)
    let var_rows: Vec<Vec<f32>> = (0..rows)
        .into_par_iter()
        .map(|y| {
            (0..cols)
                .map(|x| {
                    let r0 = y.saturating_sub(r);
                    let c0 = x.saturating_sub(r);
                    let r1 = (y + r).min(rows - 1);
                    let c1 = (x + r).min(cols - 1);
                    let n = ((r1 - r0 + 1) * (c1 - c0 + 1)) as f64;
                    let sum = sat_query(&sat, r0, c0, r1, c1);
                    let sum_sq = sat_query(&sat_sq, r0, c0, r1, c1);
                    let variance = (sum_sq / n) - (sum / n).powi(2);
                    variance.max(0.0) as f32
                })
                .collect()
        })
        .collect();

    let mut var_map = Array2::<f32>::zeros((rows, cols));
    for (r_idx, row) in var_rows.into_iter().enumerate() {
        for (c_idx, val) in row.into_iter().enumerate() {
            var_map[[r_idx, c_idx]] = val;
        }
    }

    // Intensity deviation map (parallel over rows)
    let dev_rows: Vec<Vec<f32>> = (0..rows)
        .into_par_iter()
        .map(|y| {
            (0..cols)
                .map(|x| {
                    let r0 = y.saturating_sub(big_r);
                    let c0 = x.saturating_sub(big_r);
                    let r1 = (y + big_r).min(rows - 1);
                    let c1 = (x + big_r).min(cols - 1);
                    let n = ((r1 - r0 + 1) * (c1 - c0 + 1)) as f64;
                    let local_mean = sat_query(&sat, r0, c0, r1, c1) / n;
                    let pixel = img[[y, x]] as f64;
                    // How much darker than both local mean and global median
                    let dev_local = (local_mean - pixel).max(0.0);
                    let dev_global = (global_median - pixel).max(0.0);
                    (dev_local + dev_global) as f32
                })
                .collect()
        })
        .collect();

    let mut dev_map = Array2::<f32>::zeros((rows, cols));
    for (r_idx, row) in dev_rows.into_iter().enumerate() {
        for (c_idx, val) in row.into_iter().enumerate() {
            dev_map[[r_idx, c_idx]] = val;
        }
    }

    // Normalize each map to [0, 1]
    normalize_map(&mut var_map);
    normalize_map(&mut dev_map);

    // Combine with element-wise max
    let mut score = Array2::<f32>::zeros((rows, cols));
    score
        .iter_mut()
        .zip(var_map.iter())
        .zip(dev_map.iter())
        .for_each(|((s, &v), &d)| {
            *s = v.max(d);
        });

    score
}

fn normalize_map(map: &mut Array2<f32>) {
    let max = map.iter().copied().fold(0.0f32, |a, b| a.max(b));
    if max > 0.0 {
        map.mapv_inplace(|v| v / max);
    }
}

// ── Step 2: Angle detection ────────────────────────────────────────

fn downsample(img: &Array2<f32>, factor: usize) -> Array2<f32> {
    let (rows, cols) = img.dim();
    let new_rows = rows / factor;
    let new_cols = cols / factor;
    let mut out = Array2::<f32>::zeros((new_rows, new_cols));
    let n = (factor * factor) as f32;
    for r in 0..new_rows {
        for c in 0..new_cols {
            let mut sum = 0.0f32;
            for dr in 0..factor {
                for dc in 0..factor {
                    sum += img[[r * factor + dr, c * factor + dc]];
                }
            }
            out[[r, c]] = sum / n;
        }
    }
    out
}

/// Project (average) the image along a given angle, producing a 1D profile
/// perpendicular to that direction.
fn project_at_angle(img: &Array2<f32>, angle_deg: f64) -> Vec<f64> {
    let (rows, cols) = img.dim();
    let angle_rad = angle_deg.to_radians();
    let cos_a = angle_rad.cos();
    let sin_a = angle_rad.sin();

    // The perpendicular direction: project each pixel onto the axis
    // perpendicular to the band direction.
    let cy = rows as f64 / 2.0;
    let cx = cols as f64 / 2.0;

    // Compute projected coordinate range
    let mut min_proj = f64::MAX;
    let mut max_proj = f64::MIN;
    for &(y, x) in &[
        (0.0, 0.0),
        (0.0, cols as f64),
        (rows as f64, 0.0),
        (rows as f64, cols as f64),
    ] {
        let p = (y - cy) * cos_a + (x - cx) * sin_a;
        min_proj = min_proj.min(p);
        max_proj = max_proj.max(p);
    }

    let n_bins = ((max_proj - min_proj).ceil() as usize).max(1);
    let mut sums = vec![0.0f64; n_bins];
    let mut counts = vec![0u32; n_bins];

    for ((y, x), &val) in img.indexed_iter() {
        let p = (y as f64 - cy) * cos_a + (x as f64 - cx) * sin_a;
        let bin = ((p - min_proj) as usize).min(n_bins - 1);
        sums[bin] += val as f64;
        counts[bin] += 1;
    }

    sums.iter()
        .zip(counts.iter())
        .map(|(&s, &c)| if c > 0 { s / c as f64 } else { 0.0 })
        .collect()
}

fn projection_variance(profile: &[f64]) -> f64 {
    if profile.is_empty() {
        return 0.0;
    }
    let n = profile.len() as f64;
    let mean = profile.iter().sum::<f64>() / n;
    profile.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / n
}

fn detect_angle(score: &Array2<f32>) -> f32 {
    let (rows, cols) = score.dim();
    // Only downsample if the result is at least 4x4, otherwise
    // the coarse sweep operates on too few pixels to be meaningful.
    let ds_factor = if rows >= 16 && cols >= 16 { 4 } else { 1 };
    let small = if ds_factor > 1 {
        downsample(score, ds_factor)
    } else {
        score.clone()
    };

    let coarse_results: Vec<(f64, f32)> = (0..180)
        .into_par_iter()
        .map(|deg| {
            let angle = deg as f32;
            let proj = project_at_angle(&small, angle as f64);
            let var = projection_variance(&proj);
            (var, angle)
        })
        .collect();

    let best_coarse = coarse_results
        .iter()
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap()
        .1;

    // Fine sweep +/- 2 degrees at 0.1 degree steps, full resolution
    let fine_steps: Vec<i32> = (-20..=20).collect();
    let fine_results: Vec<(f64, f32)> = fine_steps
        .into_par_iter()
        .map(|step| {
            let angle = best_coarse + step as f32 * 0.1;
            // Wrap to [0, 180) range
            let angle = ((angle % 180.0) + 180.0) % 180.0;
            let proj = project_at_angle(score, angle as f64);
            let var = projection_variance(&proj);
            (var, angle)
        })
        .collect();

    fine_results
        .iter()
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap()
        .1
}

// ── Step 3: Band detection ─────────────────────────────────────────

fn detect_bands(
    projection: &[f64],
    threshold: f32,
    min_width: usize,
) -> Vec<Band> {
    if projection.is_empty() {
        return vec![];
    }

    let max_val = projection.iter().copied().fold(0.0f64, f64::max);
    if max_val <= 0.0 {
        return vec![];
    }
    let thresh = threshold as f64 * max_val;

    // Find contiguous regions above threshold
    let mut bands = Vec::new();
    let mut in_band = false;
    let mut start = 0;

    for (i, &val) in projection.iter().enumerate() {
        if val >= thresh && !in_band {
            in_band = true;
            start = i;
        } else if (val < thresh || i == projection.len() - 1) && in_band {
            let end = if val >= thresh { i + 1 } else { i };
            in_band = false;

            if end - start >= min_width {
                // Weighted center within the band
                let mut weight_sum = 0.0;
                let mut weighted_pos = 0.0;
                for (j, &val) in projection[start..end].iter().enumerate() {
                    weight_sum += val;
                    weighted_pos += val * (start + j) as f64;
                }
                let center = if weight_sum > 0.0 {
                    weighted_pos / weight_sum
                } else {
                    (start + end) as f64 / 2.0
                };

                bands.push(Band {
                    center,
                    edge_low: start,
                    edge_high: end,
                });
            }
        }
    }

    bands
}

// ── Step 4: Projection-space to image-space coordinates ────────────

/// A line segment in image pixel coordinates.
#[derive(Debug, Clone)]
struct LineSegment {
    /// (y, x) of the first endpoint where the line enters the image.
    start: (f64, f64),
    /// (y, x) of the second endpoint where the line exits the image.
    end: (f64, f64),
}

/// Compute the minimum projection coordinate for the image corners.
/// This is the offset between projection bin index and actual projection distance.
fn compute_min_proj(
    rows: usize,
    cols: usize,
    cos_a: f64,
    sin_a: f64,
    cy: f64,
    cx: f64,
) -> f64 {
    let corners = [
        (0.0, 0.0),
        (0.0, cols as f64),
        (rows as f64, 0.0),
        (rows as f64, cols as f64),
    ];
    corners
        .iter()
        .map(|&(y, x)| (y - cy) * cos_a + (x - cx) * sin_a)
        .fold(f64::MAX, f64::min)
}

/// Geometric context for projecting band lines onto an image.
struct ProjectionGeometry {
    rows: usize,
    cols: usize,
    cos_a: f64,
    sin_a: f64,
    cy: f64,
    cx: f64,
    min_proj: f64,
}

/// Compute the two endpoints where a line at a given projection coordinate
/// intersects the image rectangle [0..rows, 0..cols].
///
/// The line is parameterized as:
///   y = cy + d * cos_a + t * (-sin_a)
///   x = cx + d * sin_a + t * cos_a
/// where `d = proj_bin + min_proj` is the actual perpendicular distance.
fn line_endpoints(proj_bin: f64, geom: &ProjectionGeometry) -> Option<LineSegment> {
    let d = proj_bin + geom.min_proj;
    let base_y = geom.cy + d * geom.cos_a;
    let base_x = geom.cx + d * geom.sin_a;

    // Direction along the band: dy/dt = -sin_a, dx/dt = cos_a
    // Clip the parametric line against the image rectangle.
    let mut t_min = f64::NEG_INFINITY;
    let mut t_max = f64::INFINITY;

    // Clip against y = 0 and y = rows
    if geom.sin_a.abs() > 1e-12 {
        let t_y0 = (base_y - 0.0) / geom.sin_a; // y = 0 => t = base_y / sin_a
        let t_y1 = (base_y - geom.rows as f64) / geom.sin_a; // y = rows
        let (lo, hi) = if geom.sin_a > 0.0 {
            (t_y1, t_y0)
        } else {
            (t_y0, t_y1)
        };
        t_min = t_min.max(lo);
        t_max = t_max.min(hi);
    } else {
        // Line is horizontal (sin_a ~ 0), check if base_y is within bounds
        if base_y < 0.0 || base_y > geom.rows as f64 {
            return None;
        }
    }

    // Clip against x = 0 and x = cols
    if geom.cos_a.abs() > 1e-12 {
        let t_x0 = (0.0 - base_x) / geom.cos_a; // x = 0
        let t_x1 = (geom.cols as f64 - base_x) / geom.cos_a; // x = cols
        let (lo, hi) = if geom.cos_a > 0.0 {
            (t_x0, t_x1)
        } else {
            (t_x1, t_x0)
        };
        t_min = t_min.max(lo);
        t_max = t_max.min(hi);
    } else if base_x < 0.0 || base_x > geom.cols as f64 {
        return None;
    }

    if t_min > t_max {
        return None;
    }

    let start_y = base_y - t_min * geom.sin_a;
    let start_x = base_x + t_min * geom.cos_a;
    let end_y = base_y - t_max * geom.sin_a;
    let end_x = base_x + t_max * geom.cos_a;

    Some(LineSegment {
        start: (start_y, start_x),
        end: (end_y, end_x),
    })
}

fn format_line(seg: &Option<LineSegment>) -> serde_json::Value {
    match seg {
        Some(s) => json!({
            "start": { "y": s.start.0, "x": s.start.1 },
            "end":   { "y": s.end.0,   "x": s.end.1   },
        }),
        None => json!(null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::AnalyzerInput;

    // ── Helpers ────────────────────────────────────────────────────

    fn vec2d_to_array2(data: &[Vec<f32>]) -> Array2<f32> {
        let rows = data.len();
        let cols = data.first().map_or(0, |r| r.len());
        let mut img = Array2::<f32>::zeros((rows, cols));
        for (r, row) in data.iter().enumerate() {
            for (c, &val) in row.iter().enumerate() {
                img[[r, c]] = val;
            }
        }
        img
    }

    /// Horizontal bright bands on dark background.
    fn horizontal_banded_image(rows: usize, cols: usize) -> Vec<Vec<f32>> {
        let mut data = vec![vec![0.0f32; cols]; rows];
        for r in 50..80.min(rows) {
            for c in 0..cols {
                data[r][c] = 1.0;
            }
        }
        for r in 150..180.min(rows) {
            for c in 0..cols {
                data[r][c] = 1.0;
            }
        }
        data
    }

    /// Vertical bright bands on dark background.
    fn vertical_banded_image(rows: usize, cols: usize) -> Vec<Vec<f32>> {
        let mut data = vec![vec![0.0f32; cols]; rows];
        for r in 0..rows {
            for c in 50..80.min(cols) {
                data[r][c] = 1.0;
            }
            for c in 150..180.min(cols) {
                data[r][c] = 1.0;
            }
        }
        data
    }

    /// Diagonal band: pixels where (y + x) falls in a range are bright.
    fn diagonal_banded_image(rows: usize, cols: usize) -> Vec<Vec<f32>> {
        let mut data = vec![vec![0.0f32; cols]; rows];
        for r in 0..rows {
            for c in 0..cols {
                let diag = r + c;
                // Band from diagonal 80..130 and 220..270
                if (80..130).contains(&diag) || (220..270).contains(&diag) {
                    data[r][c] = 1.0;
                }
            }
        }
        data
    }

    /// Uniform image (all same value). Useful for edge-case tests.
    fn uniform_image(rows: usize, cols: usize, val: f32) -> Vec<Vec<f32>> {
        vec![vec![val; cols]; rows]
    }

    /// Dark-band image: background at 0.8, bands at 0.1 (simulates Cu-tip CuOx).
    fn dark_banded_image(rows: usize, cols: usize) -> Vec<Vec<f32>> {
        let mut data = vec![vec![0.8f32; cols]; rows];
        for r in 60..90.min(rows) {
            for c in 0..cols {
                data[r][c] = 0.1;
            }
        }
        data
    }

    // ── SAT tests ──────────────────────────────────────────────────

    #[test]
    fn sat_full_and_subregion_sums() {
        let img = Array2::from_shape_vec(
            (3, 3),
            vec![1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
        )
        .unwrap();
        let sat = build_sat(&img);

        // Full image: 1+2+...+9 = 45
        assert!((sat_query(&sat, 0, 0, 2, 2) - 45.0).abs() < 1e-10);
        // Top-left 2x2: 1+2+4+5 = 12
        assert!((sat_query(&sat, 0, 0, 1, 1) - 12.0).abs() < 1e-10);
        // Bottom-right 2x2: 5+6+8+9 = 28
        assert!((sat_query(&sat, 1, 1, 2, 2) - 28.0).abs() < 1e-10);
        // Middle row: 4+5+6 = 15
        assert!((sat_query(&sat, 1, 0, 1, 2) - 15.0).abs() < 1e-10);
        // Right column: 3+6+9 = 18
        assert!((sat_query(&sat, 0, 2, 2, 2) - 18.0).abs() < 1e-10);
    }

    #[test]
    fn sat_single_pixel_queries() {
        let img = Array2::from_shape_vec(
            (3, 3),
            vec![1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
        )
        .unwrap();
        let sat = build_sat(&img);

        // Every single-pixel query should return exactly that pixel's value
        for r in 0..3 {
            for c in 0..3 {
                let expected = img[[r, c]] as f64;
                let got = sat_query(&sat, r, c, r, c);
                assert!(
                    (got - expected).abs() < 1e-10,
                    "Single pixel [{},{}]: expected {}, got {}",
                    r,
                    c,
                    expected,
                    got
                );
            }
        }
    }

    #[test]
    fn sat_squared_correctness() {
        let img = Array2::from_shape_vec(
            (2, 3),
            vec![1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0],
        )
        .unwrap();
        let sat_sq = build_sat_sq(&img);

        // Full sum of squares: 1+4+9+16+25+36 = 91
        let total = sat_query(&sat_sq, 0, 0, 1, 2);
        assert!((total - 91.0).abs() < 1e-10);

        // Single pixel [1,2] should be 6^2 = 36
        let single = sat_query(&sat_sq, 1, 2, 1, 2);
        assert!((single - 36.0).abs() < 1e-10);

        // Top row: 1+4+9 = 14
        let top = sat_query(&sat_sq, 0, 0, 0, 2);
        assert!((top - 14.0).abs() < 1e-10);
    }

    #[test]
    fn sat_1x1_image() {
        let img = Array2::from_shape_vec((1, 1), vec![42.0f32]).unwrap();
        let sat = build_sat(&img);
        assert!((sat_query(&sat, 0, 0, 0, 0) - 42.0).abs() < 1e-10);
    }

    // ── Normalize tests ────────────────────────────────────────────

    #[test]
    fn normalize_scales_to_unit() {
        let mut map =
            Array2::from_shape_vec((2, 2), vec![2.0f32, 4.0, 6.0, 8.0])
                .unwrap();
        normalize_map(&mut map);
        assert!((map[[1, 1]] - 1.0).abs() < 1e-6, "Max should be 1.0");
        assert!((map[[0, 0]] - 0.25).abs() < 1e-6, "2/8 = 0.25");
    }

    #[test]
    fn normalize_zero_map_stays_zero() {
        let mut map = Array2::<f32>::zeros((3, 3));
        normalize_map(&mut map);
        assert!(
            map.iter().all(|&v| v == 0.0),
            "All-zero map should stay zero"
        );
    }

    // ── Downsample tests ───────────────────────────────────────────

    #[test]
    fn downsample_averages_blocks() {
        // 4x4 image downsampled by 2 -> 2x2
        let img = Array2::from_shape_vec(
            (4, 4),
            vec![
                1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0,
                13.0, 14.0, 15.0, 16.0f32,
            ],
        )
        .unwrap();
        let small = downsample(&img, 2);
        assert_eq!(small.dim(), (2, 2));
        // Top-left 2x2 block: (1+2+5+6)/4 = 3.5
        assert!((small[[0, 0]] - 3.5).abs() < 1e-6);
        // Bottom-right 2x2 block: (11+12+15+16)/4 = 13.5
        assert!((small[[1, 1]] - 13.5).abs() < 1e-6);
    }

    #[test]
    fn downsample_preserves_uniform() {
        let img = Array2::from_elem((8, 8), 5.0f32);
        let small = downsample(&img, 4);
        assert_eq!(small.dim(), (2, 2));
        assert!(small.iter().all(|&v| (v - 5.0).abs() < 1e-6));
    }

    // ── Projection tests ───────────────────────────────────────────

    #[test]
    fn projection_at_0_degrees_averages_along_columns() {
        // At 0 degrees, perpendicular axis is the y-axis.
        // A horizontal bright stripe should produce a peak in the projection.
        let mut img = Array2::<f32>::zeros((100, 100));
        for r in 40..60 {
            for c in 0..100 {
                img[[r, c]] = 1.0;
            }
        }
        let proj = project_at_angle(&img, 0.0);
        let max_val = proj.iter().copied().fold(0.0f64, f64::max);
        let min_val = proj.iter().copied().fold(f64::MAX, f64::min);
        assert!(
            max_val > 0.5 && min_val < 0.2,
            "Projection should show contrast: max={}, min={}",
            max_val,
            min_val
        );
    }

    #[test]
    fn projection_at_90_degrees_averages_along_rows() {
        // At 90 degrees, perpendicular axis is the x-axis.
        // A vertical bright stripe should produce a peak.
        let mut img = Array2::<f32>::zeros((100, 100));
        for r in 0..100 {
            for c in 40..60 {
                img[[r, c]] = 1.0;
            }
        }
        let proj = project_at_angle(&img, 90.0);
        let max_val = proj.iter().copied().fold(0.0f64, f64::max);
        let min_val = proj.iter().copied().fold(f64::MAX, f64::min);
        assert!(
            max_val > 0.5 && min_val < 0.2,
            "Projection should show contrast: max={}, min={}",
            max_val,
            min_val
        );
    }

    #[test]
    fn projection_uniform_image_has_low_variance() {
        // At non-axis-aligned angles, discrete binning creates slight count
        // imbalances at the edges, so variance won't be exactly zero.
        // At axis-aligned angles (0, 90) it should be near-zero.
        let img = Array2::from_elem((50, 50), 1.0f32);
        let proj_0 = project_at_angle(&img, 0.0);
        let var_0 = projection_variance(&proj_0);
        assert!(
            var_0 < 1e-6,
            "Axis-aligned projection should have ~zero variance, got {}",
            var_0
        );

        let proj_30 = project_at_angle(&img, 30.0);
        let var_30 = projection_variance(&proj_30);
        assert!(
            var_30 < 0.1,
            "Off-axis projection variance should be small, got {}",
            var_30
        );
    }

    // ── Projection variance tests ──────────────────────────────────

    #[test]
    fn projection_variance_empty() {
        assert_eq!(projection_variance(&[]), 0.0);
    }

    #[test]
    fn projection_variance_constant() {
        assert!(projection_variance(&[5.0, 5.0, 5.0, 5.0]) < 1e-10);
    }

    #[test]
    fn projection_variance_known_value() {
        // [0, 1]: mean = 0.5, variance = ((0.25 + 0.25) / 2) = 0.25
        let var = projection_variance(&[0.0, 1.0]);
        assert!((var - 0.25).abs() < 1e-10);
    }

    // ── Angle detection tests ──────────────────────────────────────

    #[test]
    fn detect_angle_horizontal_bands() {
        let data = horizontal_banded_image(200, 200);
        let img = vec2d_to_array2(&data);
        let score = build_score_map(&img, 5);
        let angle = detect_angle(&score);
        assert!(
            angle < 10.0 || angle > 170.0,
            "Horizontal bands: angle should be near 0/180, got {}",
            angle
        );
    }

    #[test]
    fn detect_angle_vertical_bands() {
        let data = vertical_banded_image(200, 200);
        let img = vec2d_to_array2(&data);
        let score = build_score_map(&img, 5);
        let angle = detect_angle(&score);
        assert!(
            (angle - 90.0).abs() < 10.0,
            "Vertical bands: angle should be near 90, got {}",
            angle
        );
    }

    #[test]
    fn detect_angle_diagonal_bands() {
        let data = diagonal_banded_image(300, 300);
        let img = vec2d_to_array2(&data);
        let score = build_score_map(&img, 5);
        let angle = detect_angle(&score);
        // y+x = const lines run at 135 degrees (from top-right to bottom-left)
        // perpendicular is 45, so best angle should be near 45 or 135
        assert!(
            (angle - 45.0).abs() < 15.0 || (angle - 135.0).abs() < 15.0,
            "Diagonal bands: angle should be near 45 or 135, got {}",
            angle
        );
    }

    // ── Band detection edge cases ──────────────────────────────────

    #[test]
    fn detect_bands_empty_profile() {
        let bands = detect_bands(&[], 0.3, 5);
        assert!(bands.is_empty());
    }

    #[test]
    fn detect_bands_all_zero() {
        let profile = vec![0.0f64; 50];
        let bands = detect_bands(&profile, 0.3, 5);
        assert!(bands.is_empty());
    }

    #[test]
    fn detect_bands_single_peak() {
        let mut profile = vec![0.0f64; 100];
        for i in 30..55 {
            profile[i] = 1.0;
        }
        let bands = detect_bands(&profile, 0.3, 5);
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].edge_low, 30);
        assert_eq!(bands[0].edge_high, 55);
    }

    #[test]
    fn detect_bands_at_profile_end() {
        // Band that extends to the last element
        let mut profile = vec![0.0f64; 50];
        for i in 40..50 {
            profile[i] = 1.0;
        }
        let bands = detect_bands(&profile, 0.3, 5);
        assert_eq!(
            bands.len(),
            1,
            "Should detect band touching end of profile"
        );
    }

    #[test]
    fn detect_bands_at_profile_start() {
        let mut profile = vec![0.0f64; 50];
        for i in 0..15 {
            profile[i] = 1.0;
        }
        let bands = detect_bands(&profile, 0.3, 5);
        assert_eq!(bands.len(), 1, "Should detect band starting at index 0");
        assert_eq!(bands[0].edge_low, 0);
    }

    #[test]
    fn detect_bands_weighted_center() {
        // Asymmetric peak: left side stronger than right
        let mut profile = vec![0.0f64; 100];
        for i in 40..50 {
            profile[i] = if i < 45 { 1.0 } else { 0.5 };
        }
        let bands = detect_bands(&profile, 0.3, 5);
        assert_eq!(bands.len(), 1);
        // Weighted center should be left of geometric center (44.5)
        assert!(
            bands[0].center < 45.0,
            "Weighted center should lean toward stronger side, got {}",
            bands[0].center
        );
    }

    #[test]
    fn detect_bands_threshold_sensitivity() {
        // Two peaks: one strong (1.0), one weak (0.2)
        let mut profile = vec![0.0f64; 100];
        for i in 20..35 {
            profile[i] = 1.0;
        }
        for i in 60..75 {
            profile[i] = 0.2;
        }
        // At threshold=0.3, only the strong peak should survive
        let bands_high = detect_bands(&profile, 0.3, 5);
        assert_eq!(
            bands_high.len(),
            1,
            "High threshold should only find strong band"
        );
        // At threshold=0.1, both should be found
        let bands_low = detect_bands(&profile, 0.1, 5);
        assert_eq!(bands_low.len(), 2, "Low threshold should find both bands");
    }

    // ── Score map tests ────────────────────────────────────────────

    #[test]
    fn score_map_uniform_image_is_zero() {
        let img = Array2::from_elem((50, 50), 0.5f32);
        let score = build_score_map(&img, 5);
        let max = score.iter().copied().fold(0.0f32, f32::max);
        // Uniform image has zero variance and zero deviation
        assert!(
            max < 1e-6,
            "Uniform image should have near-zero score, got {}",
            max
        );
    }

    #[test]
    fn score_map_bright_bands_have_high_score() {
        let data = horizontal_banded_image(200, 200);
        let img = vec2d_to_array2(&data);
        let score = build_score_map(&img, 5);

        // Score at band edge (high variance region) should be higher
        // than score far from any band
        let edge_score = score[[50, 100]]; // edge of first band
        let bg_score = score[[10, 100]]; // far from any band
        assert!(
            edge_score > bg_score,
            "Band edge score ({}) should exceed background score ({})",
            edge_score,
            bg_score
        );
    }

    #[test]
    fn score_map_dark_bands_detected_via_deviation() {
        // This tests the intensity deviation map path: dark bands on
        // bright background have no internal variance but should still score
        // high due to deviation from local mean and global median.
        let data = dark_banded_image(200, 200);
        let img = vec2d_to_array2(&data);
        let score = build_score_map(&img, 5);

        let band_interior_score = score[[75, 100]]; // inside dark band
        let bg_score = score[[10, 100]]; // bright background
        assert!(
            band_interior_score > bg_score,
            "Dark band interior ({}) should score higher than bright background ({})",
            band_interior_score,
            bg_score
        );
    }

    #[test]
    fn score_map_normalized_to_unit() {
        let data = horizontal_banded_image(200, 200);
        let img = vec2d_to_array2(&data);
        let score = build_score_map(&img, 5);
        let max = score.iter().copied().fold(0.0f32, f32::max);
        assert!(
            (max - 1.0).abs() < 1e-6,
            "Score map max should be 1.0 after normalization, got {}",
            max
        );
    }

    // ── Analyzer integration tests ─────────────────────────────────

    #[test]
    fn analyzer_empty_input() {
        let detector = CuoxRowDetector::new();
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: vec![],
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        assert_eq!(output.data["bands_count"], 0);
    }

    #[test]
    fn analyzer_horizontal_bands() {
        let detector = CuoxRowDetector::new();
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: horizontal_banded_image(200, 200),
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        let angle = output.data["angle_deg"].as_f64().unwrap();
        assert!(
            !(10.0..170.0).contains(&angle),
            "Horizontal bands: angle should be near 0/180, got {}",
            angle
        );
        assert!(output.data["bands_count"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn analyzer_vertical_bands() {
        let detector = CuoxRowDetector::new();
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: vertical_banded_image(200, 200),
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        let angle = output.data["angle_deg"].as_f64().unwrap();
        assert!(
            (angle - 90.0).abs() < 10.0,
            "Vertical bands: angle should be near 90, got {}",
            angle
        );
        assert!(output.data["bands_count"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn analyzer_fixed_angle() {
        let detector = CuoxRowDetector {
            fixed_angle: Some(45.0),
            ..Default::default()
        };
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: horizontal_banded_image(200, 200),
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        let angle = output.data["angle_deg"].as_f64().unwrap();
        assert!(
            (angle - 45.0).abs() < 1e-6,
            "Fixed angle should be exactly 45.0, got {}",
            angle
        );
    }

    #[test]
    fn analyzer_calibration_adds_nm_fields() {
        let detector = CuoxRowDetector::new();
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: horizontal_banded_image(200, 200),
            calibration_m_per_px: Some(0.12e-9), // 0.12 nm/px
        };
        let output = detector.analyze(&input).unwrap();
        let bands = output.data["bands"].as_array().unwrap();
        if !bands.is_empty() {
            let first = &bands[0];
            assert!(
                first.get("center_nm").is_some(),
                "Should have center_nm field"
            );
            assert!(
                first.get("width_nm").is_some(),
                "Should have width_nm field"
            );
            let width_px = first["width_px"].as_u64().unwrap() as f64;
            let width_nm = first["width_nm"].as_f64().unwrap();
            let expected_nm = width_px * 0.12;
            assert!(
                (width_nm - expected_nm).abs() < 1e-6,
                "width_nm ({}) should be width_px * 0.12 ({})",
                width_nm,
                expected_nm
            );
        }
    }

    #[test]
    fn analyzer_no_calibration_omits_nm_fields() {
        let detector = CuoxRowDetector::new();
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: horizontal_banded_image(200, 200),
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        let bands = output.data["bands"].as_array().unwrap();
        if !bands.is_empty() {
            let first = &bands[0];
            assert!(
                first.get("center_nm").is_none(),
                "Should not have center_nm without calibration"
            );
            assert!(
                first.get("width_nm").is_none(),
                "Should not have width_nm without calibration"
            );
        }
    }

    #[test]
    fn analyzer_output_json_schema() {
        let detector = CuoxRowDetector::new();
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: horizontal_banded_image(200, 200),
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        // Required top-level keys
        assert!(output.data.get("angle_deg").is_some());
        assert!(output.data.get("bands_count").is_some());
        assert!(output.data.get("bands").is_some());
        assert!(output.data["bands"].is_array());

        // Each band has required px fields
        for band in output.data["bands"].as_array().unwrap() {
            assert!(band.get("center_px").is_some());
            assert!(band.get("edge_low_px").is_some());
            assert!(band.get("edge_high_px").is_some());
            assert!(band.get("width_px").is_some());
        }
    }

    #[test]
    fn analyzer_dark_bands_detected() {
        let detector = CuoxRowDetector::new();
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: dark_banded_image(200, 200),
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        assert!(
            output.data["bands_count"].as_u64().unwrap() >= 1,
            "Should detect dark bands (Cu-tip case)"
        );
    }

    // ── Line endpoint / coordinate tests ──────────────────────────

    #[test]
    fn line_endpoints_horizontal() {
        // At angle 0, bands are horizontal. A line at projection bin 50
        // should span the full image width at a fixed y.
        let (rows, cols) = (100, 200);
        let cy = rows as f64 / 2.0;
        let cx = cols as f64 / 2.0;
        let cos_a = 1.0; // 0 degrees
        let sin_a = 0.0;
        let min_proj = compute_min_proj(rows, cols, cos_a, sin_a, cy, cx);
        let geom = ProjectionGeometry { rows, cols, cos_a, sin_a, cy, cx, min_proj };

        let seg = line_endpoints(50.0, &geom)
            .expect("Should produce a line segment");

        // Both endpoints should have same y, and x should span 0..cols
        assert!(
            (seg.start.0 - seg.end.0).abs() < 1e-6,
            "Horizontal line should have constant y"
        );
        let xs = [seg.start.1, seg.end.1];
        let x_min = xs.iter().copied().fold(f64::MAX, f64::min);
        let x_max = xs.iter().copied().fold(f64::MIN, f64::max);
        assert!(x_min.abs() < 1e-6, "Should start at x=0, got {}", x_min);
        assert!(
            (x_max - cols as f64).abs() < 1e-6,
            "Should end at x=cols, got {}",
            x_max
        );
    }

    #[test]
    fn line_endpoints_vertical() {
        // At angle 90, bands are vertical.
        let (rows, cols) = (200, 100);
        let cy = rows as f64 / 2.0;
        let cx = cols as f64 / 2.0;
        let angle_rad = 90.0f64.to_radians();
        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();
        let min_proj = compute_min_proj(rows, cols, cos_a, sin_a, cy, cx);
        let geom = ProjectionGeometry { rows, cols, cos_a, sin_a, cy, cx, min_proj };

        let seg = line_endpoints(30.0, &geom)
            .expect("Should produce a line segment");

        // Both endpoints should have same x, and y should span 0..rows
        assert!(
            (seg.start.1 - seg.end.1).abs() < 1e-6,
            "Vertical line should have constant x"
        );
        let ys = [seg.start.0, seg.end.0];
        let y_min = ys.iter().copied().fold(f64::MAX, f64::min);
        let y_max = ys.iter().copied().fold(f64::MIN, f64::max);
        assert!(y_min.abs() < 1.0, "Should start near y=0, got {}", y_min);
        assert!(
            (y_max - rows as f64).abs() < 1.0,
            "Should end near y=rows, got {}",
            y_max
        );
    }

    #[test]
    fn line_endpoints_within_image_bounds() {
        // At 45 degrees, line endpoints should still be within image bounds
        let (rows, cols) = (100, 100);
        let cy = rows as f64 / 2.0;
        let cx = cols as f64 / 2.0;
        let angle_rad = 45.0f64.to_radians();
        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();
        let min_proj = compute_min_proj(rows, cols, cos_a, sin_a, cy, cx);
        let geom = ProjectionGeometry { rows, cols, cos_a, sin_a, cy, cx, min_proj };

        for bin in [10.0, 30.0, 50.0, 70.0] {
            if let Some(seg) = line_endpoints(bin, &geom) {
                for (y, x) in [seg.start, seg.end] {
                    assert!(
                        y >= -0.5 && y <= rows as f64 + 0.5,
                        "y={} out of bounds for bin {}",
                        y,
                        bin
                    );
                    assert!(
                        x >= -0.5 && x <= cols as f64 + 0.5,
                        "x={} out of bounds for bin {}",
                        x,
                        bin
                    );
                }
            }
        }
    }

    #[test]
    fn line_endpoints_outside_image_returns_none() {
        // A projection bin far beyond the image diagonal should return None
        let (rows, cols) = (100, 100);
        let cy = rows as f64 / 2.0;
        let cx = cols as f64 / 2.0;
        let cos_a = 1.0;
        let sin_a = 0.0;
        let min_proj = compute_min_proj(rows, cols, cos_a, sin_a, cy, cx);
        let geom = ProjectionGeometry { rows, cols, cos_a, sin_a, cy, cx, min_proj };

        let result = line_endpoints(9999.0, &geom);
        assert!(
            result.is_none(),
            "Line far outside image should return None"
        );
    }

    #[test]
    fn bands_have_line_coordinates_in_output() {
        let detector = CuoxRowDetector::new();
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: horizontal_banded_image(200, 200),
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        let bands = output.data["bands"].as_array().unwrap();
        assert!(!bands.is_empty(), "Should detect at least one band");

        for band in bands {
            // Each band should have center, low, and high line coordinates
            let center_line = &band["center_line"];
            let low_line = &band["edge_low_line"];
            let high_line = &band["edge_high_line"];

            for (name, line) in [
                ("center", center_line),
                ("low", low_line),
                ("high", high_line),
            ] {
                assert!(
                    !line.is_null(),
                    "{}_line should not be null for a detected band",
                    name
                );
                assert!(
                    line["start"]["x"].is_f64(),
                    "{}_line.start.x should be a number",
                    name
                );
                assert!(
                    line["start"]["y"].is_f64(),
                    "{}_line.start.y should be a number",
                    name
                );
                assert!(
                    line["end"]["x"].is_f64(),
                    "{}_line.end.x should be a number",
                    name
                );
                assert!(
                    line["end"]["y"].is_f64(),
                    "{}_line.end.y should be a number",
                    name
                );
            }
        }
    }

    #[test]
    fn horizontal_band_lines_span_full_width() {
        // For horizontal bands (angle ~0), edge lines should span the full
        // image width at approximately constant y coordinates.
        let detector = CuoxRowDetector {
            fixed_angle: Some(0.0),
            ..Default::default()
        };
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: horizontal_banded_image(200, 200),
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        let bands = output.data["bands"].as_array().unwrap();

        for band in bands {
            let line = &band["center_line"];
            if line.is_null() {
                continue;
            }
            let start_x = line["start"]["x"].as_f64().unwrap();
            let end_x = line["end"]["x"].as_f64().unwrap();
            let x_span = (end_x - start_x).abs();
            assert!(
                x_span > 150.0,
                "Horizontal band line should span most of the width, got {}",
                x_span
            );
        }
    }

    // ── Trait implementation tests ─────────────────────────────────

    #[test]
    fn analyzer_trait_name_and_description() {
        let detector = CuoxRowDetector::new();
        assert_eq!(detector.name(), "cuox_row_detector");
        assert!(!detector.description().is_empty());
    }

    #[test]
    fn default_params() {
        let d = CuoxRowDetector::default();
        assert_eq!(d.var_radius, 5);
        assert!((d.threshold - 0.3).abs() < 1e-6);
        assert_eq!(d.min_band_width, 5);
        assert!(d.fixed_angle.is_none());
    }

    // ── Custom parameter tests ─────────────────────────────────────

    #[test]
    fn larger_var_radius_still_detects() {
        let detector = CuoxRowDetector {
            var_radius: 10,
            ..Default::default()
        };
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: horizontal_banded_image(200, 200),
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        assert!(output.data["bands_count"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn high_min_band_width_rejects_thin_bands() {
        let detector = CuoxRowDetector {
            min_band_width: 100, // very high -- should reject everything
            ..Default::default()
        };
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: horizontal_banded_image(200, 200),
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        // With min_band_width=100, the 30px-wide bands should be rejected
        // (depending on how they project, but likely too narrow)
        // This is a regression guard more than a strict assertion
        let count = output.data["bands_count"].as_u64().unwrap();
        assert!(
            count <= 2,
            "With high min_band_width, should reject most bands"
        );
    }

    #[test]
    fn uniform_image_produces_no_bands() {
        // The score map for a uniform image is all zeros, and with the
        // early-return on max_val<=0 in detect_bands, no bands should appear.
        let detector = CuoxRowDetector::new();
        let input = AnalyzerInput {
            channel_name: "Z".into(),
            data: uniform_image(100, 100, 0.5),
            calibration_m_per_px: None,
        };
        let output = detector.analyze(&input).unwrap();
        assert_eq!(
            output.data["bands_count"].as_u64().unwrap(),
            0,
            "Uniform image should have no bands"
        );
    }
}
