//! Constant-distance scan planner.
//!
//! This is the front end for the constant-distance scan, built in three pieces.
//! Only the first exists so far; the other two have their subcommands stubbed
//! out so it is clear where they will land.
//!
//! 1. **Plan** (`plan`) — take a constant-current Z map and compute the Z the
//!    tip must hold, feedback off, to keep a fixed physical separation. Pure
//!    geometry: no hardware, fully testable offline. See
//!    [`rusty_tip::analyzer::rolling_ellipsoid`].
//! 2. **Acquire** (`acquire`) — run a constant-current scan and reconstruct a 2D
//!    Z map from the nanonis-rs TCP sample stream. Not implemented.
//! 3. **Trace** (`trace`) — drive the tip along a planned trajectory with FolMe
//!    XY moves and Z position sets, feedback off. Not implemented, and gated on
//!    hardware measurements that have not been made yet.
//!
//! Alongside those there is `baseline`, which does not use the planner at all.
//! It builds the two-pass record-and-play configuration that Nanonis multi-pass
//! runs natively (`Z_tip = Z + lift`, a plain vertical shift) and hands it to
//! the controller. That is the published method (Moreno et al., Nano Lett.
//! 2015) and the thing our trajectory has to beat at step edges, so it is worth
//! being able to run it from here.
//!
//! `drift` exposes the Z drift measurement and compensation from the action
//! layer, so they can be exercised on hardware before anything depends on
//! them: measure between scans, compensate, measure again.
//!
//! # Units
//!
//! Everything on the command line is in **nanometres**, because that is the
//! scale the numbers actually live at. Everything inside the program, and
//! everything written to a file, is in **metres**, because that is what the
//! controller and Gwyddion expect. The conversion happens once, at argument
//! parsing.

mod surface;

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use ndarray::{Array2, ArrayView2};
use textplots::{Chart, Plot, Shape};

use rusty_tip::action::drift::{CompensateDrift, MeasureZDrift};
use rusty_tip::action::multi_pass::ApplyMultiPass;
use rusty_tip::action::{Action, ActionContext, ActionOutput, DataStore};
use rusty_tip::analyzer::rolling_ellipsoid::{
    Border, GridSpacing, RollingEllipsoid, vertical_clearance,
};
use rusty_tip::event::EventBus;
use rusty_tip::export::{gsf, write_table, write_xyz};
use rusty_tip::multi_pass::MultiPassConfig;
use rusty_tip::nanonis_controller::{NanonisController, NanonisSetupConfig};
use rusty_tip::shutdown::ShutdownFlag;
use rusty_tip::signal_registry::{SignalIndex, SignalRegistry};
use rusty_tip::spm_controller::DriftComp;

/// Nanometres to metres.
const NM: f64 = 1e-9;

#[derive(Parser)]
#[command(
    name = "const-distance",
    version,
    about = "Plan and (eventually) run constant-distance scans",
    long_about = "Constant-distance scanning in three pieces: plan a tip \
        trajectory from a constant-current topograph, acquire that topograph \
        from the TCP sample stream, then trace the trajectory with feedback \
        off. Only the planner is implemented."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compute a constant-distance tip trajectory from a topograph.
    Plan(PlanArgs),
    /// Acquire a Z map from the TCP sample stream. Not implemented.
    Acquire,
    /// Drive the tip along a planned trajectory. Not implemented.
    Trace,
    /// Configure native multi-pass for a constant-lift baseline scan.
    Baseline(BaselineArgs),
    /// Measure the Z drift, or set the piezo compensation that cancels it.
    Drift(DriftArgs),
}

#[derive(Args)]
struct DriftArgs {
    #[command(subcommand)]
    op: DriftOp,

    /// RT signal slot carrying the Z position, in metres. 30 on a stock
    /// signal assignment; `--signal-name` resolves it from the controller.
    #[arg(long, default_value_t = 30)]
    signal: u32,

    /// Resolve the Z signal by name instead of by slot, e.g. "Z (m)".
    #[arg(long, conflicts_with = "signal")]
    signal_name: Option<String>,

    /// Length of one measurement window, in seconds. `compensate` runs three.
    #[arg(long, default_value_t = 5.0)]
    window: f64,

    /// Samples per window. At least 3.
    #[arg(long, default_value_t = 16)]
    samples: usize,

    /// Controller address.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Controller port.
    #[arg(long, default_value_t = 6501)]
    port: u16,
}

#[derive(Subcommand)]
enum DriftOp {
    /// Print the compensation velocities and which axes have saturated.
    Status,
    /// Fit a Z drift rate and print it. Changes nothing on the controller.
    Measure,
    /// Measure, set the Z velocity that cancels the drift, and report the
    /// residual. Leaves compensation switched on.
    Compensate,
    /// Switch compensation off. The velocities are kept, so `status` still
    /// shows what was last applied.
    Off,
}

#[derive(Args)]
struct BaselineArgs {
    /// Lift for the second pass, in metres (0.5e-9 for half a nanometre).
    /// Positive is the direction the `Play offset` field takes; which way
    /// that points on hardware has NOT been confirmed yet, so check against
    /// the GUI before trusting the sign.
    #[arg(short, long, default_value_t = 2e-10)]
    lift: f64,

    /// RT signal slot to record in the first pass. 30 is Z (m) on a stock
    /// signal assignment; `--signal-name` resolves it from the controller
    /// instead.
    #[arg(long, default_value_t = 30)]
    signal: u32,

    /// Resolve the recorded signal by name instead of by slot, e.g. "Z (m)".
    /// Needs a connection.
    #[arg(long, conflicts_with = "signal")]
    signal_name: Option<String>,

    /// Where to write the .mpas file.
    #[arg(short, long, default_value = "baseline.mpas")]
    output: PathBuf,

    /// Path the *controller* should load the file from. Defaults to `--output`,
    /// which is right only when Nanonis runs on this machine; on a real
    /// instrument the file has to be somewhere that PC can see.
    #[arg(long)]
    host_path: Option<String>,

    /// Write the file and stop, without connecting to anything.
    #[arg(long)]
    dry_run: bool,

    /// Controller address.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Controller port.
    #[arg(long, default_value_t = 6501)]
    port: u16,

    /// Settle time at the beginning of each line, in seconds.
    #[arg(long, default_value_t = 0.0)]
    delay: f64,
}

#[derive(Args)]
struct PlanArgs {
    /// Input height map: a .gsf file, or a whitespace-separated ASCII grid.
    #[arg(short, long, group = "source")]
    input: Option<PathBuf>,

    /// Generate a synthetic test surface instead of reading one.
    #[arg(long, group = "source", value_enum)]
    synthetic: Option<surface::Kind>,

    /// Unit of the values in an ASCII input file.
    #[arg(long, value_enum, default_value_t = InputUnit::M)]
    input_unit: InputUnit,

    /// Sample spacing along the fast axis, metres. Read from the file for .gsf.
    #[arg(long, default_value_t = 1e-10)]
    dx: f64,

    /// Sample spacing along the slow axis, metres. Defaults to --dx.
    #[arg(long)]
    dy: Option<f64>,

    /// Samples per line, synthetic surfaces only.
    #[arg(long, default_value_t = 256)]
    nx: usize,

    /// Number of lines, synthetic surfaces only.
    #[arg(long, default_value_t = 256)]
    ny: usize,

    /// Characteristic feature height, metres, synthetic surfaces only.
    #[arg(long, default_value_t = 3e-10)]
    amplitude: f64,

    /// Lateral semi-axis of the tip ellipsoid, metres.
    #[arg(short = 'a', long, default_value_t = 1e-9)]
    lateral: f64,

    /// Lateral semi-axis along the slow axis, metres. Defaults to --lateral.
    #[arg(long)]
    lateral_y: Option<f64>,

    /// Vertical semi-axis of the tip ellipsoid, metres. Smaller means blunter.
    #[arg(short = 'c', long, default_value_t = 5e-10)]
    vertical: f64,

    /// How to treat the footprint where it overhangs the frame edge.
    #[arg(long, value_enum, default_value_t = BorderArg::Replicate)]
    border: BorderArg,

    /// Directory to write the results into. Created if missing.
    #[arg(short, long, default_value = "const-distance-out")]
    out_dir: PathBuf,

    /// Row to print as an ASCII cross-section. Defaults to the middle row.
    #[arg(long)]
    profile: Option<usize>,

    /// Skip the ASCII point clouds, which are large and slow to write.
    #[arg(long)]
    no_xyz: bool,
}

/// Unit of the values in an ASCII input file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum InputUnit {
    /// Metres, as the controller reports Z.
    M,
    /// Nanometres.
    Nm,
}

/// CLI mirror of [`Border`], so the enum stays free of clap derives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BorderArg {
    /// Ignore out-of-frame samples. Honest, but the tip may dive at the edge.
    Truncate,
    /// Assume the surface continues at its border height. Conservative.
    Replicate,
}

impl From<BorderArg> for Border {
    fn from(b: BorderArg) -> Self {
        match b {
            BorderArg::Truncate => Border::Truncate,
            BorderArg::Replicate => Border::Replicate,
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Plan(args) => plan(args),
        Command::Acquire => Err("`acquire` is piece 2 and is not implemented yet: \
             mapping the nanonis-rs TCP sample stream into scan lines has not \
             been designed. Use `plan --synthetic combo` in the meantime."
            .into()),
        Command::Trace => Err("`trace` is piece 3 and is not implemented yet: it \
             is gated on hardware measurements (TCP latency, feedback-off \
             sequencing, Z step response, FolMe blocking behaviour) that have \
             not been made."
            .into()),
        Command::Baseline(args) => baseline(args),
        Command::Drift(args) => drift(args),
    }
}

/// Connect and run one drift operation, printing velocities in pm/s.
///
/// Like `baseline`, this talks to the action layer directly rather than
/// through a routine: a routine withdraws the tip when it ends, and a drift
/// measurement is something done *between* passes with the tip engaged.
fn drift(args: DriftArgs) -> Result<(), Box<dyn Error>> {
    let client = rusty_tip::NanonisClient::builder()
        .address(&args.host)
        .port(args.port)
        .build()?;
    println!("connected to {}:{}", args.host, args.port);
    let mut controller = NanonisController::new(client, NanonisSetupConfig::default());

    let z = match &args.signal_name {
        Some(name) => {
            let registry = SignalRegistry::from_controller(&mut controller)?;
            let signal = registry
                .get_by_name(name)
                .ok_or_else(|| format!("no signal called {name:?} on this controller"))?
                .signal_index();
            println!("resolved {name:?} to RT slot {}", signal.0);
            signal
        }
        None => SignalIndex(args.signal),
    };

    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut store = DataStore::new();
    let mut ctx = ActionContext {
        controller: &mut controller,
        store: &mut store,
        events: &events,
        shutdown: &shutdown,
    };
    let window_ms = (args.window * 1000.0) as u64;

    let print_status = |comp: &DriftComp| {
        println!(
            "compensation {}: vx {:.3} pm/s, vy {:.3} pm/s, vz {:.3} pm/s",
            if comp.enabled { "on" } else { "off" },
            comp.vx / PM,
            comp.vy / PM,
            comp.vz / PM
        );
        let saturated: Vec<&str> = [
            (comp.x_saturated, "x"),
            (comp.y_saturated, "y"),
            (comp.z_saturated, "z"),
        ]
        .into_iter()
        .filter_map(|(s, axis)| s.then_some(axis))
        .collect();
        match saturated.is_empty() {
            true => println!(
                "no axis saturated (limit {}% of range)",
                comp.saturation_limit_percent
            ),
            false => println!(
                "SATURATED on {}: compensation on that axis has stopped and only \
                 an off/on cycle restarts it",
                saturated.join(", ")
            ),
        }
    };

    match args.op {
        DriftOp::Status => {
            print_status(&ctx.controller.drift_comp_get()?);
        }
        DriftOp::Measure => {
            print_status(&ctx.controller.drift_comp_get()?);
            println!(
                "measuring over {:.1} s with {} samples (Z controller must be on, scan stopped)",
                args.window, args.samples
            );
            let output = MeasureZDrift {
                z,
                window_ms,
                samples: args.samples,
            }
            .execute(&mut ctx)?;
            println!("Z drift: {:.3} pm/s", value_of(output)? / PM);
        }
        DriftOp::Compensate => {
            print_status(&ctx.controller.drift_comp_get()?);
            println!(
                "compensating: three windows of {:.1} s, {} samples each",
                args.window, args.samples
            );
            let output = CompensateDrift {
                z,
                window_ms,
                samples: args.samples,
            }
            .execute(&mut ctx)?;
            println!("residual Z drift: {:.3} pm/s", value_of(output)? / PM);
            print_status(&ctx.controller.drift_comp_get()?);
        }
        DriftOp::Off => {
            let comp = ctx.controller.drift_comp_get()?;
            ctx.controller.drift_comp_set(&DriftComp {
                enabled: false,
                ..comp
            })?;
            print_status(&ctx.controller.drift_comp_get()?);
        }
    }
    Ok(())
}

/// Picometres to metres.
const PM: f64 = 1e-12;

fn value_of(output: ActionOutput) -> Result<f64, Box<dyn Error>> {
    match output {
        ActionOutput::Value(v) => Ok(v),
        other => Err(format!("expected a value, got {other:?}").into()),
    }
}

/// Build the constant-lift multi-pass configuration and, unless `--dry-run`,
/// load and activate it on the controller.
fn baseline(args: BaselineArgs) -> Result<(), Box<dyn Error>> {
    let host_path = args
        .host_path
        .clone()
        .unwrap_or_else(|| args.output.to_string_lossy().into_owned());

    // Connect first when the signal has to be resolved by name, so a typo fails
    // before anything is written.
    let mut controller = match args.dry_run {
        true => None,
        false => {
            let client = rusty_tip::NanonisClient::builder()
                .address(&args.host)
                .port(args.port)
                .build()?;
            println!("connected to {}:{}", args.host, args.port);
            Some(NanonisController::new(
                client,
                NanonisSetupConfig::default(),
            ))
        }
    };

    let signal = match (&args.signal_name, controller.as_mut()) {
        (Some(name), Some(c)) => {
            // Through the registry rather than a name scan of our own, so the
            // aliases and cleaned names it knows about work here too.
            let registry = SignalRegistry::from_controller(c)?;
            let signal = registry
                .get_by_name(name)
                .ok_or_else(|| format!("no signal called {name:?} on this controller"))?
                .signal_index();
            println!("resolved {name:?} to RT slot {}", signal.0);
            signal
        }
        (Some(_), None) => return Err("--signal-name needs a connection; drop --dry-run".into()),
        (None, _) => SignalIndex(args.signal),
    };

    // A lift beyond the piezo range is a unit slip, not a plan: `--lift 0.5`
    // is half a metre.
    if args.lift.abs() > 100e-9 {
        return Err(format!(
            "--lift {} m is more than 100 nm; it takes metres, so half a nanometre is 0.5e-9",
            args.lift
        )
        .into());
    }

    let mut config = MultiPassConfig::constant_lift(signal, args.lift);
    for pass in &mut config.passes {
        pass.delay = args.delay;
    }

    // Print the sections the way the Multi Pass window lists them, so this can
    // be checked against the GUI at a glance.
    for (i, pass) in config.passes.iter().enumerate() {
        let what = match (pass.recorded(), pass.played()) {
            (Some(s), _) => format!("record RT slot {}", s.0),
            (_, Some((offset, _))) => format!("play back, offset {} nm", offset / NM),
            _ => "nothing".to_string(),
        };
        println!("  {}: {what}", MultiPassConfig::label(i));
    }

    for i in config.unsourced_playbacks() {
        println!(
            "  warning: {} plays back, but nothing is recorded in that direction",
            MultiPassConfig::label(i)
        );
    }

    let Some(mut controller) = controller else {
        config.write(&args.output)?;
        println!("wrote {} (dry run, nothing sent)", args.output.display());
        return Ok(());
    };

    // Straight to the action rather than through `Rt`: the harness withdraws
    // the tip when a routine ends, which is right for a routine and wrong for
    // a command that only rewrites a configuration.
    let events = EventBus::new();
    let shutdown = ShutdownFlag::new();
    let mut store = DataStore::new();
    let mut ctx = ActionContext {
        controller: &mut controller,
        store: &mut store,
        events: &events,
        shutdown: &shutdown,
    };

    ApplyMultiPass {
        config: config.clone(),
        local_path: args.output.clone(),
        host_path: host_path.clone(),
    }
    .execute(&mut ctx)?;

    let buffer = ctx.controller.scan_buffer_get()?;
    println!(
        "scan buffer records RT slots {:?} at {}x{}",
        buffer.channels.iter().map(|c| c.0).collect::<Vec<_>>(),
        buffer.pixels,
        buffer.lines
    );
    println!("wrote {}", args.output.display());
    println!("loaded from {host_path} and activated multi-pass");
    println!(
        "note: Linefeed scan mode is not part of the file and cannot be set \
         over TCP. Tick it by hand in Scan Control, or the passes will not \
         land on the same line."
    );
    println!("unconfirmed: the offset sign (does positive approach or retract?).");
    Ok(())
}

fn plan(args: PlanArgs) -> Result<(), Box<dyn Error>> {
    let (z, sp) = load_input(&args)?;
    let (ny, nx) = z.dim();

    let tip_model = RollingEllipsoid::new(
        args.lateral,
        args.lateral_y.unwrap_or(args.lateral),
        args.vertical,
    );
    let border: Border = args.border.into();

    let (ry, rx) = tip_model.radii(sp);
    println!(
        "surface   {nx} x {ny} samples, {:.3} x {:.3} nm",
        nx as f64 * sp.dx / NM,
        ny as f64 * sp.dy / NM
    );
    println!(
        "spacing   dx = {:.4} nm, dy = {:.4} nm",
        sp.dx / NM,
        sp.dy / NM
    );
    println!(
        "tip       a_x = {:.3} nm, a_y = {:.3} nm, c = {:.3} nm",
        tip_model.a_x / NM,
        tip_model.a_y / NM,
        tip_model.c / NM
    );
    println!(
        "footprint {} x {} samples, border policy {:?}",
        2 * ry + 1,
        2 * rx + 1,
        border
    );

    // A footprint narrower than a few samples silently returns the input
    // unchanged, which looks like a result. Say so before it wastes an hour.
    if !tip_model.is_resolved(sp, 5) {
        eprintln!(
            "warning: the ellipsoid spans fewer than 5 samples on at least one \
             axis, so the plan will be close to a copy of the input. Increase \
             --lateral or scan with finer sampling."
        );
    }

    let z_tip = tip_model.tip_trajectory(z.view(), sp, border);
    let clearance = vertical_clearance(z.view(), z_tip.view());
    let plan = Plan {
        stats: ClearanceStats::of(clearance.view()),
        z,
        z_tip,
        clearance,
        sp,
    };
    report(plan.stats);

    let row = args.profile.unwrap_or(ny / 2).min(ny - 1);
    plot_profile(plan.z.view(), plan.z_tip.view(), plan.sp, row);

    write_outputs(&args, &plan, &tip_model, border)?;
    Ok(())
}

/// Read the input height map, or generate a synthetic one.
///
/// Returns the map in metres together with its sample spacing. A `.gsf` file
/// carries its own spacing, and that wins over `--dx`/`--dy`; everything else
/// has to be told.
fn load_input(args: &PlanArgs) -> Result<(Array2<f64>, GridSpacing), Box<dyn Error>> {
    let cli_spacing = GridSpacing {
        dx: args.dx,
        dy: args.dy.unwrap_or(args.dx),
    };

    if let Some(kind) = args.synthetic {
        let z = surface::generate(kind, args.ny, args.nx, cli_spacing, args.amplitude);
        return Ok((z, cli_spacing));
    }

    let path = args
        .input
        .as_ref()
        .ok_or("give either --input <FILE> or --synthetic <KIND>")?;

    if path.extension().and_then(|e| e.to_str()) == Some("gsf") {
        let field = gsf::read_gsf(path)?;
        let sp = field.spacing();
        println!(
            "read {} ({} units)",
            path.display(),
            if field.z_units.is_empty() {
                "no"
            } else {
                &field.z_units
            }
        );
        return Ok((field.data, sp));
    }

    let z = read_ascii_grid(path, args.input_unit)?;
    Ok((z, cli_spacing))
}

/// Parse a whitespace-separated ASCII grid: one line per scan line.
///
/// Blank lines and `#` comments are skipped, so a file exported by another
/// tool with a header usually just works. Every row must have the same length;
/// a ragged file is a corrupted file, not something to pad silently.
fn read_ascii_grid(path: &Path, unit: InputUnit) -> Result<Array2<f64>, Box<dyn Error>> {
    let scale = match unit {
        InputUnit::M => 1.0,
        InputUnit::Nm => NM,
    };

    let text = fs::read_to_string(path)?;
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let row: Result<Vec<f64>, _> = line
            .split_whitespace()
            .map(|t| t.parse::<f64>().map(|v| v * scale))
            .collect();
        let row = row.map_err(|e| format!("{}: line {}: {e}", path.display(), n + 1))?;
        rows.push(row);
    }

    let nx = rows.first().map(Vec::len).unwrap_or(0);
    if nx == 0 {
        return Err(format!("{}: no data", path.display()).into());
    }
    if let Some(bad) = rows.iter().position(|r| r.len() != nx) {
        return Err(format!(
            "{}: row {} has {} values, expected {nx}",
            path.display(),
            bad + 1,
            rows[bad].len()
        )
        .into());
    }

    let ny = rows.len();
    Ok(Array2::from_shape_vec(
        (ny, nx),
        rows.into_iter().flatten().collect(),
    )?)
}

/// A finished plan: the surface it was computed from, the trajectory, and the
/// clearance between them, all on one grid.
///
/// Bundled because they only ever travel together, and because keeping the
/// stats beside the maps they summarise means the printed report and the
/// recorded metadata cannot disagree.
struct Plan {
    z: Array2<f64>,
    z_tip: Array2<f64>,
    clearance: Array2<f64>,
    stats: ClearanceStats,
    sp: GridSpacing,
}

/// Summary of a clearance map, computed once and shared by the terminal
/// report and the recorded metadata so the two cannot drift apart.
#[derive(Debug, Clone, Copy)]
struct ClearanceStats {
    min: f64,
    max: f64,
    mean: f64,
    /// Fraction of samples the plan lifted by more than 1 pm.
    lifted: f64,
}

impl ClearanceStats {
    fn of(clearance: ArrayView2<f64>) -> Self {
        let n = clearance.len() as f64;
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut sum = 0.0;
        let mut lifted = 0usize;
        for v in clearance.iter() {
            min = min.min(*v);
            max = max.max(*v);
            sum += v;
            // 1 pm is well under anything the Z axis resolves, so treat
            // anything below it as "the plan did not move the tip here".
            if *v > 1e-12 {
                lifted += 1;
            }
        }
        Self {
            min,
            max,
            mean: sum / n,
            lifted: lifted as f64 / n,
        }
    }
}

/// Print how far the plan lifts the tip off the surface.
fn report(stats: ClearanceStats) {
    let ClearanceStats {
        min,
        max,
        mean,
        lifted,
    } = stats;

    println!("\nvertical clearance (planned Z minus surface Z)");
    // Clamped in the planner, so a negative value here means the invariant
    // itself broke, not that a terrace rounded the wrong way.
    println!(
        "  min  {:>9.4} nm   (zero on flat ground; never negative)",
        min / NM
    );
    println!("  mean {:>9.4} nm", mean / NM);
    println!("  max  {:>9.4} nm", max / NM);
    println!(
        "  {:.1}% of samples lifted by more than 1 pm",
        lifted * 100.0
    );
}

/// Draw the surface and the planned trajectory over one scan line.
///
/// The terminal plot is not the deliverable, the exported files are. It is here
/// because a wrong plan is usually obvious in a single cross-section, and
/// noticing that before opening Gwyddion saves a round trip.
fn plot_profile(z: ArrayView2<f64>, z_tip: ArrayView2<f64>, sp: GridSpacing, row: usize) {
    let series = |m: ArrayView2<f64>| -> Vec<(f32, f32)> {
        m.row(row)
            .iter()
            .enumerate()
            .map(|(j, v)| (((j as f64 + 0.5) * sp.dx / NM) as f32, (v / NM) as f32))
            .collect()
    };
    let surface = series(z);
    let tip = series(z_tip);
    let x_max = surface.last().map(|p| p.0).unwrap_or(1.0);

    println!("\nrow {row}: surface (lower trace) and planned tip path, nm vs nm");
    Chart::new(160, 50, 0.0, x_max)
        .lineplot(&Shape::Lines(&surface))
        .lineplot(&Shape::Lines(&tip))
        .display();
}

/// Write every output file, plus a README explaining what they are.
fn write_outputs(
    args: &PlanArgs,
    plan: &Plan,
    tip_model: &RollingEllipsoid,
    border: Border,
) -> Result<(), Box<dyn Error>> {
    let Plan {
        z,
        z_tip,
        clearance,
        stats,
        sp,
    } = plan;
    let sp = *sp;
    let dir = &args.out_dir;
    fs::create_dir_all(dir)?;

    gsf::write_map(
        dir.join("surface.gsf"),
        z.view(),
        sp,
        "Z surface (const current)",
    )?;
    gsf::write_map(
        dir.join("tip.gsf"),
        z_tip.view(),
        sp,
        "Z tip (const distance)",
    )?;
    gsf::write_map(
        dir.join("clearance.gsf"),
        clearance.view(),
        sp,
        "Vertical clearance",
    )?;

    // One table with everything co-registered: this is the file to plot when
    // the question is "how do the two clouds differ", because the difference is
    // already a column rather than something to recompute by joining files.
    write_table(
        dir.join("compare.dat"),
        sp,
        &[
            ("z_surface_m", z.view()),
            ("z_tip_m", z_tip.view()),
            ("clearance_m", clearance.view()),
        ],
    )?;

    if !args.no_xyz {
        write_xyz(dir.join("surface.xyz"), z.view(), sp)?;
        write_xyz(dir.join("tip.xyz"), z_tip.view(), sp)?;
    }

    let (ny, nx) = z.dim();
    let meta = serde_json::json!({
        "samples": { "nx": nx, "ny": ny },
        "spacing_m": { "dx": sp.dx, "dy": sp.dy },
        "ellipsoid_m": { "a_x": tip_model.a_x, "a_y": tip_model.a_y, "c": tip_model.c },
        "border": format!("{border:?}"),
        "source": match args.synthetic {
            Some(kind) => format!("synthetic:{kind:?}"),
            None => args.input.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
        },
        "clearance_m": {
            "min": stats.min,
            "max": stats.max,
            "mean": stats.mean,
        },
    });
    fs::write(dir.join("plan.json"), serde_json::to_string_pretty(&meta)?)?;
    fs::write(dir.join("README.txt"), readme(args.no_xyz))?;

    println!("\nwrote {}/", dir.display());
    for name in output_names(args.no_xyz) {
        println!("  {name}");
    }
    Ok(())
}

fn output_names(no_xyz: bool) -> Vec<&'static str> {
    let mut names = vec!["surface.gsf", "tip.gsf", "clearance.gsf", "compare.dat"];
    if !no_xyz {
        names.push("surface.xyz");
        names.push("tip.xyz");
    }
    names.push("plan.json");
    names.push("README.txt");
    names
}

fn readme(no_xyz: bool) -> String {
    let xyz = if no_xyz {
        "  (point clouds skipped: --no-xyz was given)\n"
    } else {
        "  surface.xyz     ASCII point cloud of the surface, columns: x y z, metres\n\
         \x20 tip.xyz         ASCII point cloud of the planned trajectory, same columns\n"
    };
    format!(
        "Constant-distance scan plan\n\
         ===========================\n\n\
         All lengths are in metres. Maps are row-major: row = slow scan axis,\n\
         column = fast scan axis, first row at the top.\n\n\
         Files\n\
         -----\n\
         \x20 surface.gsf     input topograph (constant current) as a Gwyddion field\n\
         \x20 tip.gsf         planned tip trajectory (constant distance)\n\
         \x20 clearance.gsf   tip.gsf minus surface.gsf; where and how far the plan lifts\n\
         \x20 compare.dat     all three co-registered, one line per sample:\n\
         \x20                 x_m y_m z_surface_m z_tip_m clearance_m\n\
         {xyz}\
         \x20 plan.json       the parameters this plan was computed with\n\n\
         Viewing\n\
         -------\n\
         Gwyddion:  gwyddion surface.gsf tip.gsf clearance.gsf\n\
         \x20          The .gsf files carry their own dimensions and units, so no\n\
         \x20          calibration needs entering. Data > Arithmetic subtracts two.\n\
         \x20          The .xyz files import via File > Open with the XYZ module.\n\n\
         gnuplot:   plot 'compare.dat' u 1:3 w l title 'surface', '' u 1:4 w l title 'tip'\n\
         \x20          (that plots every row on top of each other; add\n\
         \x20          `every ::0::NX-1` to isolate the first scan line)\n\n\
         numpy:     x, y, zs, zt, gap = np.loadtxt('compare.dat', unpack=True)\n\n\
         What to look for\n\
         ----------------\n\
         \x20 - clearance must never be negative: the tip never below the surface\n\
         \x20 - clearance must be zero on flat terraces: this plans a constant\n\
         \x20   distance, not a constant lift\n\
         \x20 - at a step edge the trajectory should start rising up to one\n\
         \x20   lateral semi-axis before the edge, and stay high the same\n\
         \x20   distance past it\n"
    )
}
