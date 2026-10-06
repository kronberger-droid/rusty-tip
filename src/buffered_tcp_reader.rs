//! Buffered TCP Reader for continuous signal data collection
//!
//! This module provides a BufferedTCPReader that buffers the Nanonis TCP
//! Logger's data stream in the background with timestamps, for
//! time-windowed queries during SPM experiments.
//!
//! It reads the logger's socket itself rather than through
//! `nanonis_rs::TCPLoggerStream`, since that one drops the frame this reader
//! most needs: the first frame after every logger start is not data but the
//! list of signals the columns hold, in column order, with any signal the
//! logger could not stream already left out (one with no Signals Manager
//! slot, say). The header's state field marks it, `Start` against `Running`
//! for data; its counter is 0 like the first data frame's, so the counter
//! cannot tell them apart. See [`Window::columns`].

use crate::NanonisError;
use crate::types::{SignalFrame, TCPLogStatus, TimestampedSignalFrame};
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::io::{ErrorKind, Read};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Bytes in a frame header: channels (u32), oversampling (f32), counter
/// (u64), state (u16), all big-endian.
const HEADER_BYTES: usize = 18;

/// More channels than the logger can have: a header claiming this many is
/// garbage, and allocating for it would be a mistake.
const MAX_CHANNELS: usize = 128;

/// How long one socket read waits before checking whether to stop. Short,
/// so a stopped logger does not kill the reader and a shutdown is prompt.
const POLL: Duration = Duration::from_millis(200);

/// Frames together with the column list they were sent under, taken in one
/// look so a start frame arriving in between cannot pair old frames with a
/// new list.
pub struct Window {
    pub frames: Vec<TimestampedSignalFrame>,
    /// The signal indexes of the columns, in column order, as the logger
    /// announced them at its last start; `None` before any start frame was
    /// seen. A signal the logger was asked for and could not stream is not
    /// in it.
    pub columns: Option<Arc<[u32]>>,
}

/// The buffer and what describes it, under one lock.
#[derive(Default)]
struct Stream {
    frames: VecDeque<TimestampedSignalFrame>,
    columns: Option<Arc<[u32]>>,
    /// Set by [`BufferedTCPReader::forget_columns`]: data frames are dropped
    /// until the next start frame, since until then nothing says which list
    /// they were sent under.
    awaiting_start: bool,
}

/// Buffered TCP reader that continuously collects timestamped signal data
///
/// A background thread reads the logger's frames into a circular buffer of
/// [`TimestampedSignalFrame`]s, and keeps the column list the logger last
/// announced. A new announcement clears the buffer, since the frames before
/// it belong to the previous channel list.
pub struct BufferedTCPReader {
    stream: Arc<RwLock<Stream>>,
    /// Background thread handle for buffering operations
    buffering_thread: Option<JoinHandle<Result<(), NanonisError>>>,
    /// Signal to shut down background thread
    shutdown_signal: Arc<AtomicBool>,
    /// Error from the reader thread, if it died unexpectedly.
    stream_error: Arc<Mutex<Option<String>>>,
    /// A handle on the socket, to unblock the thread on stop.
    socket: TcpStream,
}

impl BufferedTCPReader {
    /// Connect to the TCP Logger's data port and start buffering in the
    /// background.
    ///
    /// # Arguments
    /// * `host` - TCP server host address (e.g., "127.0.0.1")
    /// * `port` - TCP logger data stream port (typically 6590)
    /// * `buffer_size` - Maximum number of frames to keep in circular buffer
    pub fn new(host: &str, port: u16, buffer_size: usize) -> Result<Self, NanonisError> {
        let addr: SocketAddr = (host, port)
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .ok_or_else(|| NanonisError::Protocol(format!("Invalid address: {host}:{port}")))?;
        let io = |context: String| move |source| NanonisError::Io { source, context };
        let socket = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
            .map_err(io(format!("Failed to connect to TCP stream at {addr}")))?;
        socket
            .set_read_timeout(Some(POLL))
            .map_err(io("Setting TCP stream read timeout".into()))?;
        let mut reading = socket
            .try_clone()
            .map_err(io("Cloning the TCP stream".into()))?;

        let stream = Arc::new(RwLock::new(Stream::default()));
        let shutdown_signal = Arc::new(AtomicBool::new(false));
        let stream_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let start_time = Instant::now();

        let (stream_t, shutdown_t, error_t) = (
            Arc::clone(&stream),
            Arc::clone(&shutdown_signal),
            Arc::clone(&stream_error),
        );
        let buffering_thread = thread::Builder::new()
            .name("tcp-logger-buffer".into())
            .spawn(move || -> Result<(), NanonisError> {
                log::debug!("Started buffering thread for TCP logger data");
                loop {
                    let frame = match read_frame(&mut reading, &shutdown_t) {
                        Ok(Some(frame)) => frame,
                        Ok(None) => return Ok(()),
                        Err(e) => {
                            log::error!("TCP logger stream error: {e}");
                            *error_t.lock() = Some(e.to_string());
                            return Err(e);
                        }
                    };
                    let mut s = stream_t.write();
                    if frame.state == TCPLogStatus::Start as u16 {
                        let announced: Arc<[u32]> =
                            frame.data.iter().map(|&v| v.round() as u32).collect();
                        log::debug!("TCP logger announced its columns: {announced:?}");
                        s.frames.clear();
                        s.columns = Some(announced);
                        s.awaiting_start = false;
                    } else if !s.awaiting_start {
                        s.frames.push_back(TimestampedSignalFrame::new(
                            SignalFrame {
                                counter: frame.counter,
                                data: frame.data,
                            },
                            start_time,
                        ));
                        if s.frames.len() > buffer_size {
                            s.frames.pop_front();
                        }
                    }
                }
            })
            .expect("failed to spawn tcp-logger-buffer thread");

        Ok(Self {
            stream,
            buffering_thread: Some(buffering_thread),
            shutdown_signal,
            stream_error,
            socket,
        })
    }

    /// Check if the background buffering thread is still active.
    ///
    /// Returns `false` if shutdown was requested OR if the background
    /// thread exited on its own (e.g., due to a TCP stream error).
    pub fn is_buffering(&self) -> bool {
        if self.shutdown_signal.load(Ordering::Relaxed) {
            return false;
        }
        self.buffering_thread
            .as_ref()
            .is_some_and(|h| !h.is_finished())
    }

    /// Number of frames currently buffered.
    pub fn buffered_frames(&self) -> usize {
        self.stream.read().frames.len()
    }

    /// The announced columns (see [`Window::columns`]) and how many values
    /// the newest buffered frame carries, read together.
    pub fn layout(&self) -> (Option<Arc<[u32]>>, Option<usize>) {
        let s = self.stream.read();
        (
            s.columns.clone(),
            s.frames.back().map(|f| f.signal_frame.data.len()),
        )
    }

    /// Forget the announced columns, for a channel list about to change:
    /// until the logger announces the new one, the old one would be read
    /// against frames it does not describe. Data frames are dropped until
    /// that start frame, so a frame sent under the old list cannot pass for
    /// one of the new list, even by being as wide.
    pub fn forget_columns(&self) {
        let mut s = self.stream.write();
        s.awaiting_start = true;
        s.columns = None;
        s.frames.clear();
    }

    /// Returns the error message from the reader thread, if it died
    /// unexpectedly (e.g., due to a connection reset).
    ///
    /// Returns `None` if the stream is still running or shut down cleanly.
    pub fn stream_error(&self) -> Option<String> {
        self.stream_error.lock().clone()
    }

    /// The frames since a timestamp, with the columns they were sent under.
    pub fn get_data_since(&self, since: Instant) -> Window {
        // Frames arrive in time order, so the matches are a suffix: walk in
        // from the newest end instead of past the whole buffer, which is what
        // a 10 ms poll would otherwise do under the lock each time.
        let s = self.stream.read();
        let first = s
            .frames
            .iter()
            .rposition(|frame| frame.timestamp < since)
            .map_or(0, |i| i + 1);
        Window {
            frames: s.frames.range(first..).cloned().collect(),
            columns: s.columns.clone(),
        }
    }

    /// Every frame currently buffered, oldest first, with their columns.
    pub fn snapshot(&self) -> Window {
        let s = self.stream.read();
        Window {
            frames: s.frames.iter().cloned().collect(),
            columns: s.columns.clone(),
        }
    }

    /// Clear all buffered data. The background thread keeps running and
    /// fills the buffer again.
    pub fn clear_buffer(&self) {
        self.stream.write().frames.clear();
        log::debug!("Cleared TCP reader buffer");
    }

    /// Stop background buffering and close the connection.
    pub fn stop(&mut self) -> Result<(), NanonisError> {
        self.shutdown_signal.store(true, Ordering::Relaxed);
        let _ = self.socket.shutdown(Shutdown::Both);
        match self.buffering_thread.take() {
            Some(handle) => handle.join().unwrap_or_else(|_| {
                Err(NanonisError::Protocol("Buffering thread panicked".into()))
            }),
            None => Ok(()),
        }
    }
}

impl Drop for BufferedTCPReader {
    /// Automatically stop buffering when BufferedTCPReader is dropped
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// One frame as the logger sends it.
struct RawFrame {
    counter: u64,
    state: u16,
    data: Vec<f32>,
}

/// Read one frame, waiting through quiet spells; `None` once shutdown is
/// requested.
fn read_frame(
    stream: &mut impl Read,
    shutdown: &AtomicBool,
) -> Result<Option<RawFrame>, NanonisError> {
    let mut h = [0u8; HEADER_BYTES];
    if !read_fully(stream, &mut h, shutdown)? {
        return Ok(None);
    }
    let channels = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize;
    let counter = u64::from_be_bytes([h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15]]);
    let state = u16::from_be_bytes([h[16], h[17]]);
    if channels > MAX_CHANNELS {
        return Err(NanonisError::Protocol(format!(
            "TCP logger frame claims {channels} channels; the stream is out of step"
        )));
    }
    let mut payload = vec![0u8; channels * 4];
    if !read_fully(stream, &mut payload, shutdown)? {
        return Ok(None);
    }
    let data = payload
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_be_bytes(*b))
        .collect();
    Ok(Some(RawFrame {
        counter,
        state,
        data,
    }))
}

/// Fill `buf`, retrying across read timeouts so an idle logger is not an
/// error; `false` when shutdown was requested first. A closed or failed
/// socket after a shutdown request is the shutdown, not an error.
fn read_fully(
    stream: &mut impl Read,
    buf: &mut [u8],
    shutdown: &AtomicBool,
) -> Result<bool, NanonisError> {
    let mut filled = 0;
    while filled < buf.len() {
        let read = stream.read(&mut buf[filled..]);
        if shutdown.load(Ordering::Relaxed) {
            return Ok(false);
        }
        match read {
            Ok(0) => {
                return Err(NanonisError::Io {
                    source: std::io::Error::new(ErrorKind::UnexpectedEof, "connection closed"),
                    context: "TCP logger closed the data connection".to_string(),
                });
            }
            Ok(n) => filled += n,
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(e) => {
                return Err(NanonisError::Io {
                    source: e,
                    context: "Reading the TCP logger stream".to_string(),
                });
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    fn frame(counter: u64, state: u16, values: &[f32]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend((values.len() as u32).to_be_bytes());
        out.extend(10f32.to_be_bytes());
        out.extend(counter.to_be_bytes());
        out.extend(state.to_be_bytes());
        for v in values {
            out.extend(v.to_be_bytes());
        }
        out
    }

    fn wait_for(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn columns(reader: &BufferedTCPReader) -> Option<Vec<u32>> {
        reader.layout().0.map(|c| c.to_vec())
    }

    /// The start frame is the column list, not data; the counter-0 data
    /// frame after it is kept; a new start drops the old list's frames, and
    /// after the list is forgotten nothing is buffered until it does.
    #[test]
    fn the_start_frame_names_the_columns_and_a_new_one_clears_the_buffer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (go_on, wait) = std::sync::mpsc::channel::<()>();
        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(&frame(0, 2, &[30.0, 0.0, 24.0])).unwrap();
            s.write_all(&frame(0, 4, &[7.5e-7, 0.0, 2.0])).unwrap();
            s.write_all(&frame(1, 4, &[7.5e-7, 0.0, 2.0])).unwrap();
            wait.recv().unwrap();
            // Sent after the list changed but before its start frame.
            s.write_all(&frame(2, 4, &[7.5e-7, 0.0, 2.0])).unwrap();
            s.write_all(&frame(0, 2, &[24.0])).unwrap();
            s.write_all(&frame(0, 4, &[2.0])).unwrap();
            wait.recv().unwrap();
        });

        let mut reader = BufferedTCPReader::new("127.0.0.1", port, 100).unwrap();
        wait_for(|| reader.buffered_frames() == 2);
        assert_eq!(columns(&reader), Some(vec![30, 0, 24]));
        let window = reader.snapshot();
        assert_eq!(window.columns.as_deref(), Some(&[30, 0, 24][..]));
        assert_eq!(window.frames.len(), 2);

        reader.forget_columns();
        assert_eq!(reader.layout(), (None, None));

        go_on.send(()).unwrap();
        wait_for(|| columns(&reader) == Some(vec![24]) && reader.buffered_frames() == 1);
        assert_eq!(reader.snapshot().frames[0].signal_frame.data, vec![2.0]);

        go_on.send(()).unwrap();
        reader.stop().unwrap();
        server.join().unwrap();
    }
}
