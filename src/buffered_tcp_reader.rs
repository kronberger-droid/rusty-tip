//! Buffered TCP Reader for continuous signal data collection
//!
//! This module provides a BufferedTCPReader that automatically buffers TCP logger data
//! in the background using a lightweight time-series database approach. It leverages
//! the existing TCPLoggerStream infrastructure while providing efficient time-windowed
//! queries for synchronized data collection during SPM experiments.

use crate::types::TimestampedSignalFrame;
use crate::NanonisError;
use nanonis_rs::TCPLoggerStream;
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
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
    /// Maximum number of frames to keep in circular buffer
    max_buffer_size: usize,
    /// Time when data collection started (for relative timestamps)
    start_time: Instant,
    /// Signal to shut down background thread
    shutdown_signal: Arc<AtomicBool>,
    /// Number of channels (configuration parameter)
    num_channels: u32,
    /// Oversampling rate (configuration parameter)
    oversampling: f32,
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
    /// * `num_channels` - Number of channels being recorded by TCP logger
    /// * `oversampling` - Oversampling rate configured for TCP logger
    ///
    /// # Returns
    /// A BufferedTCPReader with active background collection, ready for queries
    ///
    /// # Implementation Notes
    /// - Creates TCPLoggerStream and gets its background reader channel
    /// - Starts buffering thread that converts SignalFrame to TimestampedSignalFrame
    /// - Implements circular buffer behavior (drops oldest when full)
    pub fn new(
        host: &str,
        port: u16,
        buffer_size: usize,
        num_channels: u32,
        oversampling: f32,
    ) -> Result<Self, NanonisError> {
        let tcp_stream = TCPLoggerStream::new(host, port)?;
        let tcp_receiver = tcp_stream.spawn_background_reader();

        let buffer =
            Arc::new(RwLock::new(VecDeque::with_capacity(buffer_size)));
        let buffer_clone = buffer.clone();

        let shutdown_signal = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown_signal.clone();

        let start_time = Instant::now();

        // Don't block waiting for first frame - let background thread handle it
        // The TCP logger might not be started yet when this constructor runs

        let buffering_thread = thread::spawn(
            move || -> Result<(), NanonisError> {
                log::debug!("Started buffering thread for TCP logger data");

                while !shutdown_clone.load(Ordering::Relaxed) {
                    match tcp_receiver.recv_timeout(Duration::from_millis(100))
                    {
                        Ok(signal_frame) => {
                            // Skip the first frame (signal indices metadata)
                            if signal_frame.counter == 0 {
                                log::debug!("Skipping metadata frame (counter=0) with signal indices");
                                continue;
                            }

                            let timestamped_frame = TimestampedSignalFrame::new(
                                signal_frame,
                                start_time,
                            );

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
                            log::info!("TCP logger stream disconnected ending buffering");
                            break;
                        }
                    }
                }
                Ok(())
            },
        );

        Ok(Self {
            buffer,
            buffering_thread: Some(buffering_thread),
            max_buffer_size: buffer_size,
            start_time,
            shutdown_signal,
            num_channels,
            oversampling,
        })
    }

    /// Check if the background buffering thread is still active
    ///
    /// # Returns
    /// `true` if buffering is active, `false` if stopped or failed
    pub fn is_buffering(&self) -> bool {
        !self.shutdown_signal.load(Ordering::Relaxed)
    }

    /// Get current buffer utilization as a percentage
    ///
    /// # Returns
    /// Value between 0.0 and 1.0 indicating how full the buffer is
    ///
    /// # Usage
    /// Useful for monitoring buffer health and detecting if data collection
    /// is faster than buffer capacity
    pub fn buffer_utilization(&self) -> f64 {
        let buffer = self.buffer.read();
        buffer.len() as f64 / self.max_buffer_size as f64
    }

    /// Get the total uptime of the buffered TCP reader
    ///
    /// Returns the duration since the BufferedTCPReader was created and started
    /// collecting data. This can be useful for monitoring, logging, and understanding
    /// the data collection timespan.
    ///
    /// # Returns
    /// Duration since the reader was started
    ///
    /// # Thread Safety
    /// This method is very fast as it only reads the start_time field and calculates
    /// the current duration. No locks are acquired.
    ///
    /// # Example
    /// ```rust,ignore
    /// let tcp_reader = BufferedTCPReader::new("127.0.0.1", 6590, 1000, 24, 100.0)?;
    ///
    /// // Later...
    /// let uptime = tcp_reader.uptime();
    /// println!("TCP reader has been running for {:.1}s", uptime.as_secs_f64());
    ///
    /// // Useful for rate calculations
    /// let (frame_count, _, _) = tcp_reader.buffer_stats();
    /// let avg_rate = frame_count as f64 / uptime.as_secs_f64();
    /// println!("Average data rate: {:.1} frames/sec", avg_rate);
    /// ```
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
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
    pub fn get_data_since(
        &self,
        since: Instant,
    ) -> Vec<TimestampedSignalFrame> {
        let buffer = self.buffer.read();
        buffer
            .iter()
            .filter(|frame| frame.timestamp >= since)
            .cloned()
            .collect()
    }

    /// Get signal data between two timestamps (time window query)
    ///
    /// # Arguments
    /// * `start` - Start of time window (inclusive)
    /// * `end` - End of time window (inclusive)
    ///
    /// # Returns
    /// Vector of timestamped signal frames within the specified time window
    ///
    /// # Thread Safety
    /// Minimizes lock time to avoid blocking the buffering thread.
    ///
    /// # Usage
    /// This is the core method for synchronized data collection during actions.
    /// Typically used to get data before/during/after specific operations.
    pub fn get_data_between(
        &self,
        start: Instant,
        end: Instant,
    ) -> Vec<TimestampedSignalFrame> {
        let buffer = self.buffer.read();
        buffer
            .iter()
            .filter(|frame| frame.timestamp >= start && frame.timestamp <= end)
            .cloned()
            .collect()
    }

    /// Get recent signal data for a specific duration
    ///
    /// # Arguments
    /// * `duration` - How far back to collect data from current time
    ///
    /// # Returns
    /// Vector of timestamped signal frames from the recent past
    ///
    /// # Thread Safety
    /// Delegates to get_data_since() which minimizes lock time.
    ///
    /// # Usage
    /// Convenient for real-time monitoring and getting recent signal history
    /// without needing to track specific timestamps
    pub fn get_recent_data(
        &self,
        duration: Duration,
    ) -> Vec<TimestampedSignalFrame> {
        let since = Instant::now() - duration;
        self.get_data_since(since)
    }

    /// Get all buffered signal data
    ///
    /// # Returns
    /// Vector containing all currently buffered timestamped signal frames
    ///
    /// # Thread Safety
    /// WARNING: This clones the entire buffer. For large buffers, prefer time-windowed queries.
    /// Lock is held briefly but cloning large amounts of data may still impact performance.
    ///
    /// # Usage
    /// Useful for final data collection when stopping buffering, or for
    /// full experiment analysis
    pub fn get_all_data(&self) -> Vec<TimestampedSignalFrame> {
        let buffer = self.buffer.read();
        buffer.iter().cloned().collect()
    }

    /// Get TCP logger configuration that was provided during construction
    ///
    /// # Returns
    /// Tuple of (num_channels, oversampling) from the TCP logger
    ///
    /// # Usage
    /// Needed when converting TimestampedSignalFrame back to TCPLoggerData
    /// for backward compatibility
    pub fn get_tcp_config(&self) -> (u32, f32) {
        (self.num_channels, self.oversampling)
    }

    /// Get buffer statistics for monitoring
    ///
    /// # Returns
    /// Tuple of (current_count, max_capacity, time_span_of_data)
    ///
    /// # Thread Safety
    /// Very brief lock to read buffer metadata only, no cloning.
    ///
    /// # Usage
    /// Useful for monitoring buffer health, detecting overruns, and
    /// understanding the time span of collected data
    pub fn buffer_stats(&self) -> (usize, usize, Duration) {
        let buffer = self.buffer.read();
        let count = buffer.len();
        let capacity = self.max_buffer_size;
        let time_span = if let (Some(first), Some(last)) =
            (buffer.front(), buffer.back())
        {
            last.timestamp.duration_since(first.timestamp)
        } else {
            Duration::ZERO
        };
        (count, capacity, time_span)
    }

    /// Get the most recent N frames from the buffer
    ///
    /// Returns frames in reverse chronological order (newest first).
    /// If fewer than `count` frames are available, returns all available frames.
    ///
    /// # Arguments
    /// * `count` - Maximum number of frames to retrieve
    ///
    /// # Returns
    /// Vector of timestamped signal frames, newest first
    ///
    /// # Example
    /// ```rust,ignore
    /// let recent_100 = tcp_reader.get_recent_frames(100);
    /// ```
    pub fn get_recent_frames(
        &self,
        count: usize,
    ) -> Vec<TimestampedSignalFrame> {
        let buffer = self.buffer.read();
        buffer.iter().rev().take(count).cloned().collect()
    }

    /// Get the oldest N frames from the buffer
    ///
    /// Returns frames in chronological order (oldest first).
    /// If fewer than `count` frames are available, returns all available frames.
    /// Useful for FIFO processing or getting a stable baseline.
    ///
    /// # Arguments
    /// * `count` - Maximum number of frames to retrieve
    ///
    /// # Returns
    /// Vector of timestamped signal frames, oldest first
    ///
    /// # Example
    /// ```rust,ignore
    /// let baseline = tcp_reader.get_oldest_frames(50);
    /// ```
    pub fn get_oldest_frames(
        &self,
        count: usize,
    ) -> Vec<TimestampedSignalFrame> {
        let buffer = self.buffer.read();
        buffer.iter().take(count).cloned().collect()
    }

    /// Get the current number of frames in the buffer
    ///
    /// Returns the total count of frames currently stored in the circular buffer.
    /// This can be used to check buffer fill level or validate requests.
    ///
    /// # Returns
    /// Number of frames currently buffered
    ///
    /// # Example
    /// ```rust,ignore
    /// let available = tcp_reader.frame_count();
    /// if available >= 100 {
    ///     let data = tcp_reader.get_recent_frames(100);
    /// }
    /// ```
    pub fn frame_count(&self) -> usize {
        let buffer = self.buffer.read();
        buffer.len()
    }

    /// Get frames from a specific range in the buffer
    ///
    /// Returns frames starting from `start_idx` (0 = oldest frame) for `count` frames.
    /// If the range extends beyond available data, returns available frames only.
    /// Useful for windowed analysis or specific time periods.
    ///
    /// # Arguments
    /// * `start_idx` - Starting index (0 = oldest frame in buffer)
    /// * `count` - Number of frames to retrieve from start_idx
    ///
    /// # Returns
    /// Vector of timestamped signal frames in chronological order
    ///
    /// # Example
    /// ```rust,ignore
    /// // Get frames 50-149 (middle section of buffer)
    /// let middle_data = tcp_reader.get_frame_range(50, 100);
    /// ```
    pub fn get_frame_range(
        &self,
        start_idx: usize,
        count: usize,
    ) -> Vec<TimestampedSignalFrame> {
        let buffer = self.buffer.read();

        buffer.iter().skip(start_idx).take(count).cloned().collect()
    }

    /// Check if the buffer has at least N frames available
    ///
    /// Convenience method to check data availability before requesting frames.
    /// More efficient than getting frame_count() when you only need a threshold check.
    ///
    /// # Arguments
    /// * `min_count` - Minimum number of frames required
    ///
    /// # Returns
    /// True if buffer contains at least `min_count` frames
    ///
    /// # Example
    /// ```rust,ignore
    /// if tcp_reader.has_frames(100) {
    ///     let stable_data = tcp_reader.get_recent_frames(100);
    /// } else {
    ///     println!("Not enough data yet, only {} frames", tcp_reader.frame_count());
    /// }
    /// ```
    pub fn has_frames(&self, min_count: usize) -> bool {
        self.frame_count() > min_count
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
            match handle.join() {
                Ok(result) => result,
                Err(_) => Err(NanonisError::Protocol(
                    "Buffering thread panicked".to_string(),
                )),
            }
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
