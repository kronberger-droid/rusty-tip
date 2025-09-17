use chrono::{DateTime, Utc};
use log::info;
use serde::{Deserialize, Serialize};
use std::{io::Write, path::PathBuf};

use crate::error::NanonisError;

#[derive(Debug, Serialize, Deserialize)]
struct LogEntry<T>
where
    T: Serialize,
{
    timestamp: DateTime<Utc>,
    data: T,
}

#[derive(Debug)]
pub struct Logger<T>
where
    T: Serialize,
{
    buffer: Vec<LogEntry<T>>,
    buffer_size: usize,
    file_path: PathBuf,
}

impl<T> Logger<T>
where
    T: Serialize,
{
    pub fn new<P: Into<PathBuf>>(file_path: P, buffer_size: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(buffer_size),
            buffer_size,
            file_path: file_path.into(),
        }
    }

    pub fn add(&mut self, data: T) -> Result<(), NanonisError> {
        let entry = LogEntry {
            timestamp: Utc::now(),
            data,
        };

        self.buffer.push(entry);

        if self.buffer.len() >= self.buffer_size {
            self.flush()?;
        }

        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), NanonisError> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)
            .map_err(|source| NanonisError::Io {
                source,
                context: format!(
                    "Logger could not create file at {:?}",
                    self.file_path
                ),
            })?;

        let mut writer = std::io::BufWriter::new(file);

        for entry in &self.buffer {
            let json_line = serde_json::to_string(entry)?;
            writeln!(writer, "{}", json_line)?;
        }

        writer.flush()?;
        self.buffer.clear();
        info!("Logger flushed successfully to file");
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.len() == 0
    }
}

impl<T> Drop for Logger<T>
where
    T: Serialize,
{
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
