//! Rolling-ellipsoid dilation: constant-current topography to a
//! constant-distance tip trajectory.
//!
//! # What this computes
//!
//! A constant-current scan gives a height map `Z[i, j]`: the Z the feedback
//! loop had to hold to keep the tunnelling current at its setpoint. To scan at
//! constant *distance* instead, the feedback is switched off and Z is driven
//! along a precomputed path. That path is not `Z` itself, because a tip has a
//! finite body: near a step edge the flank of the tip touches the upper terrace
//! long before the apex is above it.
//!
//! The classical fix is the rolling-sphere construction (the same idea as the
//! rolling sphere in lightning protection, or the rolling-ball baseline in AFM
//! image processing): roll a rigid body over the surface and record where its
//! apex goes. Here the body is an ellipsoid, so the lateral and vertical
//! clearances can be chosen independently: an ellipsoid wider than it is tall
//! gives step edges extra lateral berth without pulling the tip needlessly far
//! from flat terraces.
//!
//! # The construction
//!
//! Let the ellipsoid have lateral semi-axes `a_x`, `a_y` and vertical semi-axis
//! `c`, all in metres, and let the scan grid have sample spacing `dx`, `dy` in
//! metres. Place the ellipsoid centre at height `Zc` above a lateral position;
//! its lower surface at lateral offset `(dx*v, dy*u)` from that centre sits at
//!
//! ```text
//!     Zc - c * sqrt(1 - (u*dy/a_y)^2 - (v*dx/a_x)^2)
//! ```
//!
//! The ellipsoid is just clear of the surface when that lower surface touches
//! `Z` somewhere and is above it everywhere else, which means
//!
//! ```text
//!     Zc[i, j] = max over (u, v) of ( Z[i+u, j+v] + B[u, v] )
//!
//!     B[u, v] = c * sqrt(1 - t)   where t = (u*dy/a_y)^2 + (v*dx/a_x)^2 <= 1
//!             = -infinity         otherwise (offset is outside the footprint)
//! ```
//!
//! That is a grayscale morphological *dilation* of `Z` by the structuring
//! element `B`. The apex of the ellipsoid hangs `c` below its centre, so the
//! trajectory the tip must follow is
//!
//! ```text
//!     Z_tip = Zc - c
//! ```
//!
//! `B` is precomputed once as a small table ([`RollingEllipsoid::bias_table`]).
//! The `-infinity` entries are not a special case in the inner loop: they can
//! never win a maximum against a finite candidate, so "outside the ellipse" and
//! "inside" collapse into one branch-free max.
//!
//! # What this does *not* model
//!
//! - Feedback off holds **Z**, not distance. This computes the Z path that
//!   *would* hold distance for a rigid tip of the assumed shape; drift, creep
//!   and a wrong tip shape all break that.
//! - A constant-current Z map is an iso-current surface, not true topography.
//!   Electronic contrast enters as apparent height. It is a decent first-order
//!   proxy and nothing more.
//! - The tip is modelled as a single smooth ellipsoid. Double tips, asymmetric
//!   tips and adsorbate-terminated tips are out of scope.
//!
//! # Cost
//!
//! `O(ny * nx * (2*ry+1) * (2*rx+1))` with `ry = ceil(a_y/dy)`,
//! `rx = ceil(a_x/dx)`. Rows are dilated in parallel. This is the naive
//! rectangular-window form; if the footprint ever gets big enough to hurt, the
//! van Herk / Gil-Werman running-max trick makes it independent of radius, at
//! the cost of only being separable for flat (non-domed) elements, so the dome
//! would have to be handled as a sum of shifted 1D passes.

use ndarray::{Array2, ArrayView2, Axis, Zip};

/// Sample spacing of a scan grid, in metres.
///
/// Nanonis has no notion of a pixel size: this comes out of the scan range
/// divided by the number of samples per line, and the number of lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridSpacing {
    /// Spacing along the fast (column) axis, metres per sample.
    pub dx: f64,
    /// Spacing along the slow (row) axis, metres per sample.
    pub dy: f64,
}

impl GridSpacing {
    /// Equal spacing on both axes.
    pub fn square(d: f64) -> Self {
        Self { dx: d, dy: d }
    }

    /// Spacing implied by a scan range and a sample count.
    ///
    /// `width` and `height` are the scan frame size in metres, `nx` and `ny`
    /// the number of samples per line and the number of lines.
    pub fn from_frame(width: f64, height: f64, nx: usize, ny: usize) -> Self {
        Self {
            dx: width / nx as f64,
            dy: height / ny as f64,
        }
    }
}

/// How the dilation treats samples that fall outside the scan frame.
///
/// The footprint of the ellipsoid overhangs the edge of the map for any sample
/// within `a` of the border, and what lies out there was never measured. The
/// two answers differ only inside that border strip, but they differ in a way
/// that matters: [`Border::Truncate`] lets the tip dive at the frame edge,
/// [`Border::Replicate`] does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Border {
    /// Ignore out-of-frame offsets: the window is clipped to the array.
    ///
    /// Equivalent to assuming the surface outside the frame is infinitely far
    /// below, so it never constrains the tip. Honest about what was measured,
    /// but it means the trajectory within `a` of the border is computed from
    /// less evidence than the interior, and can sit lower than it should if the
    /// surface actually continues upward outside the frame.
    #[default]
    Truncate,
    /// Clamp out-of-frame offsets to the nearest edge sample.
    ///
    /// Equivalent to assuming the surface continues outward at its border
    /// height. The conservative choice: it never lets the tip drop for lack of
    /// data. Prefer this when the trajectory will actually be driven on
    /// hardware and a border dive would be a crash.
    Replicate,
}

/// A rigid ellipsoid rolled over the surface to plan a constant-distance path.
///
/// Semi-axes are in metres. `a_x` and `a_y` are lateral (along the fast and
/// slow scan axis), `c` is vertical.
///
/// Two things about the parameters are worth having straight before choosing
/// them:
///
/// - **Neither buys a constant lift.** On a flat terrace the ellipsoid rests on
///   the surface and the trajectory *is* the surface, whatever the semi-axes
///   are. They set the clearance only where the surface is stepped or curved. A
///   constant standoff is a different request, and a much simpler one.
/// - **Bluntness is the aspect ratio `a/c`, and it runs the way that surprises
///   people.** A large `c` at fixed `a` is a slender, sharp body that tucks in
///   close to a step edge; a *small* `c` is a flat, blunt body that has to
///   stand well off. Wider than tall is what gives edges lateral berth. In the
///   limit `c -> 0` the dome becomes a disc and the construction degenerates to
///   a plain sliding-window maximum, the most conservative plan available.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RollingEllipsoid {
    /// Lateral semi-axis along the fast (column) axis, metres.
    pub a_x: f64,
    /// Lateral semi-axis along the slow (row) axis, metres.
    pub a_y: f64,
    /// Vertical semi-axis, metres. The apex hangs this far below the centre.
    pub c: f64,
}

impl RollingEllipsoid {
    /// An ellipsoid with the same lateral semi-axis on both scan axes.
    pub fn isotropic(a: f64, c: f64) -> Self {
        Self { a_x: a, a_y: a, c }
    }

    /// An ellipsoid with independent lateral semi-axes.
    ///
    /// Useful when the two scan axes are sampled very differently, or when a
    /// known tip asymmetry should be modelled.
    pub fn new(a_x: f64, a_y: f64, c: f64) -> Self {
        Self { a_x, a_y, c }
    }

    /// Footprint radius in samples, `(ry, rx)`.
    ///
    /// Rounded up, so the footprint always covers the full lateral extent of
    /// the ellipsoid.
    pub fn radii(&self, sp: GridSpacing) -> (usize, usize) {
        let ry = (self.a_y / sp.dy).ceil() as usize;
        let rx = (self.a_x / sp.dx).ceil() as usize;
        (ry, rx)
    }

    /// Whether the footprint is wide enough for the dilation to mean anything.
    ///
    /// If the ellipsoid is narrower than one sample on an axis, the only offset
    /// that survives the ellipse test on that axis is zero, the dilation
    /// collapses to `Zc = Z + c`, and the trajectory comes back as `Z_tip = Z`:
    /// the surface, unchanged, silently. A footprint of at least a few samples
    /// per axis is the difference between planning a path and doing nothing.
    ///
    /// Returns `true` when both axes span at least `min_samples` samples of
    /// ellipsoid (i.e. `2*r + 1 >= min_samples`). Three is the bare minimum;
    /// five or more is a usable dome.
    pub fn is_resolved(&self, sp: GridSpacing, min_samples: usize) -> bool {
        let (ry, rx) = self.radii(sp);
        2 * ry + 1 >= min_samples && 2 * rx + 1 >= min_samples
    }

    /// The structuring element `B`, shape `(2*ry+1, 2*rx+1)`.
    ///
    /// Indexed by shifted offset: `B[(u + ry, v + rx)]` is the value for offset
    /// `(u, v)`. Entries whose offset falls outside the ellipse are
    /// `f64::NEG_INFINITY`.
    ///
    /// # Panics
    ///
    /// If any semi-axis or spacing is not finite and strictly positive.
    pub fn bias_table(&self, sp: GridSpacing) -> Array2<f64> {
        assert!(
            self.a_x > 0.0 && self.a_y > 0.0 && self.c > 0.0,
            "ellipsoid semi-axes must be positive (a_x={}, a_y={}, c={})",
            self.a_x,
            self.a_y,
            self.c
        );
        assert!(
            sp.dx > 0.0 && sp.dy > 0.0 && sp.dx.is_finite() && sp.dy.is_finite(),
            "grid spacing must be finite and positive (dx={}, dy={})",
            sp.dx,
            sp.dy
        );

        let (ry, rx) = self.radii(sp);
        let mut b = Array2::from_elem((2 * ry + 1, 2 * rx + 1), f64::NEG_INFINITY);

        for du in 0..=2 * ry {
            let u = du as f64 - ry as f64;
            let ty = (u * sp.dy / self.a_y).powi(2);
            for dv in 0..=2 * rx {
                let v = dv as f64 - rx as f64;
                let t = ty + (v * sp.dx / self.a_x).powi(2);
                if t <= 1.0 {
                    b[(du, dv)] = self.c * (1.0 - t).sqrt();
                }
            }
        }
        b
    }

    /// The Z trajectory the tip must hold to keep the ellipsoid clear of `z`.
    ///
    /// `z` is a height map in metres, indexed `[row, column]` = `[y, x]`, as it
    /// comes off a constant-current scan. The result has the same shape and is
    /// also in metres, in the same reference frame as `z`.
    ///
    /// Guarantees, all of which are covered by the module tests:
    ///
    /// - `Z_tip[i, j] >= z[i, j]` everywhere, exactly, not just to within
    ///   round-off. The tip never goes below the surface directly under it.
    /// - On a plane, `Z_tip == z` exactly. `c` buys clearance at *edges*, not a
    ///   constant lift; a constant lift would be `z + h`, which is a different
    ///   (and much simpler) thing to ask for.
    /// - Approaching an up-step of height `H`, `Z_tip` starts to rise at most
    ///   `a` before the edge and reaches `H` exactly at the edge. Leaving a
    ///   down-step, it stays high for at most `a` past the edge.
    ///
    /// # Panics
    ///
    /// If `z` is empty, or any semi-axis or spacing is not finite and strictly
    /// positive.
    pub fn tip_trajectory(
        &self,
        z: ArrayView2<f64>,
        sp: GridSpacing,
        border: Border,
    ) -> Array2<f64> {
        let (ny, nx) = z.dim();
        assert!(ny > 0 && nx > 0, "height map must not be empty");

        let bias = self.bias_table(sp);
        let (ry, rx) = self.radii(sp);

        // One row of the dilation. Split out so rows can be planned in
        // parallel: every output sample depends only on `z`, never on another
        // output sample, so there is nothing to synchronise.
        let plan_row = |i: usize, row: &mut ndarray::ArrayViewMut1<f64>| {
            // The row indices depend only on `i`, so resolving them once per
            // row keeps the border policy out of the inner loop entirely.
            let rows: Vec<Option<usize>> = (0..=2 * ry)
                .map(|du| offset_index(i, du, ry, ny, border))
                .collect();

            for (j, out) in row.iter_mut().enumerate() {
                let mut peak = f64::NEG_INFINITY;

                for (du, ii) in rows.iter().enumerate() {
                    let Some(ii) = *ii else {
                        continue;
                    };
                    for dv in 0..=2 * rx {
                        let Some(jj) = offset_index(j, dv, rx, nx, border) else {
                            continue;
                        };
                        // NEG_INFINITY entries (outside the ellipse) can never
                        // win this max, so no explicit footprint test is needed.
                        let candidate = z[(ii, jj)] + bias[(du, dv)];
                        if candidate > peak {
                            peak = candidate;
                        }
                    }
                }
                // `peak` is the ellipsoid centre height; the apex hangs `c`
                // below. The offset (0, 0) is always inside the footprint and
                // always in range, so `peak >= z + c` holds exactly in real
                // arithmetic and the subtraction can never put the apex below
                // the surface. In floating point it can, by an ulp: `(z + c) -
                // c` is not exactly `z`. Clamping restores the invariant
                // exactly, which is worth doing because downstream code is
                // entitled to treat a negative clearance as a bug rather than
                // as noise. It cannot mask a real error: no real violation is
                // reachable.
                *out = (peak - self.c).max(z[(i, j)]);
            }
        };

        // Written straight into the output, so the map is allocated once. The
        // previous shape (collect rows, then flatten) copied every sample a
        // second time, which at scan sizes is megabytes for nothing.
        let mut out = Array2::zeros((ny, nx));
        Zip::indexed(out.axis_iter_mut(Axis(0))).par_for_each(|i, mut row| plan_row(i, &mut row));
        out
    }
}

/// Resolve a footprint offset to a sample index, honouring the border policy.
///
/// `pos` is the output sample, `d` the shifted offset index into the bias table
/// (`d - r` is the signed offset), `n` the array extent on that axis. Returns
/// `None` when the offset leaves the frame and the policy is to drop it.
#[inline]
fn offset_index(pos: usize, d: usize, r: usize, n: usize, border: Border) -> Option<usize> {
    let idx = pos as isize + d as isize - r as isize;
    match border {
        Border::Truncate => {
            if idx < 0 || idx >= n as isize {
                None
            } else {
                Some(idx as usize)
            }
        }
        Border::Replicate => Some(idx.clamp(0, n as isize - 1) as usize),
    }
}

/// Vertical gap between the planned trajectory and the surface below it.
///
/// This is *not* the physical tip-sample distance: that is the shortest
/// distance from the ellipsoid to the surface, which by construction is zero
/// wherever the ellipsoid touches. This is the plain vertical difference, which
/// is what you can actually inspect in a Z map, and it shows where the plan
/// lifts the tip and by how much.
///
/// # Panics
///
/// If the two maps have different shapes.
pub fn vertical_clearance(z: ArrayView2<f64>, z_tip: ArrayView2<f64>) -> Array2<f64> {
    assert_eq!(
        z.dim(),
        z_tip.dim(),
        "surface and trajectory must have the same shape"
    );
    // `Zip` walks both maps contiguously; indexing by `(i, j)` would redo the
    // stride arithmetic and bounds checks for every sample.
    Zip::from(z).and(z_tip).map_collect(|s, t| t - s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::{Array2, array};

    const D: f64 = 1e-10; // 0.1 nm sample spacing, a plausible atomic-scale grid

    fn spacing() -> GridSpacing {
        GridSpacing::square(D)
    }

    /// A single-row map, so the dilation reduces to the 1D case and the
    /// expected trajectory can be reasoned about by hand.
    fn row_map(values: &[f64]) -> Array2<f64> {
        Array2::from_shape_vec((1, values.len()), values.to_vec()).unwrap()
    }

    #[test]
    fn bias_table_has_expected_shape_and_apex() {
        // a = 3 samples wide, so ry = rx = 3 and the table is 7x7.
        let e = RollingEllipsoid::isotropic(3.0 * D, 5.0 * D);
        let b = e.bias_table(spacing());

        assert_eq!(b.dim(), (7, 7));
        // Centre offset: the full vertical semi-axis.
        assert!((b[(3, 3)] - 5.0 * D).abs() < 1e-18);
        // Exactly on the rim: the dome has come all the way down.
        assert!(b[(3, 6)].abs() < 1e-18);
        // Corner of the bounding box: outside the ellipse.
        assert_eq!(b[(0, 0)], f64::NEG_INFINITY);
        // Symmetric about the centre in both axes.
        assert_eq!(b[(3, 2)], b[(3, 4)]);
        assert_eq!(b[(2, 3)], b[(4, 3)]);
    }

    #[test]
    fn anisotropic_footprint_follows_the_semi_axes() {
        // Wide along x, narrow along y.
        let e = RollingEllipsoid::new(5.0 * D, 1.0 * D, 2.0 * D);
        assert_eq!(e.radii(spacing()), (1, 5));
        assert_eq!(e.bias_table(spacing()).dim(), (3, 11));
    }

    #[test]
    fn clearance_is_exactly_zero_on_a_terrace_not_merely_small() {
        // `(z + c) - c` is not exactly `z` in floating point, so without the
        // clamp in `tip_trajectory` this comes out at about -1e-25 m: harmless
        // in magnitude, but enough to make `clearance < 0.0` fire on flat
        // ground and turn a valid invariant into a flaky one.
        let z = Array2::from_elem((4, 6), 3.7e-9);
        let e = RollingEllipsoid::isotropic(4.0 * D, 2.0 * D);
        let tip = e.tip_trajectory(z.view(), spacing(), Border::Replicate);
        let gap = vertical_clearance(z.view(), tip.view());
        assert!(
            gap.iter().all(|g| *g == 0.0),
            "clearance was not exactly zero"
        );
    }

    #[test]
    fn flat_plane_is_traced_exactly() {
        // The whole point of returning the apex rather than the centre: on a
        // plane the ellipsoid rests on the surface and the tip path *is* the
        // surface. If this ever comes back as `z + c`, the `- c` was dropped.
        let z = Array2::from_elem((9, 11), 4.2e-9);
        let e = RollingEllipsoid::isotropic(4.0 * D, 2.0 * D);

        for border in [Border::Truncate, Border::Replicate] {
            let tip = e.tip_trajectory(z.view(), spacing(), border);
            for (got, want) in tip.iter().zip(z.iter()) {
                assert!((got - want).abs() < 1e-20, "got {got}, want {want}");
            }
        }
    }

    #[test]
    fn trajectory_never_dips_below_the_surface() {
        // A deterministic but lumpy surface: sums of incommensurate sines, so
        // no accidental symmetry with the sample grid.
        let (ny, nx) = (24, 31);
        let z = Array2::from_shape_fn((ny, nx), |(i, j)| {
            let (y, x) = (i as f64, j as f64);
            D * ((x * 0.37).sin() * 3.0 + (y * 0.21).cos() * 2.0 + (x * 0.11 + y * 0.29).sin())
        });
        let e = RollingEllipsoid::isotropic(3.0 * D, 2.0 * D);

        for border in [Border::Truncate, Border::Replicate] {
            let tip = e.tip_trajectory(z.view(), spacing(), border);
            let gap = vertical_clearance(z.view(), tip.view());
            let worst = gap.iter().cloned().fold(f64::INFINITY, f64::min);
            // Exactly zero, not "zero to within a tolerance": the trajectory is
            // clamped so that a negative clearance is always a real defect.
            assert!(worst >= 0.0, "tip dipped below the surface by {worst} m");
        }
    }

    #[test]
    fn up_step_is_anticipated_within_one_lateral_semi_axis() {
        // Surface: flat at 0, stepping up to H at column 20 and staying there.
        const EDGE: usize = 20;
        const H: f64 = 5.0 * D;
        let a = 6.0 * D; // 6 samples of reach
        let mut z = vec![0.0; 40];
        z[EDGE..].fill(H);
        let z = row_map(&z);

        let e = RollingEllipsoid::isotropic(a, 3.0 * D);
        let tip = e.tip_trajectory(z.view(), spacing(), Border::Truncate);
        let tip = tip.row(0);

        // Far from the edge the tip has no reason to know about the step.
        for j in 0..EDGE - 6 {
            assert!(tip[j].abs() < 1e-20, "tip lifted early at column {j}");
        }
        // Over the approach it rises monotonically...
        for j in (EDGE - 6)..EDGE {
            assert!(
                tip[j + 1] >= tip[j] - 1e-20,
                "approach not monotone at column {j}"
            );
        }
        // ...and arrives at full step height exactly at the edge.
        assert!((tip[EDGE] - H).abs() < 1e-20);
        // It also starts rising somewhere in the approach, rather than jumping.
        assert!(tip[EDGE - 3] > 0.0 && tip[EDGE - 3] < H);
    }

    #[test]
    fn down_step_keeps_the_tip_high_past_the_edge() {
        // The mirror of the previous case: high at 0..20, then a drop to 0.
        // Naively following Z would slam the tip flank into the upper terrace
        // it just left, so the trajectory has to lag.
        const EDGE: usize = 20;
        const H: f64 = 5.0 * D;
        let a = 6.0 * D;
        let mut z = vec![H; 40];
        z[EDGE..].fill(0.0);
        let z = row_map(&z);

        let e = RollingEllipsoid::isotropic(a, 3.0 * D);
        let tip = e.tip_trajectory(z.view(), spacing(), Border::Truncate);
        let tip = tip.row(0);

        // Still elevated immediately past the edge, even though Z is 0 there.
        assert!(tip[EDGE] > 0.0, "tip dropped straight off the step");
        assert!(tip[EDGE + 1] > 0.0);
        // Descends monotonically once past the edge.
        for j in EDGE..(EDGE + 6) {
            assert!(tip[j + 1] <= tip[j] + 1e-20, "descent not monotone at {j}");
        }
        // Beyond the reach of the ellipsoid it has forgotten the step.
        for j in (EDGE + 7)..40 {
            assert!(tip[j].abs() < 1e-20, "tip still high at column {j}");
        }
    }

    #[test]
    fn dilation_commutes_with_reflection() {
        // A strong structural invariant: the ellipsoid is symmetric, so
        // planning a mirrored surface must give the mirrored plan. This is what
        // makes the up-step and down-step cases each other's reflection, and it
        // catches off-by-one errors in the bias-table indexing that the
        // symmetric test surfaces above would hide.
        let mut z = vec![0.0; 30];
        z[7..13].fill(4.0 * D);
        z[20] = -2.0 * D;
        let forward = row_map(&z);
        z.reverse();
        let mirrored = row_map(&z);

        let e = RollingEllipsoid::isotropic(3.5 * D, 2.0 * D);
        let a = e.tip_trajectory(forward.view(), spacing(), Border::Replicate);
        let b = e.tip_trajectory(mirrored.view(), spacing(), Border::Replicate);

        for j in 0..30 {
            let (x, y) = (a[(0, j)], b[(0, 29 - j)]);
            assert!((x - y).abs() < 1e-20, "asymmetry at column {j}: {x} vs {y}");
        }
    }

    #[test]
    fn border_policies_differ_only_at_the_border() {
        // A tall wall at the left edge. Truncate has no data beyond column 0 and
        // ignores it; Replicate assumes the wall continues off-frame. They must
        // agree everywhere the footprint fits inside the frame.
        let mut z = vec![0.0; 30];
        z[0..3].fill(9.0 * D);
        let z = row_map(&z);

        let e = RollingEllipsoid::isotropic(4.0 * D, 3.0 * D);
        let t = e.tip_trajectory(z.view(), spacing(), Border::Truncate);
        let r = e.tip_trajectory(z.view(), spacing(), Border::Replicate);

        let (ry, rx) = e.radii(spacing());
        assert_eq!(ry, 4);
        for j in rx..(30 - rx) {
            assert!(
                (t[(0, j)] - r[(0, j)]).abs() < 1e-20,
                "policies disagree in the interior at column {j}"
            );
        }
        // Both keep the tip above the wall itself.
        assert!(t[(0, 0)] >= 9.0 * D - 1e-20);
        assert!(r[(0, 0)] >= 9.0 * D - 1e-20);
    }

    #[test]
    fn footprint_narrower_than_one_sample_is_a_no_op() {
        // Documents the failure mode `is_resolved` exists to catch: an
        // ellipsoid smaller than the sample spacing sees only the sample it
        // sits on, so the plan comes back as the raw surface. It is not an
        // error, it is worse: it looks like a result.
        let e = RollingEllipsoid::isotropic(0.4 * D, 5.0 * D);
        assert!(!e.is_resolved(spacing(), 5));

        let z = array![[0.0, 0.0, 7.0 * D, 0.0, 0.0]];
        let tip = e.tip_trajectory(z.view(), spacing(), Border::Truncate);
        for (got, want) in tip.iter().zip(z.iter()) {
            assert!((got - want).abs() < 1e-20, "expected a no-op, got {got}");
        }

        // A properly resolved ellipsoid does react to the same spike.
        let e = RollingEllipsoid::isotropic(3.0 * D, 5.0 * D);
        assert!(e.is_resolved(spacing(), 5));
        let tip = e.tip_trajectory(z.view(), spacing(), Border::Truncate);
        assert!(tip[(0, 0)] > 0.0, "resolved ellipsoid ignored the spike");
    }

    #[test]
    fn blunter_ellipsoid_stands_further_off_an_edge() {
        // Which way `c` works is the easy thing to get backwards, so it is
        // pinned here. Approaching an up-step of height H from distance d, the
        // apex is allowed to sag below H by `c * (1 - sqrt(1 - d^2/a^2))`, and
        // that sag *grows* with c. So at fixed `a`:
        //
        //   large c  ->  slender, sharp body  ->  tucks in close to the edge
        //   small c  ->  flat, blunt body     ->  has to stand well off
        //
        // The bluntness knob is the aspect ratio a/c, and "wider than tall" is
        // what buys lateral clearance at edges. Neither choice lifts the tip on
        // flat ground.
        const EDGE: usize = 15;
        let mut z = vec![0.0; 30];
        z[EDGE..].fill(4.0 * D);
        let z = row_map(&z);
        let sp = spacing();

        let blunt = RollingEllipsoid::isotropic(5.0 * D, 1.0 * D);
        let sharp = RollingEllipsoid::isotropic(5.0 * D, 8.0 * D);
        let b = blunt.tip_trajectory(z.view(), sp, Border::Truncate);
        let s = sharp.tip_trajectory(z.view(), sp, Border::Truncate);

        assert!(
            b[(0, EDGE - 2)] > s[(0, EDGE - 2)],
            "blunt body should stand further off the edge than the sharp one"
        );
        assert!(
            b[(0, 0)].abs() < 1e-20 && s[(0, 0)].abs() < 1e-20,
            "neither body should lift the tip on a flat terrace"
        );
    }

    #[test]
    fn a_vanishingly_flat_ellipsoid_becomes_a_plain_maximum_filter() {
        // The limiting case of the above, and a useful sanity anchor: as c goes
        // to zero the dome flattens into a disc, the bias table collapses to
        // all-zeros-inside-the-footprint, and the dilation degenerates to a
        // plain sliding-window maximum. The trajectory then sits at full step
        // height across the whole reach `a`, which is the most conservative
        // plan the construction can produce.
        const EDGE: usize = 15;
        const H: f64 = 4.0 * D;
        let a = 5.0 * D;
        let mut z = vec![0.0; 30];
        z[EDGE..].fill(H);
        let z = row_map(&z);

        let disc = RollingEllipsoid::isotropic(a, 1e-6 * D);
        let tip = disc.tip_trajectory(z.view(), spacing(), Border::Truncate);

        for j in (EDGE - 5)..EDGE {
            // The residual sag is bounded by c itself, so that is the scale
            // the comparison has to be made at, not machine epsilon.
            assert!(
                (tip[(0, j)] - H).abs() < 1e-5 * D,
                "expected a flat plateau at step height, got {} at {j}",
                tip[(0, j)]
            );
        }
        assert!(tip[(0, EDGE - 6)].abs() < 1e-20, "plateau reached beyond a");
    }
}
