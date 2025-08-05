use crate::{MachineState, NanonisClient, NanonisError};
use async_trait::async_trait;
use log::{debug, error, info, trace};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::{sync::mpsc, time::Duration};

#[derive(Debug, Clone)]
pub struct DiskWriterConfig {
    pub file_path: PathBuf,
    pub format: DiskWriterFormat,
    pub buffer_size: usize,
}

#[derive(Debug, Clone)]
pub enum DiskWriterFormat {
    Json { pretty: bool },
    Binary,
}

#[async_trait]
pub trait DiskWriter: Send + Sync {
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

impl JsonDiskWriter {
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
}

#[async_trait]
impl DiskWriter for JsonDiskWriter {
    async fn write_single(&mut self, data: MachineState) -> Result<(), std::io::Error> {
        // Serialize to JSON based on config
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
            "Wrote JSON sample #{} ({} bytes)",
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
    // Client address: idealy on different port than controller client
    nanonis_address: String,
    monitor_port: u16,

    // Configuration
    signal_indices: Vec<i32>,
    sample_rate: Duration,

    // Control
    is_running: Arc<AtomicBool>,

    // Communication channels
    data_sender: Option<mpsc::Sender<MachineState>>,
    shutdown_sender: Option<mpsc::Sender<()>>,

    // Task handle for cleanup
    monitor_handle: Option<tokio::task::JoinHandle<()>>,
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

impl AsyncSignalMonitor {
    pub fn new(
        nanonis_address: &str,
        monitor_port: u16,
        signal_indices: Vec<i32>,
        sample_rate_hz: f32,
    ) -> Result<Self, NanonisError> {
        info!("Created AsyncSignalMonitor for {nanonis_address}:{monitor_port} with {signal_indices:?} at {sample_rate_hz:.1}Hz");

        Ok(Self {
            nanonis_address: nanonis_address.to_string(),
            monitor_port,
            signal_indices,
            sample_rate: Duration::from_millis((1000.0 / sample_rate_hz) as u64),
            is_running: Arc::new(AtomicBool::new(false)),
            data_sender: None,
            shutdown_sender: None,
            monitor_handle: None,
        })
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
        let client_port = self.monitor_port;
        let signal_indices = self.signal_indices.clone();
        let sample_interval = self.sample_rate;
        let is_running = self.is_running.clone();
        let data_sender_clone = data_sender.clone();

        let monitor_handle = tokio::spawn(async move {
            info!("Creating client connection in monitoring task: {client_address}:{client_port}");

            match NanonisClient::new(&client_address, &client_port.to_string()) {
                Ok(client) => {
                    info!("Client connected, starting monitoring loop");
                    monitoring_task(
                        client,
                        data_sender_clone,
                        shutdown_receiver,
                        signal_indices,
                        sample_interval,
                        buffer_size,
                        is_running,
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

#[allow(unused)]
async fn monitoring_task(
    mut client: NanonisClient,
    data_sender: mpsc::Sender<MachineState>,
    mut shutdown_receiver: mpsc::Receiver<()>,
    signal_indices: Vec<i32>,
    sample_interval: Duration,
    buffer_size: usize,
    is_running: Arc<AtomicBool>,
) {
    // Implementation here
}
