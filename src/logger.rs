use log::info;
use serde::{de::DeserializeOwned, Serialize};
use std::{io::Write, path::PathBuf};

use crate::error::NanonisError;

// Removed LogEntry wrapper - ActionLogEntry already has timestamps

#[derive(Debug)]
pub struct Logger<T>
where
    T: Serialize + Clone + DeserializeOwned,
{
    buffer: Vec<T>,
    buffer_size: usize,
    file_path: PathBuf,
    final_format_json: bool, // If true, convert to JSON on final flush
}

impl<T> Logger<T>
where
    T: Serialize + Clone + DeserializeOwned,
{
    pub fn new<P: Into<PathBuf>>(file_path: P, buffer_size: usize, final_format_json: bool) -> Self {
        let mut path = file_path.into();
        
        // Automatically add appropriate file extension
        if final_format_json {
            // For JSON output, ensure .json extension
            if path.extension().is_none() || path.extension() != Some(std::ffi::OsStr::new("json")) {
                path.set_extension("json");
            }
        } else {
            // For JSONL output, ensure .jsonl extension
            if path.extension().is_none() || path.extension() != Some(std::ffi::OsStr::new("jsonl")) {
                path.set_extension("jsonl");
            }
        }
        
        Self {
            buffer: Vec::with_capacity(buffer_size),
            buffer_size,
            file_path: path,
            final_format_json,
        }
    }

    pub fn add(&mut self, data: T) -> Result<(), NanonisError> {
        self.buffer.push(data);

        if self.buffer.len() >= self.buffer_size {
            self.flush()?;
        }

        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), NanonisError> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        // Always write JSONL for intermediate flushes (efficient)
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

        for data in &self.buffer {
            let json_line = serde_json::to_string(data)?;
            writeln!(writer, "{}", json_line)?;
        }

        writer.flush()?;
        self.buffer.clear();
        info!("Logger flushed successfully to file");
        Ok(())
    }

    /// Convert JSONL file to JSON array format (for final post-experiment analysis)
    pub fn finalize_as_json(&mut self) -> Result<(), NanonisError> {
        if !self.final_format_json {
            return Ok(()); // No conversion needed
        }

        // First flush any remaining buffer
        self.flush()?;

        // Read all JSONL entries
        let content = std::fs::read_to_string(&self.file_path)
            .map_err(|source| NanonisError::Io {
                source,
                context: format!("Could not read JSONL file at {:?}", self.file_path),
            })?;

        let mut entries = Vec::new();
        for line in content.lines() {
            if !line.trim().is_empty() {
                let data: T = serde_json::from_str(line)?;
                entries.push(data);
            }
        }

        // Write as JSON array with pretty formatting
        let json_output = serde_json::to_string_pretty(&entries)?;
        std::fs::write(&self.file_path, json_output)
            .map_err(|source| NanonisError::Io {
                source,
                context: format!("Could not write JSON file at {:?}", self.file_path),
            })?;

        info!("Converted {} entries from JSONL to JSON format", entries.len());
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
    T: Serialize + Clone + DeserializeOwned,
{
    fn drop(&mut self) {
        let _ = self.flush();
        let _ = self.finalize_as_json();
    }
}
