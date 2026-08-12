//! Buffered TCP Reader for continuous signal data collection
//!
//! This module provides a BufferedTCPReader that automatically buffers TCP logger data
//! in the background using a lightweight time-series database approach. It leverages
//! the existing TCPLoggerStream infrastructure while providing efficient time-windowed
//! queries for synchronized data collection during SPM experiments.

use crate::NanonisError;
use crate::types::TimestampedSignalFrame;
use nanonis_rs::TCPLoggerStream;
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// TODO: For 2kHz sampling, consider replacing with:
// use crossbeam::queue::ArrayQueue; // Lock-free ring buffer
// use parking_lot::RwLock;          // Faster reader-writer lock

/// Buffered TCP reader that continuously collects timestamped signal data
///
/// This component creates a background thread that reads lightweight SignalFrame data
/// from TCPLoggerStream's channel and buffers it with high-resolution timestamps in a
/// circular buffer. It provides time-windowed query methods for retrieving data before,
/// during, and after specific time periods.
///
/// # High-Frequency Performance (2kHz+)
/// **IMPORTANT**: At sampling rates above 1kHz, lock contention becomes critical:
/// - Current implementation uses `Mutex<VecDeque>` suitable for <1kHz
/// - For 2kHz+, consider `crossbeam::queue::ArrayQueue` (lock-free)
/// - Alternative: `parking_lot::RwLock` for multiple concurrent readers
/// - Query methods must complete in <0.1ms to avoid data loss
///
/// # Memory Efficiency
/// Works with lightweight SignalFrame structures (just counter + data) throughout the
/// entire pipeline, avoiding the overhead of full TCPLoggerData per frame.
///
/// # Architecture
/// - TCPLoggerStream converts protocol data to SignalFrame (protocol → lightweight conversion)
/// - BufferedTCPReader adds timestamps to SignalFrame (timing layer)
/// - Thread-safe time-windowed queries while continuous collection runs in background
pub struct BufferedTCPReader {
    /// Thread-safe circular buffer of timestamped signal frames
    buffer: Arc<RwLock<VecDeque<TimestampedSignalFrame>>>,
    /// Background thread handle for buffering operations
    buffering_thread: Option<JoinHandle<Result<(), NanonisError>>>,
    /// Signal to shut down background thread
    shutdown_signal: Arc<AtomicBool>,
    /// Error from the TCP stream reader thread, if it died unexpectedly.
    /// Set by the buffering thread when it detects the stream disconnected.
    stream_error: Arc<Mutex<Option<String>>>,
}

impl BufferedTCPReader {
    /// Create a new BufferedTCPReader with automatic background data collection
    ///
    /// This establishes a connection to the TCP logger stream and starts a background
    /// thread for continuous data buffering with lightweight SignalFrame structures.
    ///
    /// # Arguments
    /// * `host` - TCP server host address (e.g., "127.0.0.1")
    /// * `port` - TCP logger data stream port (typically 6590)
    /// * `buffer_size` - Maximum number of frames to keep in circular buffer
    ///
    /// # Returns
    /// A BufferedTCPReader with active background collection, ready for queries
    ///
    /// # Implementation Notes
    /// - Creates TCPLoggerStream and gets its background reader channel
    /// - Starts buffering thread that converts SignalFrame to TimestampedSignalFrame
    /// - Implements circular buffer behavior (drops oldest when full)
    pub fn new(host: &str, port: u16, buffer_size: usize) -> Result<Self, NanonisError> {
        let tcp_stream = TCPLoggerStream::new(host, port)?;
        let (tcp_receiver, stream_handle) = tcp_stream.spawn_background_reader();

        let buffer = Arc::new(RwLock::new(VecDeque::with_capacity(buffer_size)));
        let buffer_clone = buffer.clone();

        let shutdown_signal = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown_signal.clone();

        let stream_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let stream_error_clone = stream_error.clone();

        let start_time = Instant::now();

        // Don't block waiting for first frame - let background thread handle it
        // The TCP logger might not be started yet when this constructor runs

        let buffering_thread = thread::Builder::new()
            .name("tcp-logger-buffer".into())
            .spawn(move || -> Result<(), NanonisError> {
                log::debug!("Started buffering thread for TCP logger data");

                while !shutdown_clone.load(Ordering::Relaxed) {
                    match tcp_receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(signal_frame) => {
                            // Skip the first frame (signal indices metadata)
                            if signal_frame.counter == 0 {
                                log::debug!(
                                    "Skipping metadata frame (counter=0) with signal indices"
                                );
                                continue;
                            }

                            let timestamped_frame =
                                TimestampedSignalFrame::new(signal_frame, start_time);

                            {
                                let mut buffer = buffer_clone.write();
                                buffer.push_back(timestamped_frame);

                                if buffer.len() > buffer_size {
                                    buffer.pop_front();
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            continue;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            // The stream reader thread exited. Join it to
                            // find out why and surface the error.
                            match stream_handle.join() {
                                Ok(Ok(())) => {
                                    log::info!("TCP logger stream closed cleanly");
                                }
                                Ok(Err(e)) => {
                                    log::error!("TCP logger stream error: {e}");
                                    *stream_error_clone.lock() = Some(e.to_string());
                                }
                                Err(_) => {
                                    log::error!("TCP logger stream thread panicked");
                                    *stream_error_clone.lock() =
                                        Some("stream reader thread panicked".into());
                                }
                            }
                            break;
                        }
                    }
                }
                Ok(())
            })
            .map_err(|e| NanonisError::Io {
                source: e,
                context: "spawning tcp-logger-buffer thread".to_string(),
            })?;

        Ok(Self {
            buffer,
            buffering_thread: Some(buffering_thread),
            shutdown_signal,
            stream_error,
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
        // The thread may have died (stream error, disconnect) without
        // the shutdown signal being set. Check the JoinHandle directly.
        self.buffering_thread
            .as_ref()
            .is_some_and(|h| !h.is_finished())
    }

    /// Returns the error message from the TCP stream reader thread, if it
    /// died unexpectedly (e.g., due to a connection reset or read timeout).
    ///
    /// Returns `None` if the stream is still running or shut down cleanly.
    pub fn stream_error(&self) -> Option<String> {
        self.stream_error.lock().clone()
    }

    /// Get all signal data since a specific timestamp
    ///
    /// # Arguments
    /// * `since` - Timestamp to start collecting data from
    ///
    /// # Returns
    /// Vector of timestamped signal frames from the specified time onwards
    ///
    /// # Thread Safety
    /// This method acquires a lock on the buffer briefly to copy matching frames.
    /// Lock is held for minimal time to avoid blocking the buffering thread.
    pub fn get_data_since(&self, since: Instant) -> Vec<TimestampedSignalFrame> {
        let buffer = self.buffer.read();
        buffer
            .iter()
            .filter(|frame| frame.timestamp >= since)
            .cloned()
            .collect()
    }

    /// Clear all buffered data
    ///
    /// This removes all frames from the buffer, effectively resetting it to an empty state.
    /// The background thread continues to run and will start filling the buffer again.
    /// This is useful when you want to discard old data and start fresh.
    ///
    /// # Example
    /// ```rust,ignore
    /// // Clear any stale data before starting a new measurement
    /// tcp_reader.clear_buffer();
    /// thread::sleep(Duration::from_millis(500)); // Wait for fresh data
    /// let fresh_data = tcp_reader.get_recent_data(Duration::from_millis(100));
    /// ```
    pub fn clear_buffer(&self) {
        let mut buffer = self.buffer.write();
        buffer.clear();
        log::debug!("Cleared TCP reader buffer");
    }

    /// Stop background buffering and clean up resources
    ///
    /// # Returns
    /// Result indicating if cleanup was successful
    ///
    /// # Implementation Notes
    /// - Sets shutdown signal to stop background thread
    /// - Waits for thread to finish and returns any errors
    /// - Called automatically when BufferedTCPReader is dropped
    pub fn stop(&mut self) -> Result<(), NanonisError> {
        self.shutdown_signal.store(true, Ordering::Relaxed);
        if let Some(handle) = self.buffering_thread.take() {
            handle
                .join()
                .unwrap_or_else(|_| Err(NanonisError::Protocol("Buffering thread panicked".into())))
        } else {
            Ok(())
        }
    }
}

impl Drop for BufferedTCPReader {
    /// Automatically stop buffering when BufferedTCPReader is dropped
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
