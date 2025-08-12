use crate::{MachineState, NanonisClient, NanonisError, SessionMetadata};
use log::{debug, error, info, trace};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DiskWriterConfig {
    pub file_path: PathBuf,
    pub format: DiskWriterFormat,
    pub buffer_size: usize,
}

pub struct DiskWriterConfigBuilder {
    file_path: Option<PathBuf>,
    format: DiskWriterFormat,
    buffer_size: usize,
}

impl DiskWriterConfig {
    /// Create a new DiskWriterConfig builder with sensible defaults
    pub fn builder() -> DiskWriterConfigBuilder {
        DiskWriterConfigBuilder {
            file_path: None,
            format: DiskWriterFormat::Json { pretty: false },
            buffer_size: 8192, // 8KB default buffer
        }
    }
}

impl DiskWriterConfigBuilder {
    /// Set the file path for the disk writer (required)
    pub fn file_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    /// Set the format for the disk writer (optional, defaults to JSON non-pretty)
    pub fn format(mut self, format: DiskWriterFormat) -> Self {
        self.format = format;
        self
    }

    /// Set the buffer size (optional, defaults to 8KB)
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Build the DiskWriterConfig with validation
    pub fn build(self) -> Result<DiskWriterConfig, String> {
        let file_path = self.file_path.ok_or("file_path is required")?;

        Ok(DiskWriterConfig {
            file_path,
            format: self.format,
            buffer_size: self.buffer_size,
        })
    }
}

#[derive(Debug, Clone)]
pub enum DiskWriterFormat {
    Json { pretty: bool },
    Binary,
}

pub trait DiskWriter: Send + Sync {
    fn write_metadata(&mut self, metadata: SessionMetadata) -> Result<(), std::io::Error>;
    fn write_single(&mut self, data: MachineState) -> Result<(), std::io::Error>;
    fn write_batch(&mut self, data: Vec<MachineState>) -> Result<(), std::io::Error>;
    fn flush(&mut self) -> Result<(), std::io::Error>;
    fn close(&mut self) -> Result<(), std::io::Error>;
}

pub struct JsonDiskWriter {
    file: BufWriter<File>,
    config: DiskWriterConfig,
    samples_written: u64,
}

pub struct JsonDiskWriterBuilder {
    file_path: Option<PathBuf>,
    pretty: bool,
    buffer_size: usize,
}

impl JsonDiskWriter {
    /// Create a JsonDiskWriter from a DiskWriterConfig
    pub fn new(config: DiskWriterConfig) -> Result<Self, std::io::Error> {
        // Validate that JSON format is defined
        if !matches!(config.format, DiskWriterFormat::Json { .. }) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "JsonDiskWriter requires JSON format config",
            ));
        }

        // Create parent directories if they don't exist
        if let Some(parent) = config.file_path.parent() {
            std::fs::create_dir_all(parent)?;
            debug!("Created directory structure for {parent:?}");
        }

        // Create/open the file for writing
        let file = File::create(&config.file_path)?;

        let buffered_writer = BufWriter::with_capacity(config.buffer_size, file);

        info!(
            "Created JSON disk writer: {:?} with {}KB buffer",
            config.file_path,
            config.buffer_size / 1024
        );

        Ok(Self {
            file: buffered_writer,
            config,
            samples_written: 0,
        })
    }

    /// Create a JsonDiskWriter builder with sensible defaults
    pub fn builder() -> JsonDiskWriterBuilder {
        JsonDiskWriterBuilder {
            file_path: None,
            pretty: false,
            buffer_size: 8192, // 8KB default buffer
        }
    }
}

impl JsonDiskWriterBuilder {
    /// Set the file path for the JSON writer (required)
    pub fn file_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    /// Set whether to use pretty formatting (optional, defaults to false)
    pub fn pretty(mut self, pretty: bool) -> Self {
        self.pretty = pretty;
        self
    }

    /// Set the buffer size (optional, defaults to 8KB)
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Build the JsonDiskWriter with validation
    pub fn build(self) -> Result<JsonDiskWriter, std::io::Error> {
        let file_path = self.file_path.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "file_path is required")
        })?;

        let config = DiskWriterConfig {
            file_path,
            format: DiskWriterFormat::Json { pretty: self.pretty },
            buffer_size: self.buffer_size,
        };

        JsonDiskWriter::new(config)
    }
}

impl DiskWriter for JsonDiskWriter {
    fn write_metadata(&mut self, metadata: SessionMetadata) -> Result<(), std::io::Error> {
        // Create metadata file path (same dir, different extension)
        let mut metadata_path = self.config.file_path.clone();
        metadata_path.set_extension("metadata.json");

        // Write metadata to separate file
        let metadata_json = match &self.config.format {
            DiskWriterFormat::Json { pretty: true } => serde_json::to_string_pretty(&metadata)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            DiskWriterFormat::Json { pretty: false } => serde_json::to_string(&metadata)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            _ => unreachable!("JsonDiskWriter with non-JSON format"),
        };

        std::fs::write(metadata_path, metadata_json)?;
        info!(
            "Wrote session metadata: {} signals, {} active",
            metadata.signal_names.len(),
            metadata.active_indices.len()
        );
        Ok(())
    }
    fn write_single(&mut self, data: MachineState) -> Result<(), std::io::Error> {
        // Serialize to JSON based on config (only dynamic data)
        let json_string = match &self.config.format {
            DiskWriterFormat::Json { pretty: true } => serde_json::to_string_pretty(&data)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            DiskWriterFormat::Json { pretty: false } => serde_json::to_string(&data)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            _ => unreachable!("JsonDiskWriter with non-JSON format"),
        };
        self.file.write_all(json_string.as_bytes())?;

        self.file.write_all(b"\n")?;

        self.samples_written += 1;

        trace!(
            "Wrote minimal JSON sample #{} ({} bytes)",
            self.samples_written,
            json_string.len()
        );
        Ok(())
    }

    fn write_batch(&mut self, data: Vec<MachineState>) -> Result<(), std::io::Error> {
        let batch_size = data.len();

        if batch_size == 0 {
            return Ok(());
        }

        info!("Writing batch of {} samples to disk", batch_size);

        for (index, sample) in data.into_iter().enumerate() {
            if let Err(e) = self.write_single(sample) {
                error!("Failed to write sample {index} in batch: {e}");
                return Err(e);
            }
        }

        self.flush()?;
        info!("Successfully wrote batch of {} samples", batch_size);

        Ok(())
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        self.file.flush()?;
        debug!("Flushed JSON writer buffer");
        Ok(())
    }

    fn close(&mut self) -> Result<(), std::io::Error> {
        self.flush()?;

        info!(
            "Closed JSON disk writer: {:?}, {} samples written",
            self.config.file_path, self.samples_written
        );
        Ok(())
    }
}

pub struct SyncSignalMonitor {
    // Client address: ideally on different port than controller client
    nanonis_address: String,
    nanonis_port: u16,

    // Configuration
    signal_indices: Vec<usize>,
    sample_rate: Duration,
    buffer_size: usize,

    // Metadata only (not used for signal processing)
    metadata_primary_signal_index: Option<i32>,

    // Control
    is_running: Arc<AtomicBool>,

    // Shared state
    shared_state: Option<Arc<Mutex<MachineState>>>,

    // Communication channels
    data_sender: Option<mpsc::Sender<MachineState>>,
    shutdown_sender: Option<mpsc::Sender<()>>,

    // Thread handle for cleanup
    monitor_handle: Option<thread::JoinHandle<()>>,

    // Optional disk writer for logging
    disk_writer: Option<Box<dyn DiskWriter>>,
}

/// Handle for receiving monitored signal data
pub struct SignalReceiver {
    /// Receives a continuous stream of MachineState updates
    pub data_receiver: mpsc::Receiver<MachineState>,

    /// Can be used to request shutdown of the monitor
    pub shutdown_sender: mpsc::Sender<()>,

    /// Reference to check if monitor is still running
    pub is_running: Arc<AtomicBool>,
}

/// Statistics about the signal monitor performance
#[derive(Debug, Clone)]
pub struct MonitorStats {
    pub samples_collected: usize,
    pub errors_encountered: usize,
    pub average_sample_interval_ms: f64,
    pub is_running: bool,
}

/// Builder for SyncSignalMonitor with sensible defaults
pub struct SyncSignalMonitorBuilder {
    nanonis_address: String,
    nanonis_port: u16,
    signal_indices: Option<Vec<usize>>,
    sample_rate_hz: f32,
    buffer_size: usize,
    shared_state: Option<Arc<Mutex<MachineState>>>,
    disk_writer: Option<Box<dyn DiskWriter>>,
}

impl SyncSignalMonitor {
    /// Create a new SyncSignalMonitor builder with sensible defaults
    pub fn builder() -> SyncSignalMonitorBuilder {
        SyncSignalMonitorBuilder {
            nanonis_address: "127.0.0.1".to_string(),
            nanonis_port: 6501,
            signal_indices: None,
            sample_rate_hz: 50.0,
            buffer_size: 20, // Default buffer size for signal history
            shared_state: None,
            disk_writer: None,
        }
    }

    pub fn new(
        nanonis_address: &str,
        nanonis_port: u16,
        signal_indices: Vec<usize>,
        sample_rate_hz: f32,
        buffer_size: usize,
    ) -> Result<Self, NanonisError> {
        info!("Created SyncSignalMonitor for {nanonis_address}:{nanonis_port} with {signal_indices:?} at {sample_rate_hz:.1}Hz, buffer_size: {buffer_size}");

        Ok(Self {
            nanonis_address: nanonis_address.to_string(),
            nanonis_port,
            signal_indices,
            sample_rate: Duration::from_millis((1000.0 / sample_rate_hz) as u64),
            buffer_size,
            metadata_primary_signal_index: None,
            is_running: Arc::new(AtomicBool::new(false)),
            shared_state: None,
            data_sender: None,
            shutdown_sender: None,
            monitor_handle: None,
            disk_writer: None,
        })
    }

    /// Set disk writer for logging samples to disk
    pub fn with_disk_writer(mut self, writer: Box<dyn DiskWriter>) -> Self {
        self.disk_writer = Some(writer);
        self
    }

    /// Set shared state for coordinated updates
    pub fn with_shared_state(mut self, shared_state: Arc<Mutex<MachineState>>) -> Self {
        self.shared_state = Some(shared_state);
        self
    }

    /// Start monitoring signals in background thread
    /// Set primary signal index for metadata (called by controller based on classifier knowledge)
    pub fn set_primary_signal_for_metadata(&mut self, primary_signal_index: i32) -> &mut Self {
        // Store this for metadata writing, but don't use it for signal processing
        self.metadata_primary_signal_index = Some(primary_signal_index);
        self
    }

    pub fn start_monitoring(&mut self) -> Result<SignalReceiver, NanonisError> {
        if self.is_running.load(Ordering::Relaxed) {
            return Err(NanonisError::InvalidCommand(
                "Monitor already running".to_string(),
            ));
        }

        // Create bounded channels
        let _channel_buffer_size = 1000; // Channel buffer size
        let (data_sender, data_receiver) = mpsc::channel();
        let (shutdown_sender, shutdown_receiver) = mpsc::channel();

        // Clone data for the thread
        let client_address = self.nanonis_address.clone();
        let client_port = self.nanonis_port;
        let signal_indices = self.signal_indices.clone();
        let metadata_primary_signal_index = self.metadata_primary_signal_index;
        let sample_interval = self.sample_rate;
        let signal_buffer_size = self.buffer_size;
        let is_running = self.is_running.clone();
        let data_sender_clone = data_sender.clone();
        let shared_state = self.shared_state.clone();
        let disk_writer = self.disk_writer.take(); // Take ownership of disk writer

        let monitor_handle = thread::spawn(move || {
            info!("Creating client connection in monitoring thread: {client_address}:{client_port}");

            match NanonisClient::new(&client_address, client_port) {
                Ok(client) => {
                    info!("Client connected, starting monitoring loop");
                    monitoring_task_sync(
                        client,
                        data_sender_clone,
                        shutdown_receiver,
                        MonitoringConfig {
                            signal_indices,
                            metadata_primary_signal_index,
                            sample_interval,
                            signal_buffer_size,
                            is_running,
                            shared_state,
                            disk_writer,
                        },
                    );
                }
                Err(e) => {
                    error!("Failed to create monitoring client: {e}");
                    is_running.store(false, Ordering::Relaxed);
                }
            }
        });

        // Store data for handles
        self.data_sender = Some(data_sender);
        self.shutdown_sender = Some(shutdown_sender.clone());
        self.monitor_handle = Some(monitor_handle);
        self.is_running.store(true, Ordering::Relaxed);

        Ok(SignalReceiver {
            data_receiver,
            shutdown_sender,
            is_running: self.is_running.clone(),
        })
    }
}

struct MonitoringConfig {
    signal_indices: Vec<usize>,
    metadata_primary_signal_index: Option<i32>,
    sample_interval: Duration,
    signal_buffer_size: usize,
    is_running: Arc<AtomicBool>,
    shared_state: Option<Arc<Mutex<MachineState>>>,
    disk_writer: Option<Box<dyn DiskWriter>>,
}

fn monitoring_task_sync(
    mut client: NanonisClient,
    data_sender: mpsc::Sender<MachineState>,
    shutdown_receiver: mpsc::Receiver<()>,
    mut config: MonitoringConfig,
) {
    let mut sample_buffer = Vec::<MachineState>::with_capacity(1000); // Disk write buffer

    // Create and write session metadata once at startup
    if let Some(ref mut writer) = config.disk_writer {
        match client.signal_names_get(false) {
            Ok(signal_names) => {
                let session_start = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64();

                let metadata = SessionMetadata {
                    session_id: chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string(),
                    signal_names,
                    active_indices: config.signal_indices.clone(),
                    primary_signal_index: config.metadata_primary_signal_index.unwrap_or(0) as usize,
                    session_start,
                };

                if let Err(e) = writer.write_metadata(metadata) {
                    error!("Failed to write session metadata: {e}");
                }
            }
            Err(e) => {
                error!("Failed to fetch signal names: {e}");
            }
        }
    }

    loop {
        // Check for shutdown signal (non-blocking)
        if shutdown_receiver.try_recv().is_ok() {
            info!("Shutdown signal received");
            config.is_running.store(false, Ordering::Relaxed);
            break;
        }

        if config.is_running.load(Ordering::Relaxed) {
            // Read signals from Nanonis
            let signal_indices_i32: Vec<i32> = config.signal_indices.iter().map(|&i| i as i32).collect();
            match client.signals_val_get(signal_indices_i32, true) {
                Ok(values) => {
                    let current_time = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs_f64();

                    let machine_state = if let Some(ref shared_state) = config.shared_state {
                        // Update shared state with new signal data - signal-agnostic approach
                        {
                            if let Ok(mut state) = shared_state.lock() {
                                // Populate signal mapping and all signals
                                state.all_signals = Some(values.clone());
                                state.signal_indices = Some(config.signal_indices.iter().map(|&i| i as i32).collect());
                                state.timestamp = current_time;
                                
                                // SIMPLIFIED: SignalMonitor doesn't know about "primary" signals anymore
                                // It just maintains a rolling buffer of the most recent signal readings
                                // The classifier will use its own knowledge to extract what it needs
                                
                                // Keep signal_history as a simple rolling buffer of recent values
                                // For now, we can use the first signal as a simple stream, but really
                                // the classifier should extract its own signal from all_signals using signal_indices
                                if !values.is_empty() {
                                    state.signal_history.push_back(values[0]); // Just as a placeholder stream
                                    if state.signal_history.len() > config.signal_buffer_size {
                                        state.signal_history.pop_front();
                                    }
                                    trace!("Updated signal_history with signal stream (placeholder: {})", values[0]);
                                }
                            }
                        }

                        // Get complete enriched state for writing/sending
                        if let Ok(state) = shared_state.lock() {
                            state.clone()
                        } else {
                            // Fallback if lock failed
                            let mut fallback_state = MachineState {
                                all_signals: Some(values.clone()),
                                signal_indices: Some(config.signal_indices.iter().map(|&i| i as i32).collect()),
                                timestamp: current_time,
                                ..Default::default()
                            };
                            
                            // Simple signal stream for fallback (classifier will extract what it needs)
                            if !values.is_empty() {
                                fallback_state.signal_history.push_back(values[0]);
                            }
                            
                            fallback_state
                        }
                    } else {
                        // Fallback: create basic MachineState (for backwards compatibility)
                        let mut fallback_state = MachineState {
                            all_signals: Some(values.clone()),
                            signal_indices: Some(config.signal_indices.iter().map(|&i| i as i32).collect()),
                            timestamp: current_time,
                            ..Default::default()
                        };
                        
                        // Simple signal stream for fallback (classifier will extract what it needs)
                        if !values.is_empty() {
                            fallback_state.signal_history.push_back(values[0]);
                        }
                        
                        fallback_state
                    };

                    // Add to buffer and write batch when full
                    if let Some(ref mut writer) = config.disk_writer {
                        sample_buffer.push(machine_state.clone());

                        // Write batch when buffer is full
                        if sample_buffer.len() >= 1000 {
                            trace!("Writing batch of {} samples to disk", sample_buffer.len());
                            let _ = writer.write_batch(sample_buffer.clone());
                            sample_buffer.clear();
                        }
                    }

                    // Send to channel (always send individual samples)
                    let _ = data_sender.send(machine_state);
                }
                Err(e) => {
                    error!("Failed to read signals: {e}");
                    // If we get connection errors like "Broken pipe", stop the loop
                    // as continuing will just spam errors
                    if e.to_string().contains("Broken pipe") || e.to_string().contains("failed to fill whole buffer") {
                        error!("Connection lost, stopping signal monitoring");
                        config.is_running.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            }
        }

        // Sleep for sample interval
        thread::sleep(config.sample_interval);
    }

    // Cleanup: write remaining samples in buffer
    if let Some(ref mut writer) = config.disk_writer {
        if !sample_buffer.is_empty() {
            info!(
                "Writing final batch of {} samples to disk",
                sample_buffer.len()
            );
            if let Err(e) = writer.write_batch(sample_buffer) {
                error!("Failed to write final batch: {e}");
            }
        }
        if let Err(e) = writer.close() {
            error!("Failed to close disk writer: {e}");
        }
    }
    config.is_running.store(false, Ordering::Relaxed);
    info!("Monitoring task cleanup completed");
}

impl SyncSignalMonitorBuilder {
    /// Set the Nanonis server address
    pub fn address(mut self, address: impl Into<String>) -> Self {
        self.nanonis_address = address.into();
        self
    }

    /// Set the Nanonis server port
    pub fn port(mut self, port: u16) -> Self {
        self.nanonis_port = port;
        self
    }

    /// Set the signal indices to monitor (required)
    pub fn signals(mut self, signal_indices: Vec<usize>) -> Self {
        self.signal_indices = Some(signal_indices);
        self
    }


    /// Set the sample rate in Hz
    pub fn sample_rate(mut self, sample_rate_hz: f32) -> Self {
        self.sample_rate_hz = sample_rate_hz;
        self
    }

    /// Set the signal buffer size for classifier
    pub fn buffer_size(mut self, buffer_size: usize) -> Self {
        self.buffer_size = buffer_size;
        self
    }

    /// Set shared state for coordinated updates (optional)
    pub fn with_shared_state(mut self, shared_state: Arc<Mutex<MachineState>>) -> Self {
        self.shared_state = Some(shared_state);
        self
    }

    /// Set disk writer for logging (optional)
    pub fn with_disk_writer(mut self, writer: Box<dyn DiskWriter>) -> Self {
        self.disk_writer = Some(writer);
        self
    }

    /// Build the SyncSignalMonitor with validation
    pub fn build(self) -> Result<SyncSignalMonitor, String> {
        let signal_indices = self.signal_indices
            .ok_or("signal_indices is required - use .signals(vec![...])")?;

        if signal_indices.is_empty() {
            return Err("signal_indices cannot be empty".to_string());
        }

        if self.sample_rate_hz <= 0.0 {
            return Err("sample_rate_hz must be greater than 0".to_string());
        }

        if self.sample_rate_hz > 1000.0 {
            return Err("sample_rate_hz should not exceed 1000 Hz for stability".to_string());
        }

        let monitor = SyncSignalMonitor {
            nanonis_address: self.nanonis_address,
            nanonis_port: self.nanonis_port,
            signal_indices,
            sample_rate: Duration::from_millis((1000.0 / self.sample_rate_hz) as u64),
            buffer_size: self.buffer_size,
            metadata_primary_signal_index: None,
            is_running: Arc::new(AtomicBool::new(false)),
            shared_state: self.shared_state,
            data_sender: None,
            shutdown_sender: None,
            monitor_handle: None,
            disk_writer: self.disk_writer,
        };

        info!(
            "Built SyncSignalMonitor for {}:{} with {:?} at {:.1}Hz, buffer_size: {}",
            monitor.nanonis_address, monitor.nanonis_port, monitor.signal_indices, self.sample_rate_hz, monitor.buffer_size
        );

        Ok(monitor)
    }
}
