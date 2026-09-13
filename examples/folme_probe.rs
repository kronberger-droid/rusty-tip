//! Measure the numbers a FolMe-driven constant-height trace depends on.
//!
//! Piece 3 of the constant-distance scan (`const-distance trace`) is gated on
//! hardware facts nobody has measured yet: how long a TCP round trip takes,
//! what happens in Z when the feedback is switched off, how Z responds to a
//! step with the loop open, and whether a FolMe move with the wait flag really
//! blocks until arrival. This example measures them, one probe at a time,
//! against the bare `NanonisClient`. Nothing here goes through rusty-tip's
//! controller or routine layers, on purpose: the point is to learn what the
//! instrument does before building on it.
//!
//! ```text
//! cargo run --example folme_probe -- --host 192.168.1.10 latency
//! cargo run --example folme_probe -- feedback-off
//! cargo run --example folme_probe -- --lift-nm 5 z-steps
//! cargo run --example folme_probe -- --folme-speed-nm-s 20 folme
//! cargo run --example folme_probe -- all
//! ```
//!
//! # What keeps the tip safe here
//!
//! Every Z move with the loop open goes in the *retract* direction first, and
//! the retract direction is measured rather than assumed: switching the
//! feedback off applies the controller's TipLift, and the sign of that jump
//! says which way "away from the surface" is in Z. With TipLift at zero there
//! is nothing to measure, and `z-steps` refuses to run unless `--z-sign` says
//! so explicitly. After the first lift the frequency shift is checked as well:
//! farther from the surface it must be smaller in magnitude than at the
//! setpoint, and if it is not, Z goes straight back and the probe aborts.
//! The steps themselves are always taken *up* from the lifted position and
//! never below the point where the loop opened.
//!
//! The FolMe probe keeps the feedback closed throughout, so it is an ordinary
//! constant-current move and needs no lift at all.
//!
//! Start with the tip engaged on a flat, quiet spot. Run `latency` and
//! `feedback-off` first; they change nothing you cannot undo with the Z
//! controller's on/off button.

use std::io;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};

use rusty_tip::{NanonisClient, Position};

const NM: f64 = 1e-9;
const PM: f64 = 1e-12;

#[derive(Parser)]
#[command(about = "Measure what a FolMe constant-height trace depends on")]
struct Cli {
    /// Controller address.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Controller port.
    #[arg(long, default_value_t = 6501)]
    port: u16,

    /// How far to retract, in nanometres, before any open-loop Z step. At 5 nm
    /// the tip is out of range of every short-range interaction.
    #[arg(long, default_value_t = 5.0)]
    lift_nm: f64,

    /// Which way "away from the surface" points in Z: +1 if increasing Z
    /// retracts, -1 if decreasing Z does. Measured from TipLift when omitted.
    #[arg(long, allow_hyphen_values = true)]
    z_sign: Option<i8>,

    /// RT slot of the frequency shift signal. Found by name when omitted.
    #[arg(long)]
    df_signal: Option<u8>,

    /// FolMe speed for the waypoint replay, in nm/s.
    #[arg(long, default_value_t = 20.0)]
    folme_speed_nm_s: f64,

    /// Length of the FolMe replay line, in nanometres.
    #[arg(long, default_value_t = 5.0)]
    line_nm: f64,

    /// Waypoints along the FolMe replay line, one way.
    #[arg(long, default_value_t = 64)]
    waypoints: usize,

    /// Skip the confirmation prompt.
    #[arg(long)]
    yes: bool,

    /// Which probes to run, in order.
    #[arg(required = true, value_enum)]
    probes: Vec<Probe>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Probe {
    /// Time 100 reads each of Z, XY and the frequency shift. Touches nothing.
    Latency,
    /// Open the loop, time how long the controller takes to report it off,
    /// and measure the TipLift jump. Closes the loop again afterwards.
    FeedbackOff,
    /// Open the loop, retract by --lift-nm, step Z by 10 pm, 100 pm and 1 nm
    /// (each further away), watching the frequency shift. Restores Z and
    /// closes the loop.
    ZSteps,
    /// Feedback closed: replay a line of waypoints with the wait flag set,
    /// timing every move. Restores position and FolMe speed.
    Folme,
    /// All of the above, in that order.
    All,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let probes: Vec<Probe> = if cli.probes.contains(&Probe::All) {
        vec![
            Probe::Latency,
            Probe::FeedbackOff,
            Probe::ZSteps,
            Probe::Folme,
        ]
    } else {
        cli.probes.clone()
    };

    let mut c = NanonisClient::builder()
        .address(&cli.host)
        .port(cli.port)
        .build()?;
    println!("connected to {}:{}", cli.host, cli.port);

    let df = match cli.df_signal {
        Some(slot) => slot,
        None => find_freq_shift(&mut c)?,
    };
    survey(&mut c, df)?;

    println!("\nprobes: {probes:?}");
    println!(
        "lift {} nm, z-sign {}",
        cli.lift_nm,
        match cli.z_sign {
            Some(s) => format!("{s:+} (given)"),
            None => "measured from TipLift".to_string(),
        }
    );
    if !cli.yes {
        println!("\nPress Enter to run, Ctrl+C to abort...");
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
    }

    let mut measured_sign: Option<f64> = None;
    for probe in probes {
        println!("\n===== {probe:?} =====");
        match probe {
            Probe::Latency => latency(&mut c, df)?,
            Probe::FeedbackOff => {
                let (_, sign) = open_loop(&mut c)?;
                measured_sign = measured_sign.or(sign);
                close_loop(&mut c)?;
            }
            Probe::ZSteps => {
                let sign = cli.z_sign.map(|s| s as f64).or(measured_sign);
                z_steps(&mut c, df, cli.lift_nm * NM, sign)?;
            }
            Probe::Folme => folme(
                &mut c,
                cli.folme_speed_nm_s * NM,
                cli.line_nm * NM,
                cli.waypoints,
            )?,
            Probe::All => unreachable!("expanded above"),
        }
    }
    Ok(())
}

/// The RT slot whose name mentions a frequency shift.
fn find_freq_shift(c: &mut NanonisClient) -> Result<u8, Box<dyn std::error::Error>> {
    let names = c.signal_names_get()?;
    let hit = names.iter().position(|n| {
        let n = n.to_lowercase();
        n.contains("freq") && n.contains("shift")
    });
    match hit {
        Some(i) => {
            println!("frequency shift: RT slot {i} ({:?})", names[i]);
            Ok(i as u8)
        }
        None => Err("no signal name mentions a frequency shift; pass --df-signal".into()),
    }
}

/// Print everything the probes depend on, before touching anything.
fn survey(c: &mut NanonisClient, df: u8) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n----- state before anything moves -----");
    println!("RT frequency        {:.0} Hz", c.util_rt_freq_get()?);
    println!("Z controller        {:?}", c.z_ctrl_status_get()?);
    println!(
        "Z position          {:.3} nm",
        c.z_ctrl_z_pos_get()? as f64 / NM
    );
    let (high, low) = c.z_ctrl_limits_get()?;
    println!(
        "Z limits            {:.1} nm .. {:.1} nm",
        low as f64 / NM,
        high as f64 / NM
    );
    println!(
        "TipLift             {:.1} pm",
        c.z_ctrl_tip_lift_get()? as f64 / PM
    );
    println!(
        "switch-off delay    {:.3} s",
        c.z_ctrl_switch_off_delay_get()?
    );
    let speed = c.folme_speed_get()?;
    println!(
        "FolMe speed         {:.2} nm/s ({})",
        speed.speed_m_s as f64 / NM,
        if speed.custom_speed {
            "custom"
        } else {
            "scan speed"
        }
    );
    let xy = c.folme_xy_pos_get(true)?;
    println!(
        "XY position         ({:.3}, {:.3}) nm",
        xy.x / NM,
        xy.y / NM
    );
    println!("frequency shift     {:.3} Hz", c.signal_val_get(df, true)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// latency
// ---------------------------------------------------------------------------

fn latency(c: &mut NanonisClient, df: u8) -> Result<(), Box<dyn std::error::Error>> {
    const N: usize = 100;
    let z = time_calls(N, || c.z_ctrl_z_pos_get().map(|_| ()))?;
    report("ZCtrl.ZPosGet", N, &z);
    let xy = time_calls(N, || c.folme_xy_pos_get(false).map(|_| ()))?;
    report("FolMe.XYPosGet", N, &xy);
    let s = time_calls(N, || c.signal_val_get(df, false).map(|_| ()))?;
    report("Signals.ValGet", N, &s);
    let s_wait = time_calls(N, || c.signal_val_get(df, true).map(|_| ()))?;
    report("Signals.ValGet (wait for newest)", N, &s_wait);
    println!(
        "\nA waypoint of the trace costs one XY set plus one Z set plus a read, \
         so budget roughly {:.1} ms per point from these numbers.",
        (mean(&xy) + mean(&z) + mean(&s)) * 1e3
    );
    Ok(())
}

fn time_calls<E>(n: usize, mut call: impl FnMut() -> Result<(), E>) -> Result<Vec<f64>, E> {
    let mut times = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        call()?;
        times.push(t.elapsed().as_secs_f64());
    }
    Ok(times)
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

fn report(what: &str, n: usize, times: &[f64]) {
    let max = times.iter().cloned().fold(0.0, f64::max);
    println!(
        "{what:<36} {n} calls: mean {:.2} ms, max {:.2} ms",
        mean(times) * 1e3,
        max * 1e3
    );
}

// ---------------------------------------------------------------------------
// feedback off
// ---------------------------------------------------------------------------

/// Open the loop and measure what Z does. Returns the Z the loop opened at
/// (after TipLift) and, if TipLift made it measurable, the retract sign.
fn open_loop(c: &mut NanonisClient) -> Result<(f64, Option<f64>), Box<dyn std::error::Error>> {
    let status = c.z_ctrl_status_get()?;
    if status != rusty_tip::spm_controller::ZControllerStatus::On {
        return Err(format!("the Z controller is {status:?}, not On; engage the tip first").into());
    }
    let tip_lift = c.z_ctrl_tip_lift_get()? as f64;
    let delay = c.z_ctrl_switch_off_delay_get()? as f64;
    let z0 = c.z_ctrl_z_pos_get()? as f64;
    println!("Z with the loop closed   {:.3} nm", z0 / NM);

    let t = Instant::now();
    c.z_ctrl_on_off_set(false)?;
    let mut reported_off = None;
    while t.elapsed() < Duration::from_secs(10) {
        if !c.z_ctrl_on_off_get()? {
            reported_off = Some(t.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    match reported_off {
        Some(dt) => println!(
            "controller reported off after {:.1} ms (switch-off delay is {:.0} ms)",
            dt.as_secs_f64() * 1e3,
            delay * 1e3
        ),
        None => {
            let _ = c.z_ctrl_on_off_set(true);
            return Err("the controller never reported the loop open; loop closed again".into());
        }
    }

    // Let the switch-off delay and the TipLift ramp finish before reading.
    std::thread::sleep(Duration::from_secs_f64(delay + 0.3));
    let z1 = c.z_ctrl_z_pos_get()? as f64;
    let dz = z1 - z0;
    println!(
        "Z with the loop open     {:.3} nm (moved {:+.1} pm; TipLift is {:.1} pm)",
        z1 / NM,
        dz / PM,
        tip_lift / PM
    );

    let sign = if tip_lift.abs() < PM {
        println!("TipLift is zero, so the retract direction cannot be measured here");
        None
    } else if dz.abs() < tip_lift.abs() / 2.0 {
        println!("Z moved much less than TipLift; not trusting this as a direction");
        None
    } else {
        let s = dz.signum();
        println!(
            "retract direction: {} (TipLift moved Z {})",
            if s > 0.0 { "+Z" } else { "-Z" },
            if s > 0.0 { "up" } else { "down" }
        );
        Some(s)
    };
    Ok((z1, sign))
}

fn close_loop(c: &mut NanonisClient) -> Result<(), Box<dyn std::error::Error>> {
    c.z_ctrl_on_off_set(true)?;
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10) {
        if c.z_ctrl_on_off_get()? {
            println!(
                "loop closed again after {:.1} ms, Z {:.3} nm, status {:?}",
                t.elapsed().as_secs_f64() * 1e3,
                c.z_ctrl_z_pos_get()? as f64 / NM,
                c.z_ctrl_status_get()?
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err("the controller never reported the loop closed; check the Z controller by hand".into())
}

// ---------------------------------------------------------------------------
// z steps
// ---------------------------------------------------------------------------

fn z_steps(
    c: &mut NanonisClient,
    df: u8,
    lift: f64,
    sign: Option<f64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let df_ref = c.signal_val_get(df, true)? as f64;
    let (z_open, measured) = open_loop(c)?;
    let sign = match sign.or(measured) {
        Some(s) => s,
        None => {
            close_loop(c)?;
            return Err(
                "cannot tell which way retracts: TipLift gave no direction and --z-sign \
                 was not given. Set a TipLift in the Z controller, or pass --z-sign."
                    .into(),
            );
        }
    };

    // Everything from here restores Z and closes the loop, whichever way it
    // ends, so the closure only reports.
    let z_lift = z_open + sign * lift;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        println!(
            "retracting to {:.3} nm ({:+.1} nm)",
            z_lift / NM,
            sign * lift / NM
        );
        c.z_ctrl_z_pos_set(z_lift as f32)?;
        std::thread::sleep(Duration::from_millis(500));
        let df_lift = c.signal_val_get(df, true)? as f64;
        println!("frequency shift at the setpoint {df_ref:+.3} Hz, lifted {df_lift:+.3} Hz");
        if df_lift.abs() > df_ref.abs() + 0.5 {
            return Err(format!(
                "the frequency shift GREW after lifting ({df_ref:+.3} to {df_lift:+.3} Hz), \
                 which means the lift went toward the surface. Aborting; the Z sign is wrong."
            )
            .into());
        }

        for step in [10.0 * PM, 100.0 * PM, 1.0 * NM] {
            let target = z_lift + sign * step;
            let t = Instant::now();
            c.z_ctrl_z_pos_set(target as f32)?;
            let set_ms = t.elapsed().as_secs_f64() * 1e3;
            // Watch the response settle rather than reading once.
            let mut trace = Vec::new();
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(30));
                trace.push(c.signal_val_get(df, true)? as f64);
            }
            let reached = c.z_ctrl_z_pos_get()? as f64;
            println!(
                "step {:>6.0} pm: set call {set_ms:.2} ms, Z reached {:+.1} pm of target, \
                 df over 300 ms: {}",
                step / PM,
                (reached - target) / PM,
                trace
                    .iter()
                    .map(|v| format!("{v:+.3}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            c.z_ctrl_z_pos_set(z_lift as f32)?;
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(())
    })();

    println!("returning to {:.3} nm and closing the loop", z_open / NM);
    if let Err(e) = c.z_ctrl_z_pos_set(z_open as f32) {
        eprintln!("could not return Z: {e}; closing the loop from where it is");
    }
    std::thread::sleep(Duration::from_millis(200));
    close_loop(c)?;
    result
}

// ---------------------------------------------------------------------------
// FolMe
// ---------------------------------------------------------------------------

fn folme(
    c: &mut NanonisClient,
    speed: f64,
    line: f64,
    waypoints: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    if waypoints < 2 {
        return Err("--waypoints must be at least 2".into());
    }
    let status = c.z_ctrl_status_get()?;
    if status != rusty_tip::spm_controller::ZControllerStatus::On {
        return Err(format!("the Z controller is {status:?}, not On; engage the tip first").into());
    }
    let origin = c.folme_xy_pos_get(true)?;
    let original = c.folme_speed_get()?;
    println!(
        "origin ({:.3}, {:.3}) nm, replaying {waypoints} waypoints over {:.2} nm at {:.2} nm/s, \
         expected {:.2} s one way",
        origin.x / NM,
        origin.y / NM,
        line / NM,
        speed / NM,
        line / speed
    );
    c.folme_speed_set(speed as f32, true)?;

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut times = Vec::with_capacity(waypoints);
        let total = Instant::now();
        for i in 1..=waypoints {
            let target = Position {
                x: origin.x + line * i as f64 / waypoints as f64,
                y: origin.y,
            };
            let t = Instant::now();
            c.folme_xy_pos_set(target, true)?;
            times.push(t.elapsed().as_secs_f64());
        }
        let elapsed = total.elapsed().as_secs_f64();
        let arrived = c.folme_xy_pos_get(true)?;
        report("FolMe.XYPosSet (wait)", waypoints, &times);
        println!(
            "line took {elapsed:.2} s against {:.2} s expected from distance/speed; \
             ended {:+.1} pm from the last waypoint in x",
            line / speed,
            (arrived.x - (origin.x + line)) / PM
        );
        if elapsed < 0.5 * line / speed {
            println!(
                "the moves returned far faster than the distance allows, so the wait flag \
                 did NOT block until arrival; a trace would have to poll XY itself"
            );
        }
        Ok(())
    })();

    println!("returning to the origin and restoring the FolMe speed");
    if let Err(e) = c.folme_xy_pos_set(origin, true) {
        eprintln!("could not return to the origin: {e}");
    }
    if let Err(e) = c.folme_speed_set(original.speed_m_s, original.custom_speed) {
        eprintln!("could not restore the FolMe speed: {e}");
    }
    result
}
