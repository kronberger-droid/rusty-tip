//! `rusty-tip`: drive a session from scripts and agents.
//!
//! Every command prints one JSON reply on stdout, `{"ok": true, "result":
//! …}` or `{"ok": false, "error": {"kind": …, "message": …}}`, and exits
//! with the error kind's code, so a caller branches without reading text.
//! Logs go to stderr. `rusty-tip describe` lists the commands, the request
//! schema and the exit codes.
//!
//! ```text
//! rusty-tip describe                       # the interface; needs no connection
//! rusty-tip status                         # ask the workbench or `serve`
//! rusty-tip read current "freq shift" --samples 200
//! rusty-tip --one-shot --mock status       # connect for this command only
//! rusty-tip serve --config lab.toml --read-only   # headless, watch-only
//! ```
//!
//! Commands go to the control server at `--addr`: the workbench's agent
//! socket or `rusty-tip serve`, which keep the connection between commands.
//! `--one-shot` connects for the one command instead and disconnects after.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use log::LevelFilter;

use rusty_tip::ShutdownFlag;
use rusty_tip::config::load_config;
use rusty_tip::control::server::{Server, call, check_loopback};
use rusty_tip::control::{
    DEFAULT_ADDR, ErrorKind, Limits, Reply, Request, Serving, Target, describe, execute,
};
use rusty_tip::session::{self, Backend, NanonisBackend, Session};

#[derive(Parser)]
#[command(
    name = "rusty-tip",
    version,
    about = "Drive a rusty-tip session from scripts and agents: JSON out, exit codes by outcome"
)]
struct Cli {
    /// The control server: the workbench's agent socket or `rusty-tip serve`.
    #[arg(long, global = true, default_value = DEFAULT_ADDR)]
    addr: String,
    /// Connect for this one command instead of asking a server.
    #[arg(long, global = true)]
    one_shot: bool,
    /// The config to connect with, for `--one-shot` and `serve`.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Connect to the simulator instead, for `--one-shot` and `serve`.
    #[arg(long, global = true, conflicts_with = "config")]
    mock: bool,
    /// Indent the JSON reply.
    #[arg(long, global = true)]
    pretty: bool,
    /// How much to log on stderr: off, error, warn, info, debug, trace.
    #[arg(long, global = true, default_value = "warn")]
    log: LevelFilter,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// The commands, the request schema, the exit codes. Needs no connection.
    Describe,
    /// The connection: state, controller, capabilities, live readouts.
    Status,
    /// Read signals by name, once or averaged over stream samples.
    Read {
        /// Signal names: "current", "freq shift", "Z (m)". Case does not matter.
        #[arg(required = true)]
        signals: Vec<String>,
        /// Average this many stream samples and report their spread.
        #[arg(long)]
        samples: Option<usize>,
    },
    /// Connect and serve requests on `--addr` until Ctrl+C.
    Serve {
        /// Refuse every request that can change the instrument.
        #[arg(long)]
        read_only: bool,
        /// Bounds acting requests have to stay inside, as TOML.
        #[arg(long)]
        limits: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    // A malformed command line is a reply like any other; only help and
    // the version are text, since they are asked for as text.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if !e.use_stderr() => e.exit(),
        Err(e) => {
            let reply = Reply::err(ErrorKind::BadRequest, e.to_string().trim().to_string());
            print(&reply, false);
            return ExitCode::from(reply.exit_code() as u8);
        }
    };
    env_logger::Builder::new()
        .filter_level(cli.log)
        .target(env_logger::Target::Stderr)
        .init();

    let reply = match &cli.command {
        Command::Describe => Reply::ok(describe(None)),
        Command::Status => run(&cli, Request::Status),
        Command::Read { signals, samples } => run(
            &cli,
            Request::Read {
                signals: signals.clone(),
                samples: *samples,
            },
        ),
        Command::Serve { read_only, limits } => serve(&cli, *read_only, limits.as_deref()),
    };
    print(&reply, cli.pretty);
    ExitCode::from(reply.exit_code() as u8)
}

fn print(reply: &Reply, pretty: bool) {
    let text = if pretty {
        serde_json::to_string_pretty(reply)
    } else {
        serde_json::to_string(reply)
    };
    println!("{}", text.expect("a reply serializes"));
}

/// One request, through the server or on a connection of its own.
fn run(cli: &Cli, request: Request) -> Reply {
    if !cli.one_shot {
        return call(&cli.addr, &request);
    }
    // One command reads; loading the config's layout and settings files
    // would change the instrument to answer it.
    let mut session = match connect(cli, false) {
        Ok(session) => session,
        Err(reply) => return reply,
    };
    let reply = execute(Target::Local(&mut session), &request, &Serving::default());
    session.disconnect();
    reply
}

/// What `--config` or `--mock` names, connected; with `presets`, loading
/// the config's layout and settings files as the workbench does.
fn connect(cli: &Cli, presets: bool) -> Result<Session, Reply> {
    let backend = if cli.mock {
        Backend::Mock
    } else if let Some(path) = &cli.config {
        let config = load_config(path)
            .map_err(|e| Reply::err(ErrorKind::BadRequest, format!("{}: {e}", path.display())))?;
        let mut backend = NanonisBackend::from_config(&config);
        if !presets {
            backend.layout_file = None;
            backend.settings_file = None;
        }
        Backend::Nanonis(backend)
    } else {
        return Err(Reply::err(
            ErrorKind::BadRequest,
            "connecting needs --config <file> or --mock",
        ));
    };
    let mut session = Session::new(None);
    session
        .connect(&backend)
        .map_err(|e| Reply::err(ErrorKind::NotConnected, format!("connect failed: {e}")))?;
    Ok(session)
}

/// The headless server: connect, listen, and wait for Ctrl+C.
fn serve(cli: &Cli, read_only: bool, limits: Option<&std::path::Path>) -> Reply {
    let limits = match limits.map(Limits::load).transpose() {
        Ok(limits) => limits.unwrap_or_default(),
        Err(e) => return Reply::err(ErrorKind::BadRequest, e),
    };
    if let Err(e) = check_loopback(&cli.addr) {
        return Reply::err(ErrorKind::BadRequest, format!("--addr {}: {e}", cli.addr));
    }
    let session = match connect(cli, true) {
        Ok(session) => session,
        Err(reply) => return reply,
    };
    let handle = session::spawn_with(session);
    // Nothing here shows the session's updates, but the thread sends them
    // all the same, the stream ten times a second: read them away.
    let updates = handle.updates().clone();
    std::thread::spawn(move || for _ in updates.iter() {});
    let serving = Serving { read_only, limits };
    let server = match Server::start(&cli.addr, handle.remote(), serving.clone()) {
        Ok(server) => server,
        Err(e) => {
            return Reply::err(
                ErrorKind::Failed,
                format!("cannot listen on {}: {e}", cli.addr),
            );
        }
    };

    // Up: say where, on stdout, so whoever started it can connect.
    print(
        &Reply::ok(serde_json::json!({
            "listening": server.addr().to_string(),
            "read_only": serving.read_only,
            "limits": serving.limits,
        })),
        cli.pretty,
    );

    let stop = ShutdownFlag::on_ctrl_c();
    while !stop.wait_timeout(std::time::Duration::from_secs(3600)) {}
    log::info!("Stopping the control server");
    drop(server);
    handle.join();
    Reply::ok(serde_json::json!({ "stopped": true }))
}
