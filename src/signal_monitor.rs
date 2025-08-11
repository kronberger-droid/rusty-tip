use crate::{MachineState, NanonisClient, NanonisError, SessionMetadata};
use async_trait::async_trait;
use log::{debug, error, info, trace};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::RwLock;
use tokio::{sync::mpsc, time::Duration};

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

#[async_trait]
pub trait DiskWriter: Send + Sync {
    async fn write_metadata(&mut self, metadata: SessionMetadata) -> Result<(), std::io::Error>;
    async fn write_single(&mut self, data: MachineState) -> Result<(), std::io::Error>;
    async fn write_batch(&mut self, data: Vec<MachineState>) -> Result<(), std::io::Error>;
    async fn flush(&mut self) -> Result<(), std::io::Error>;
    async fn close(&mut self) -> Result<(), std::io::Error>;
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
    pub async fn new(config: DiskWriterConfig) -> Result<Self, std::io::Error> {
        // Validate that JSON format is defined
        if !matches!(config.format, DiskWriterFormat::Json { .. }) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "JsonDiskWriter requires JSON format config",
            ));
        }

        // Create parent directories if they don't exist
        if let Some(parent) = config.file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
            debug!("Created directory structure for {parent:?}");
        }

        // Create/open the file for writing
        let file = File::create(&config.file_path).await?;

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
    pub async fn build(self) -> Result<JsonDiskWriter, std::io::Error> {
        let file_path = self.file_path.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "file_path is required")
        })?;

        let config = DiskWriterConfig {
            file_path,
            format: DiskWriterFormat::Json { pretty: self.pretty },
            buffer_size: self.buffer_size,
        };

        JsonDiskWriter::new(config).await
    }
}

#[async_trait]
impl DiskWriter for JsonDiskWriter {
    async fn write_metadata(&mut self, metadata: SessionMetadata) -> Result<(), std::io::Error> {
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

        tokio::fs::write(metadata_path, metadata_json).await?;
        info!(
            "Wrote session metadata: {} signals, {} active",
            metadata.signal_names.len(),
            metadata.active_indices.len()
        );
        Ok(())
    }
    async fn write_single(&mut self, data: MachineState) -> Result<(), std::io::Error> {
        // Serialize to JSON based on config (only dynamic data)
        let json_string = match &self.config.format {
            DiskWriterFormat::Json { pretty: true } => serde_json::to_string_pretty(&data)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            DiskWriterFormat::Json { pretty: false } => serde_json::to_string(&data)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            _ => unreachable!("JsonDiskWriter with non-JSON format"),
        };
        self.file.write_all(json_string.as_bytes()).await?;

        self.file.write_all(b"\n").await?;

        self.samples_written += 1;

        trace!(
            "Wrote minimal JSON sample #{} ({} bytes)",
            self.samples_written,
            json_string.len()
        );
        Ok(())
    }

    async fn write_batch(&mut self, data: Vec<MachineState>) -> Result<(), std::io::Error> {
        let batch_size = data.len();

        if batch_size == 0 {
            return Ok(());
        }

        info!("Writing batch of {} samples to disk", batch_size);

        for (index, sample) in data.into_iter().enumerate() {
            if let Err(e) = self.write_single(sample).await {
                error!("Failed to write sample {index} in batch: {e}");
                return Err(e);
            }
        }

        self.flush().await?;
        info!("Successfully wrote batch of {} samples", batch_size);

        Ok(())
    }

    async fn flush(&mut self) -> Result<(), std::io::Error> {
        self.file.flush().await?;
        debug!("Flushed JSON writer buffer");
        Ok(())
    }

    async fn close(&mut self) -> Result<(), std::io::Error> {
        self.flush().await?;

        info!(
            "Closed JSON disk writer: {:?}, {} samples written",
            self.config.file_path, self.samples_written
        );
        Ok(())
    }
}

pub struct AsyncSignalMonitor {
    // Client address: ideally on different port than controller client
    nanonis_address: String,
    nanonis_port: u16,

    // Configuration
    signal_indices: Vec<usize>,
    sample_rate: Duration,

    // Control
    is_running: Arc<AtomicBool>,

    // Shared state
    shared_state: Option<Arc<RwLock<MachineState>>>,

    // Communication channels
    data_sender: Option<mpsc::Sender<MachineState>>,
    shutdown_sender: Option<mpsc::Sender<()>>,

    // Task handle for cleanup
    monitor_handle: Option<tokio::task::JoinHandle<()>>,

    // Optional disk writer for logging
    disk_writer: Option<Box<dyn DiskWriter>>,
}

/// Handle for reveiving monitored signal data
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

/// Builder for AsyncSignalMonitor with sensible defaults
pub struct AsyncSignalMonitorBuilder {
    nanonis_address: String,
    nanonis_port: u16,
    signal_indices: Option<Vec<usize>>,
    sample_rate_hz: f32,
    shared_state: Option<Arc<RwLock<MachineState>>>,
    disk_writer: Option<Box<dyn DiskWriter>>,
}

impl AsyncSignalMonitor {
    /// Create a new AsyncSignalMonitor builder with sensible defaults
    pub fn builder() -> AsyncSignalMonitorBuilder {
        AsyncSignalMonitorBuilder {
            nanonis_address: "127.0.0.1".to_string(),
            nanonis_port: 6501,
            signal_indices: None,
            sample_rate_hz: 50.0,
            shared_state: None,
            disk_writer: None,
        }
    }

    pub fn new(
        nanonis_address: &str,
        nanonis_port: u16,
        signal_indices: Vec<usize>,
        sample_rate_hz: f32,
    ) -> Result<Self, NanonisError> {
        info!("Created AsyncSignalMonitor for {nanonis_address}:{nanonis_port} with {signal_indices:?} at {sample_rate_hz:.1}Hz");

        Ok(Self {
            nanonis_address: nanonis_address.to_string(),
            nanonis_port,
            signal_indices,
            sample_rate: Duration::from_millis((1000.0 / sample_rate_hz) as u64),
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
    pub fn with_shared_state(mut self, shared_state: Arc<RwLock<MachineState>>) -> Self {
        self.shared_state = Some(shared_state);
        self
    }

    /// Start monitoring signals in background task
    pub async fn start_monitoring(&mut self) -> Result<SignalReceiver, NanonisError> {
        if self.is_running.load(Ordering::Relaxed) {
            return Err(NanonisError::InvalidCommand(
                "Monitor already running".to_string(),
            ));
        }

        // Create bounded channels
        let buffer_size = 1000; // Default buffer size
        let (data_sender, data_receiver) = mpsc::channel(buffer_size);
        let (shutdown_sender, shutdown_receiver) = mpsc::channel(1);

        // Clone data for the task
        let client_address = self.nanonis_address.clone();
        let client_port = self.nanonis_port;
        let signal_indices = self.signal_indices.clone();
        let sample_interval = self.sample_rate;
        let is_running = self.is_running.clone();
        let data_sender_clone = data_sender.clone();
        let shared_state = self.shared_state.clone();
        let disk_writer = self.disk_writer.take(); // Take ownership of disk writer

        let monitor_handle = tokio::spawn(async move {
            info!("Creating client connection in monitoring task: {client_address}:{client_port}");

            match NanonisClient::new(&client_address, client_port) {
                Ok(client) => {
                    info!("Client connected, starting monitoring loop");
                    monitoring_task(
                        client,
                        data_sender_clone,
                        shutdown_receiver,
                        MonitoringConfig {
                            signal_indices,
                            sample_interval,
                            buffer_size,
                            is_running,
                            shared_state,
                            disk_writer,
                        },
                    )
                    .await;
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
    sample_interval: Duration,
    buffer_size: usize,
    is_running: Arc<AtomicBool>,
    shared_state: Option<Arc<RwLock<MachineState>>>,
    disk_writer: Option<Box<dyn DiskWriter>>,
}

async fn monitoring_task(
    mut client: NanonisClient,
    data_sender: mpsc::Sender<MachineState>,
    mut shutdown_receiver: mpsc::Receiver<()>,
    mut config: MonitoringConfig,
) {
    let mut sample_buffer = Vec::<MachineState>::with_capacity(config.buffer_size);
    let mut interval = tokio::time::interval(config.sample_interval);

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
                    primary_signal_index: config.signal_indices.first().copied().unwrap_or(0),
                    session_start,
                };

                if let Err(e) = writer.write_metadata(metadata).await {
                    error!("Failed to write session metadata: {e}");
                }
            }
            Err(e) => {
                error!("Failed to fetch signal names: {e}");
            }
        }
    }

    loop {
        tokio::select! {
            _ = shutdown_receiver.recv() => {
                info!("Shutdown signal received");
                config.is_running.store(false, std::sync::atomic::Ordering::Relaxed);
                break;
            }
            _ = interval.tick(), if config.is_running.load(Ordering::Relaxed) => {
                // Read signals from Nanonis
                let signal_indices_i32: Vec<i32> = config.signal_indices.iter().map(|&i| i as i32).collect();
                match client.signals_val_get(signal_indices_i32, true) {
                    Ok(values) => {
                        let current_time = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs_f64();

                        let machine_state = if let Some(ref shared_state) = config.shared_state {
                            // Update shared state with new signal data
                            {
                                let mut state = shared_state.write().await;
                                state.primary_signal = values[0];
                                state.all_signals = Some(values.clone());
                                state.timestamp = current_time;
                            }

                            // Get complete enriched state for writing/sending
                            shared_state.read().await.clone()
                        } else {
                            // Fallback: create basic MachineState (for backwards compatibility)
                            MachineState {
                                primary_signal: values[0],
                                all_signals: Some(values),
                                timestamp: current_time,
                                ..Default::default()
                            }
                        };

                        // Add to buffer and write batch when full
                        if let Some(ref mut writer) = config.disk_writer {
                            sample_buffer.push(machine_state.clone());

                            // Write batch when buffer is full
                            if sample_buffer.len() >= config.buffer_size {
                                trace!("Writing batch of {} samples to disk", sample_buffer.len());
                                let _ = writer.write_batch(sample_buffer.clone()).await;
                                sample_buffer.clear();
                            }
                        }

                        // Send to channel (always send individual samples)
                        let _ = data_sender.send(machine_state).await;
                    }
                    Err(e) => {
                        error!("Failed to read signals: {e}");
                        // If we get connection errors like "Broken pipe", stop the loop
                        // as continuing will just spam errors
                        if e.to_string().contains("Broken pipe") || e.to_string().contains("failed to fill whole buffer") {
                            error!("Connection lost, stopping signal monitoring");
                            config.is_running.store(false, std::sync::atomic::Ordering::Relaxed);
                            break;
                        }
                    }
                }
            }
        }
    }

    // Cleanup: write remaining samples in buffer
    if let Some(ref mut writer) = config.disk_writer {
        if !sample_buffer.is_empty() {
            info!(
                "Writing final batch of {} samples to disk",
                sample_buffer.len()
            );
            if let Err(e) = writer.write_batch(sample_buffer).await {
                error!("Failed to write final batch: {e}");
            }
        }
        if let Err(e) = writer.close().await {
            error!("Failed to close disk writer: {e}");
        }
    }
    config.is_running.store(false, Ordering::Relaxed);
    info!("Monitoring task cleanup completed");
}

impl AsyncSignalMonitorBuilder {
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

    /// Set shared state for coordinated updates (optional)
    pub fn with_shared_state(mut self, shared_state: Arc<RwLock<MachineState>>) -> Self {
        self.shared_state = Some(shared_state);
        self
    }

    /// Set disk writer for logging (optional)
    pub fn with_disk_writer(mut self, writer: Box<dyn DiskWriter>) -> Self {
        self.disk_writer = Some(writer);
        self
    }

    /// Build the AsyncSignalMonitor with validation
    pub fn build(self) -> Result<AsyncSignalMonitor, String> {
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

        let monitor = AsyncSignalMonitor {
            nanonis_address: self.nanonis_address,
            nanonis_port: self.nanonis_port,
            signal_indices,
            sample_rate: Duration::from_millis((1000.0 / self.sample_rate_hz) as u64),
            is_running: Arc::new(AtomicBool::new(false)),
            shared_state: self.shared_state,
            data_sender: None,
            shutdown_sender: None,
            monitor_handle: None,
            disk_writer: self.disk_writer,
        };

        info!(
            "Built AsyncSignalMonitor for {}:{} with {:?} at {:.1}Hz",
            monitor.nanonis_address, monitor.nanonis_port, monitor.signal_indices, self.sample_rate_hz
        );

        Ok(monitor)
    }
}
