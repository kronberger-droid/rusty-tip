//! Synthetic test surfaces.
//!
//! Piece 2 of the constant-distance scan (acquire a real Z map off the TCP
//! sample stream) does not exist yet, so there is no measured input to plan
//! against. These generators stand in for it: they produce height maps with
//! features whose correct handling can be judged by eye, which is exactly what
//! is needed to answer "is this mapping reasonable?".
//!
//! They are also a decent regression harness. A real Z map is noisy and
//! ambiguous; a synthetic staircase has a known step height and a known edge
//! position, so an obviously wrong plan looks obviously wrong.
//!
//! All heights are in metres.

use clap::ValueEnum;
use ndarray::Array2;
use rusty_tip::analyzer::rolling_ellipsoid::GridSpacing;

/// Which synthetic surface to generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Kind {
    /// Perfectly flat. The trajectory must come back identical to the input.
    Flat,
    /// One up-step across the middle, perpendicular to the fast axis.
    Step,
    /// A staircase of four terraces along the fast axis.
    Terraces,
    /// A flat terrace with a square pit cut into it.
    Pit,
    /// Three Gaussian bumps of different widths on a flat terrace.
    Bumps,
    /// Staircase, bump and pit in one frame. The best one to look at first.
    Combo,
}

/// Generate a synthetic height map, row-major `[row, column]`, in metres.
///
/// `amp` is the characteristic feature height in metres: one step of the
/// staircase, the depth of the pit, the height of the tallest bump.
pub fn generate(kind: Kind, ny: usize, nx: usize, sp: GridSpacing, amp: f64) -> Array2<f64> {
    match kind {
        Kind::Flat => Array2::zeros((ny, nx)),
        Kind::Step => Array2::from_shape_fn((ny, nx), |(_, j)| if j >= nx / 2 { amp } else { 0.0 }),
        Kind::Terraces => Array2::from_shape_fn((ny, nx), |(_, j)| {
            // Four terraces, so three edges land inside the frame.
            (j * 4 / nx) as f64 * amp
        }),
        Kind::Pit => Array2::from_shape_fn((ny, nx), |(i, j)| {
            let inside = (nx / 4..3 * nx / 4).contains(&j) && (ny / 4..3 * ny / 4).contains(&i);
            if inside { -amp } else { 0.0 }
        }),
        Kind::Bumps => {
            let width = nx as f64 * sp.dx;
            let height = ny as f64 * sp.dy;
            // Deliberately spanning an order of magnitude in width, because the
            // interesting question is which features the tip can still follow
            // into and which it rides straight over.
            let sigmas = [0.02, 0.06, 0.15].map(|f| resolved_sigma(width * f, sp));
            Array2::from_shape_fn((ny, nx), |(i, j)| {
                let x = (j as f64 + 0.5) * sp.dx;
                let y = (i as f64 + 0.5) * sp.dy;
                gaussian(x, y, width * 0.25, height * 0.5, sigmas[0], amp)
                    + gaussian(x, y, width * 0.5, height * 0.5, sigmas[1], amp * 0.7)
                    + gaussian(x, y, width * 0.75, height * 0.5, sigmas[2], amp * 0.4)
            })
        }
        Kind::Combo => {
            let width = nx as f64 * sp.dx;
            let height = ny as f64 * sp.dy;
            Array2::from_shape_fn((ny, nx), |(i, j)| {
                // Left third: staircase. Right two thirds: a bump and a pit on
                // the top terrace, so every feature type sits in one frame.
                let terrace = ((j * 6 / nx).min(2)) as f64 * amp;
                let x = (j as f64 + 0.5) * sp.dx;
                let y = (i as f64 + 0.5) * sp.dy;
                let sigma = resolved_sigma(width * 0.05, sp);
                let bump = gaussian(x, y, width * 0.6, height * 0.3, sigma, amp * 1.5);
                let in_pit =
                    (0.75..0.9).contains(&(x / width)) && (0.55..0.85).contains(&(y / height));
                // Three terraces deep, so the pit floor sits below the bottom
                // terrace rather than level with it: a hole a blunt tip cannot
                // reach into is more informative than a dent.
                terrace + bump - if in_pit { amp * 3.0 } else { 0.0 }
            })
        }
    }
}

/// Widen a Gaussian until it spans at least three samples on both axes.
///
/// Feature sizes are given as a fraction of the frame, so a small `--nx` can
/// ask for a bump narrower than the sample spacing. That is not a useful test
/// surface: it aliases into a spike whose height depends on where the grid
/// happens to fall, and the plan computed from it says more about the sampling
/// than about the tip. Widening is the lesser evil, and it is visible in the
/// output rather than silently wrong.
fn resolved_sigma(sigma: f64, sp: GridSpacing) -> f64 {
    sigma.max(3.0 * sp.dx.max(sp.dy))
}

/// A 2D Gaussian of peak height `amp` centred at `(cx, cy)` with sigma `sigma`.
fn gaussian(x: f64, y: f64, cx: f64, cy: f64, sigma: f64, amp: f64) -> f64 {
    let r2 = (x - cx).powi(2) + (y - cy).powi(2);
    amp * (-r2 / (2.0 * sigma * sigma)).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NM: f64 = 1e-9;

    fn sp() -> GridSpacing {
        GridSpacing::square(0.1 * NM)
    }

    #[test]
    fn flat_is_flat() {
        let z = generate(Kind::Flat, 8, 8, sp(), NM);
        assert!(z.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn step_has_exactly_the_requested_height() {
        let z = generate(Kind::Step, 4, 10, sp(), 0.25 * NM);
        assert_eq!(z[(0, 0)], 0.0);
        assert_eq!(z[(0, 9)], 0.25 * NM);
        // A single edge, so exactly one sign change along a row.
        let edges = (1..10).filter(|j| z[(0, *j)] != z[(0, j - 1)]).count();
        assert_eq!(edges, 1);
    }

    #[test]
    fn terraces_climb_monotonically_across_the_frame() {
        let z = generate(Kind::Terraces, 4, 40, sp(), 0.2 * NM);
        for j in 1..40 {
            assert!(z[(0, j)] >= z[(0, j - 1)], "staircase went down at {j}");
        }
        assert!(
            (z[(0, 39)] - 0.6 * NM).abs() < 1e-18,
            "expected three steps up"
        );
    }

    #[test]
    fn pit_is_negative_inside_and_zero_outside() {
        let z = generate(Kind::Pit, 20, 20, sp(), NM);
        assert_eq!(z[(10, 10)], -NM);
        assert_eq!(z[(0, 0)], 0.0);
    }

    #[test]
    fn bumps_peak_near_the_requested_amplitude() {
        let z = generate(Kind::Bumps, 256, 256, sp(), NM);
        let peak = z.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        // The narrowest bump carries the full amplitude; the sampling grid can
        // still miss its exact centre, so allow a little slack.
        assert!(peak > 0.9 * NM && peak <= 1.0 * NM, "peak was {peak}");
        assert!(z.iter().all(|v| *v >= 0.0));
    }

    #[test]
    fn bumps_stay_resolved_on_a_coarse_grid() {
        // The same surface on a grid too coarse for the nominal feature width.
        // `resolved_sigma` widens the bumps rather than letting them alias, so
        // the peak still lands close to the requested amplitude instead of
        // depending on where the samples happen to fall.
        let z = generate(Kind::Bumps, 32, 32, sp(), NM);
        let peak = z.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        assert!(peak > 0.9 * NM, "coarse grid aliased the bump away: {peak}");
    }

    #[test]
    fn combo_contains_a_step_a_bump_and_a_pit() {
        let z = generate(Kind::Combo, 64, 64, sp(), NM);
        let lo = z.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = z.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        // The pit floor must clear the bottom terrace, not just dent the top one.
        assert!(lo < 0.0, "no pit in the combo surface");
        assert!(hi > 2.0 * NM, "no bump above the top terrace");
        // Terrace structure survives on a row that misses the bump and pit.
        assert!(z[(0, 63)] > z[(0, 0)]);
    }
}
