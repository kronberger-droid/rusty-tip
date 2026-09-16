//! End-to-end checks for the constant-distance planner and its exports.
//!
//! The unit tests inside `rolling_ellipsoid` and `export` pin down each piece
//! on its own. These go through the public API the way the `const-distance`
//! binary does: build a height map, plan a trajectory, write it out, read it
//! back, and confirm the numbers survived the round trip with their physical
//! dimensions attached.

use std::fs;

use ndarray::Array2;

use rusty_tip::analyzer::rolling_ellipsoid::{
    Border, GridSpacing, RollingEllipsoid, vertical_clearance,
};
use rusty_tip::export::{gsf, write_table, write_xyz};

const NM: f64 = 1e-9;

fn scratch(name: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("rusty-tip-plan-{name}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A staircase of three terraces along the fast axis, 0.25 nm per step.
fn staircase(ny: usize, nx: usize) -> Array2<f64> {
    Array2::from_shape_fn((ny, nx), |(_, j)| (j * 3 / nx) as f64 * 0.25 * NM)
}

#[test]
fn a_planned_trajectory_survives_a_gsf_round_trip() {
    let dir = scratch("gsf");
    let sp = GridSpacing::square(0.1 * NM);
    let z = staircase(32, 96);

    let tip_model = RollingEllipsoid::isotropic(1.0 * NM, 0.4 * NM);
    let z_tip = tip_model.tip_trajectory(z.view(), sp, Border::Replicate);

    gsf::write_map(dir.join("tip.gsf"), z_tip.view(), sp, "Z tip").unwrap();
    let back = gsf::read_gsf(dir.join("tip.gsf")).unwrap();

    assert_eq!(back.data.dim(), (32, 96));
    // The physical extent, not just the pixel count, comes back: this is the
    // whole reason for exporting GSF rather than a bare image.
    assert!((back.spacing().dx - sp.dx).abs() < 1e-16);
    assert!((back.spacing().dy - sp.dy).abs() < 1e-16);
    assert_eq!(back.xy_units, "m");
    assert_eq!(back.z_units, "m");

    // f32 payload, and heights of order 1e-10, so compare relatively.
    for (got, want) in back.data.iter().zip(z_tip.iter()) {
        assert!((got - want).abs() <= want.abs() * 1e-6 + 1e-18);
    }

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_comparison_table_lines_up_with_the_maps_it_came_from() {
    let dir = scratch("table");
    let sp = GridSpacing::square(0.1 * NM);
    let z = staircase(8, 24);

    let tip_model = RollingEllipsoid::isotropic(0.8 * NM, 0.3 * NM);
    let z_tip = tip_model.tip_trajectory(z.view(), sp, Border::Replicate);
    let gap = vertical_clearance(z.view(), z_tip.view());

    let path = dir.join("compare.dat");
    write_table(
        &path,
        sp,
        &[
            ("z_surface_m", z.view()),
            ("z_tip_m", z_tip.view()),
            ("clearance_m", gap.view()),
        ],
    )
    .unwrap();

    let text = fs::read_to_string(&path).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap(),
        "# x_m y_m z_surface_m z_tip_m clearance_m"
    );

    let rows: Vec<Vec<f64>> = lines
        .map(|l| l.split_whitespace().map(|t| t.parse().unwrap()).collect())
        .collect();
    assert_eq!(rows.len(), 8 * 24, "one line per sample");

    for (n, row) in rows.iter().enumerate() {
        let (i, j) = (n / 24, n % 24);
        assert!(
            (row[0] - (j as f64 + 0.5) * sp.dx).abs() < 1e-18,
            "x wrong at {n}"
        );
        assert!(
            (row[1] - (i as f64 + 0.5) * sp.dy).abs() < 1e-18,
            "y wrong at {n}"
        );
        assert!((row[2] - z[(i, j)]).abs() < 1e-18, "surface wrong at {n}");
        assert!(
            (row[3] - z_tip[(i, j)]).abs() < 1e-18,
            "trajectory wrong at {n}"
        );
        // The clearance column must be the difference of the other two, not an
        // independently computed number that could drift out of step.
        assert!(
            (row[4] - (row[3] - row[2])).abs() < 1e-18,
            "clearance wrong at {n}"
        );
    }

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_two_point_clouds_share_a_coordinate_grid() {
    // The point of exporting both clouds is to overlay them. If the coordinate
    // columns ever diverged, the comparison would be between samples that are
    // not at the same place, and the difference would look like structure.
    let dir = scratch("clouds");
    let sp = GridSpacing {
        dx: 0.1 * NM,
        dy: 0.25 * NM,
    };
    let z = staircase(6, 12);
    let z_tip = RollingEllipsoid::isotropic(0.5 * NM, 0.2 * NM).tip_trajectory(
        z.view(),
        sp,
        Border::Replicate,
    );

    write_xyz(dir.join("surface.xyz"), z.view(), sp).unwrap();
    write_xyz(dir.join("tip.xyz"), z_tip.view(), sp).unwrap();

    let coords = |name: &str| -> Vec<(String, String)> {
        fs::read_to_string(dir.join(name))
            .unwrap()
            .lines()
            .map(|l| {
                let mut t = l.split_whitespace();
                (t.next().unwrap().to_string(), t.next().unwrap().to_string())
            })
            .collect()
    };
    assert_eq!(coords("surface.xyz"), coords("tip.xyz"));

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn planning_a_staircase_behaves_the_way_the_geometry_says_it_should() {
    // The properties worth restating at the integration level, because they are
    // what makes the exported files trustworthy rather than merely well-formed.
    let sp = GridSpacing::square(0.1 * NM);
    let z = staircase(16, 90);
    let a = 1.0 * NM;
    let z_tip =
        RollingEllipsoid::isotropic(a, 0.4 * NM).tip_trajectory(z.view(), sp, Border::Replicate);
    let gap = vertical_clearance(z.view(), z_tip.view());

    // Never below the surface, anywhere.
    assert!(gap.iter().all(|g| *g >= 0.0));

    // Zero on flat ground: this is a constant *distance* plan, not a lift.
    let reach = (a / sp.dx).ceil() as usize;
    let edge = 30; // first terrace boundary, at j = nx/3
    for j in 0..(edge - reach) {
        assert!(gap[(8, j)] == 0.0, "lifted on a terrace at column {j}");
    }

    // Lifted approaching the edge, and back at the surface once on the terrace.
    assert!(gap[(8, edge - 2)] > 0.0, "did not anticipate the step");
    assert!((z_tip[(8, edge)] - z[(8, edge)]).abs() < 1e-20);

    // The lift never exceeds one step height: the plan clears the step it is
    // approaching, it does not stack up clearance from every step in the frame.
    let max_lift = gap.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    assert!(max_lift <= 0.25 * NM + 1e-20, "lifted by {max_lift} m");
}

#[test]
fn a_blunt_tip_cannot_reach_into_a_narrow_pit() {
    // The physical claim that makes constant-distance scanning worth planning
    // at all: a finite tip cannot follow the surface into a feature narrower
    // than itself, and the plan should say so rather than pretending otherwise.
    let sp = GridSpacing::square(0.1 * NM);
    let depth = 1.0 * NM;
    let mut z = Array2::zeros((1, 60));
    for j in 28..32 {
        z[(0, j)] = -depth; // 0.4 nm wide, 1 nm deep
    }

    // Lateral semi-axis 1 nm: far wider than the pit mouth.
    let z_tip = RollingEllipsoid::isotropic(1.0 * NM, 0.3 * NM).tip_trajectory(
        z.view(),
        sp,
        Border::Replicate,
    );

    // The apex barely descends into the pit, nowhere near its floor.
    let floor = z_tip[(0, 30)];
    assert!(
        floor > -0.2 * depth,
        "a 1 nm tip reached {floor} m into a 0.4 nm wide pit"
    );
    // And it is still above the surface everywhere, as always.
    assert!(
        vertical_clearance(z.view(), z_tip.view())
            .iter()
            .all(|g| *g >= 0.0)
    );
}
