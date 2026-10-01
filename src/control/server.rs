//! The control socket: newline-delimited JSON over loopback TCP.
//!
//! A client writes one [`Request`] per line and reads one [`Reply`] per
//! line, as many as it likes on one connection. TCP rather than a Unix
//! socket since the lab PCs run Windows too; the server only binds to
//! loopback addresses, since the socket has no authentication of its own.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::{BUSY_TIMEOUT, ErrorKind, Reply, Request, Serving, Target, execute};
use crate::session::SessionRemote;

/// A running control socket. Dropping it stops accepting; connections
/// already open finish their current request and close.
pub struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Listen on `addr`, a loopback address, and answer requests against
    /// the session behind `remote`.
    pub fn start(addr: &str, remote: SessionRemote, serving: Serving) -> io::Result<Self> {
        check_loopback(addr)?;
        let listener = TcpListener::bind(addr)?;
        let local = listener.local_addr()?;
        let read_only = serving.read_only;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let serving = Arc::new(serving);
        let thread = thread::Builder::new()
            .name("control".into())
            .spawn(move || {
                // Blocks in `accept`; a drop wakes it by connecting.
                for stream in listener.incoming() {
                    if flag.load(Ordering::SeqCst) {
                        break;
                    }
                    match stream {
                        Ok(stream) => {
                            let (remote, serving) = (remote.clone(), Arc::clone(&serving));
                            let _ = thread::Builder::new()
                                .name("control client".into())
                                .spawn(move || serve_client(stream, &remote, &serving));
                        }
                        Err(e) => log::warn!("control socket: accept failed: {e}"),
                    }
                }
            })?;
        log::info!(
            "Control socket on {local}{}",
            if read_only { ", read-only" } else { "" }
        );
        Ok(Self {
            addr: local,
            stop,
            thread: Some(thread),
        })
    }

    /// The address it listens on, with the port the system picked when
    /// asked for port 0.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag; if the connect fails the
        // listener is gone already and the loop with it.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Refuse any address that is not loopback: the socket has no
/// authentication of its own. Checked before anything connects, so a bad
/// `--addr` costs nothing.
pub fn check_loopback(addr: &str) -> io::Result<()> {
    let mut resolved = addr.to_socket_addrs()?.peekable();
    if resolved.peek().is_none() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no address"));
    }
    match resolved.find(|a| !a.ip().is_loopback()) {
        None => Ok(()),
        Some(other) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "the control socket only listens on loopback, not {other}: it has no \
                 authentication of its own"
            ),
        )),
    }
}

/// Answer one client's requests until it hangs up.
fn serve_client(stream: TcpStream, remote: &SessionRemote, serving: &Serving) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else {
            return;
        };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(request) => execute(Target::Remote(remote), &request, serving),
            Err(e) => Reply::err(ErrorKind::BadRequest, format!("not a request: {e}")),
        };
        let mut text = serde_json::to_string(&reply).expect("a reply serializes");
        text.push('\n');
        if writer.write_all(text.as_bytes()).is_err() {
            return;
        }
    }
}

/// Send one request to a server and wait for its reply. A server that is
/// not there comes back as a `not_connected` reply, not an I/O error, so a
/// caller handles every outcome the same way.
pub fn call(addr: &str, request: &Request) -> Reply {
    let attempt = || -> io::Result<Reply> {
        let target = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address"))?;
        let mut stream = TcpStream::connect_timeout(&target, Duration::from_secs(2))?;
        // A read waits up to the server's own busy timeout and a margin.
        stream.set_read_timeout(Some(BUSY_TIMEOUT + Duration::from_secs(5)))?;
        let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
        line.push('\n');
        stream.write_all(line.as_bytes())?;
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply)?;
        serde_json::from_str(&reply).map_err(io::Error::other)
    };
    attempt().unwrap_or_else(|e| {
        Reply::err(
            ErrorKind::NotConnected,
            format!(
                "no control server at {addr} ({e}). Start the workbench with its agent \
                 socket on, or `rusty-tip serve`, or pass --one-shot"
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{self, Backend, Session, SessionCmd};

    fn mock_server(read_only: bool) -> (session::SessionHandle, Server) {
        let handle = session::spawn_with(Session::new(None));
        handle.send(SessionCmd::Connect(Backend::Mock)).unwrap();
        let server = Server::start(
            "127.0.0.1:0",
            handle.remote(),
            Serving {
                read_only,
                ..Serving::default()
            },
        )
        .unwrap();
        (handle, server)
    }

    #[test]
    fn a_client_reads_through_the_socket() {
        let (_handle, server) = mock_server(false);
        let addr = server.addr().to_string();
        let reply = call(
            &addr,
            &Request::Read {
                signals: vec!["current".into()],
                samples: None,
            },
        );
        assert!(reply.ok, "{reply:?}");
        let status = call(&addr, &Request::Status);
        assert_eq!(status.result.unwrap()["state"], "connected");
    }

    /// `serve` connects before it hands the session to its thread; the
    /// status has to say what it is connected to all the same.
    #[test]
    fn a_session_handed_over_connected_reports_what_it_is() {
        let mut connected = Session::new(None);
        connected.connect(&Backend::Mock).unwrap();
        let handle = session::spawn_with(connected);
        let server = Server::start("127.0.0.1:0", handle.remote(), Serving::default()).unwrap();
        let status = call(&server.addr().to_string(), &Request::Status)
            .result
            .unwrap();
        assert_eq!(status["state"], "connected");
        assert!(!status["capabilities"].as_array().unwrap().is_empty());
        assert!(status["facts"].is_object());
    }

    #[test]
    fn a_line_that_is_not_a_request_gets_a_bad_request_reply() {
        let (_handle, server) = mock_server(true);
        let mut stream = TcpStream::connect(server.addr()).unwrap();
        stream.write_all(b"{\"cmd\":\"fly\"}\n").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let reply: Reply = serde_json::from_str(&line).unwrap();
        assert_eq!(reply.error.unwrap().kind, ErrorKind::BadRequest);
    }

    #[test]
    fn no_server_is_a_not_connected_reply() {
        let reply = call("127.0.0.1:1", &Request::Status);
        assert_eq!(reply.error.unwrap().kind, ErrorKind::NotConnected);
    }

    #[test]
    fn only_loopback_is_served() {
        let handle = session::spawn_with(Session::new(None));
        assert!(Server::start("0.0.0.0:0", handle.remote(), Serving::default()).is_err());
    }
}
