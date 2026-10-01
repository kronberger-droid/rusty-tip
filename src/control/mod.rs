//! A machine interface to a session, for agents and scripts.
//!
//! A [`Request`] is one command, a [`Reply`] its answer, both JSON. The
//! same requests run three ways:
//!
//! - **Through a server** on the session thread the workbench or
//!   `rusty-tip serve` already holds, over a local TCP socket, one JSON
//!   request per line and one reply per line ([`server`]). The connection
//!   stays up between commands and one thread owns the controller.
//! - **In process**, on a [`Session`] the caller connected itself: the
//!   one-shot mode of the CLI.
//!
//! Every request says whether it [`acts`](Request::acts), that is whether it
//! can change anything on the instrument. A server started read-only
//! refuses those before they reach the session, so an agent can be given a
//! connection that can watch but not move.
//!
//! Each [`ErrorKind`] has an exit code, so a caller of the CLI can branch on
//! the outcome without reading the message.

pub mod limits;
pub mod server;

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::session::{ConnState, PresetLoad, Session, SessionCmd, SessionRemote, SessionStatus};
use crate::spm_error::SpmError;

pub use limits::Limits;

/// Version of the request and reply shapes. Raised on any change a client
/// would notice.
pub const PROTOCOL_VERSION: u32 = 1;

/// Where a server listens unless told otherwise. Loopback only: the socket
/// has no authentication of its own.
pub const DEFAULT_ADDR: &str = "127.0.0.1:47474";

/// How long a request that needs the controller waits for the session
/// thread before answering `busy`. A running job holds the thread.
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// One command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// The commands, their parameters, the exit codes and the limits.
    /// Needs no connection.
    Describe,
    /// The connection: state, controller facts, capabilities, the live
    /// readouts and the files loaded. Answered from the last report, so it
    /// does not wait for a running job.
    Status,
    /// Read signals by name: once each, or with `samples`, the mean and
    /// standard deviation of that many stream samples.
    Read {
        /// Signal names as the registry knows them: `"current"`,
        /// `"freq shift"`, `"Z (m)"`. Case does not matter.
        signals: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        samples: Option<usize>,
    },
}

impl Request {
    /// Whether the request can change anything on the instrument. A
    /// read-only server refuses the ones that do.
    pub fn acts(&self) -> bool {
        match self {
            Request::Describe | Request::Status | Request::Read { .. } => false,
        }
    }

    /// The command's name, as it appears in `cmd`.
    pub fn name(&self) -> &'static str {
        match self {
            Request::Describe => "describe",
            Request::Status => "status",
            Request::Read { .. } => "read",
        }
    }
}

/// What went wrong, coarse enough to branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The request did not parse, or named something that does not exist.
    BadRequest,
    /// No server to talk to, or the session is not connected or poisoned.
    NotConnected,
    /// The request acts and the server is read-only.
    ReadOnly,
    /// Outside the limits, or beyond what the controller can do.
    Refused,
    /// The session thread did not answer in time; a job is running.
    Busy,
    /// The controller or the link to it failed.
    Controller,
    /// Anything else.
    Failed,
}

impl ErrorKind {
    pub const ALL: [ErrorKind; 7] = [
        ErrorKind::Failed,
        ErrorKind::BadRequest,
        ErrorKind::NotConnected,
        ErrorKind::ReadOnly,
        ErrorKind::Refused,
        ErrorKind::Busy,
        ErrorKind::Controller,
    ];

    /// The CLI's exit code for this error; success is 0. 2 is also what
    /// the argument parser exits with on a malformed command line.
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorKind::Failed => 1,
            ErrorKind::BadRequest => 2,
            ErrorKind::NotConnected => 3,
            ErrorKind::ReadOnly => 4,
            ErrorKind::Refused => 5,
            ErrorKind::Busy => 6,
            ErrorKind::Controller => 7,
        }
    }

    /// The kind an error from the controller layer falls under.
    pub fn of(error: &SpmError) -> Self {
        match error {
            SpmError::Io { .. }
            | SpmError::Timeout(_)
            | SpmError::Protocol(_)
            | SpmError::Hardware { .. } => ErrorKind::Controller,
            SpmError::Unsupported(_) => ErrorKind::Refused,
            SpmError::Workflow(_) | SpmError::ShutdownRequested => ErrorKind::Failed,
        }
    }
}

/// The error half of a [`Reply`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ControlError {
    pub kind: ErrorKind,
    pub message: String,
}

/// The answer to one request: `ok` with a `result`, or not with an
/// `error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ControlError>,
}

impl Reply {
    pub fn ok(result: Value) -> Self {
        Self {
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            result: None,
            error: Some(ControlError {
                kind,
                message: message.into(),
            }),
        }
    }

    /// 0 on success, the error kind's code otherwise.
    pub fn exit_code(&self) -> i32 {
        match &self.error {
            None => 0,
            Some(e) => e.kind.exit_code(),
        }
    }
}

/// How the requests are being served, which `describe` and `status`
/// report so an agent knows what it may do.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Serving {
    /// Acting requests are refused.
    pub read_only: bool,
    pub limits: Limits,
}

/// What a request runs against.
pub enum Target<'a> {
    /// A session this caller owns: the one-shot mode.
    Local(&'a mut Session),
    /// A session thread behind a remote: the server mode.
    Remote(&'a SessionRemote),
}

/// Answer one request. Never panics on a bad request; everything comes
/// back as a [`Reply`].
pub fn execute(target: Target<'_>, request: &Request, serving: &Serving) -> Reply {
    if serving.read_only && request.acts() {
        return Reply::err(
            ErrorKind::ReadOnly,
            format!(
                "`{}` can change the instrument and this server is read-only",
                request.name()
            ),
        );
    }
    match request {
        Request::Describe => Reply::ok(describe(Some(serving))),
        Request::Status => {
            let status = match target {
                Target::Local(session) => {
                    let mut status = SessionStatus::of(session);
                    if session.state() == ConnState::Connected {
                        match session.read_readouts() {
                            Ok(readouts) => status.readouts = readouts,
                            Err(e) => status.error = Some(format!("readout failed: {e}")),
                        }
                    }
                    status
                }
                Target::Remote(remote) => remote.status(),
            };
            Reply::ok(status_json(&status, serving))
        }
        Request::Read { signals, samples } => {
            if signals.is_empty() {
                return Reply::err(ErrorKind::BadRequest, "`read` needs at least one signal");
            }
            if *samples == Some(0) {
                return Reply::err(ErrorKind::BadRequest, "`samples` has to be at least 1");
            }
            let (signals, samples) = (signals.clone(), *samples);
            on_session(target, move |session| read(session, &signals, samples))
        }
    }
}

/// Run `f` with the session: directly, or on the session thread, waiting
/// at most [`BUSY_TIMEOUT`] for it.
fn on_session(target: Target<'_>, f: impl FnOnce(&mut Session) -> Reply + Send + 'static) -> Reply {
    match target {
        Target::Local(session) => f(session),
        Target::Remote(remote) => {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let call = SessionCmd::Call(Box::new(move |session: &mut Session| {
                let _ = tx.send(f(session));
            }));
            if let Err(e) = remote.send(call) {
                return Reply::err(ErrorKind::NotConnected, e.to_string());
            }
            rx.recv_timeout(BUSY_TIMEOUT).unwrap_or_else(|_| {
                Reply::err(
                    ErrorKind::Busy,
                    format!(
                        "the session did not answer within {} s; a job is probably running",
                        BUSY_TIMEOUT.as_secs()
                    ),
                )
            })
        }
    }
}

fn read(session: &mut Session, signals: &[String], samples: Option<usize>) -> Reply {
    match session.state() {
        ConnState::Connected => {}
        state => {
            return Reply::err(
                ErrorKind::NotConnected,
                format!("the session is {state:?}, not connected"),
            );
        }
    }
    if let Some(registry) = session.registry() {
        let unknown: Vec<&str> = signals
            .iter()
            .filter(|s| registry.get_by_name(s).is_none())
            .map(String::as_str)
            .collect();
        if !unknown.is_empty() {
            return Reply::err(
                ErrorKind::BadRequest,
                format!(
                    "no signal called {}; `status` lists the readouts, the controller's \
                     signal list has the rest",
                    unknown.join(", ")
                ),
            );
        }
    }
    match session.read_named(signals, samples) {
        Ok(readings) => Reply::ok(json!({ "readings": readings })),
        Err(e) => Reply::err(ErrorKind::of(&e), e.to_string()),
    }
}

/// `status` as JSON.
fn status_json(status: &SessionStatus, serving: &Serving) -> Value {
    let load = |l: &Option<PresetLoad>| {
        l.as_ref().map(|l| {
            json!({
                "path": l.path,
                "by": l.by,
                "at_unix_s": l.at
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0),
            })
        })
    };
    let mut capabilities: Vec<_> = status.capabilities.iter().copied().collect();
    capabilities.sort_by_key(|c| format!("{c:?}"));
    json!({
        "state": status.state.unwrap_or(ConnState::Disconnected),
        "read_only": serving.read_only,
        "facts": status.facts,
        "capabilities": capabilities,
        "readouts": status.readouts,
        "layout": load(&status.layout),
        "settings": load(&status.settings),
        "error": status.error,
    })
}

/// What a client needs to drive this interface: the commands with whether
/// they act, the request schema, the exit codes, and with `serving`, the
/// read-only flag and the limits in force.
pub fn describe(serving: Option<&Serving>) -> Value {
    let commands: Vec<Value> = [
        Request::Describe,
        Request::Status,
        Request::Read {
            signals: Vec::new(),
            samples: None,
        },
    ]
    .iter()
    .map(|r| json!({ "cmd": r.name(), "acts": r.acts() }))
    .collect();
    let exit_codes: serde_json::Map<String, Value> = std::iter::once(("ok".to_string(), json!(0)))
        .chain(ErrorKind::ALL.iter().map(|k| {
            (
                serde_json::to_value(k)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default(),
                json!(k.exit_code()),
            )
        }))
        .collect();
    let mut out = json!({
        "name": "rusty-tip",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": PROTOCOL_VERSION,
        "commands": commands,
        "request_schema": schemars::schema_for!(Request),
        "reply_schema": schemars::schema_for!(Reply),
        "exit_codes": exit_codes,
    });
    if let Some(serving) = serving {
        out["read_only"] = json!(serving.read_only);
        out["limits"] = json!(serving.limits);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Backend;

    fn connected() -> Session {
        let mut session = Session::new(None);
        session.connect(&Backend::Mock).unwrap();
        session
    }

    #[test]
    fn requests_parse_from_the_json_an_agent_writes() {
        let r: Request =
            serde_json::from_str(r#"{"cmd":"read","signals":["current"],"samples":10}"#).unwrap();
        assert_eq!(
            r,
            Request::Read {
                signals: vec!["current".into()],
                samples: Some(10)
            }
        );
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"fly"}"#).is_err());
        assert!(
            serde_json::from_str::<Request>(r#"{"cmd":"read","signals":["z"],"sample":5}"#)
                .is_err(),
            "a misspelt parameter is an error, not silently dropped"
        );
    }

    #[test]
    fn every_error_kind_has_its_own_exit_code() {
        let mut codes: Vec<i32> = ErrorKind::ALL.iter().map(|k| k.exit_code()).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), ErrorKind::ALL.len());
        assert!(!codes.contains(&0));
    }

    #[test]
    fn reads_resolve_names_and_report_unknown_ones() {
        let mut session = connected();
        let serving = Serving::default();
        let reply = execute(
            Target::Local(&mut session),
            &Request::Read {
                signals: vec!["current".into(), "freq shift".into()],
                samples: None,
            },
            &serving,
        );
        assert!(reply.ok, "{reply:?}");
        let readings = &reply.result.unwrap()["readings"];
        assert_eq!(readings.as_array().unwrap().len(), 2);
        assert_eq!(readings[1]["asked"], "freq shift");

        let reply = execute(
            Target::Local(&mut session),
            &Request::Read {
                signals: vec!["warp core".into()],
                samples: None,
            },
            &serving,
        );
        assert_eq!(reply.error.unwrap().kind, ErrorKind::BadRequest);
    }

    #[test]
    fn a_read_with_samples_reports_their_spread() {
        let mut session = connected();
        let reply = execute(
            Target::Local(&mut session),
            &Request::Read {
                signals: vec!["freq shift".into()],
                samples: Some(20),
            },
            &Serving::default(),
        );
        let reading = &reply.result.unwrap()["readings"][0];
        assert_eq!(reading["samples"], 20);
        assert!(reading["std_dev"].as_f64().is_some());
    }

    #[test]
    fn nothing_reads_without_a_connection() {
        let mut session = Session::new(None);
        let reply = execute(
            Target::Local(&mut session),
            &Request::Read {
                signals: vec!["current".into()],
                samples: None,
            },
            &Serving::default(),
        );
        assert_eq!(reply.error.unwrap().kind, ErrorKind::NotConnected);

        let status = execute(
            Target::Local(&mut session),
            &Request::Status,
            &Serving::default(),
        );
        assert!(status.ok, "status answers without a connection");
        assert_eq!(status.result.unwrap()["state"], "disconnected");
    }

    /// Read-only refuses only what acts: watching stays possible.
    #[test]
    fn a_read_only_server_still_answers_what_only_reads() {
        let mut session = connected();
        let serving = Serving {
            read_only: true,
            ..Serving::default()
        };
        for request in [
            Request::Describe,
            Request::Status,
            Request::Read {
                signals: vec!["current".into()],
                samples: None,
            },
        ] {
            assert!(!request.acts());
            let reply = execute(Target::Local(&mut session), &request, &serving);
            assert!(reply.ok, "{}: {reply:?}", request.name());
        }
    }

    #[test]
    fn describe_lists_every_command_and_says_none_of_them_act_yet() {
        let d = describe(Some(&Serving {
            read_only: true,
            ..Serving::default()
        }));
        let names: Vec<&str> = d["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["cmd"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["describe", "status", "read"]);
        assert_eq!(d["read_only"], true);
        assert_eq!(d["exit_codes"]["read_only"], 4);
    }
}
