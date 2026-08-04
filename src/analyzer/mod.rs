mod adapter;
pub mod cuox_rows;

pub use adapter::RunAnalyzer;
pub use cuox_rows::CuoxRowDetector;

use crate::spm_error::SpmError;

type Result<T> = std::result::Result<T, SpmError>;

/// Input data for an analyzer.
///
/// Wraps a 2D scan frame (row-major f32 pixels) together with metadata
/// that analyzers may need for calibrated measurements.
pub struct AnalyzerInput {
    /// Channel name from the scan buffer (e.g. "Z", "Current").
    pub channel_name: String,
    /// 2D pixel data in row-major order (rows x cols).
    pub data: Vec<Vec<f32>>,
    /// Physical size of one pixel in metres, if known.
    /// Computed from scan frame size / pixel count.
    pub calibration_m_per_px: Option<f64>,
}

impl AnalyzerInput {
    /// Number of rows (height) in the image.
    pub fn rows(&self) -> usize {
        self.data.len()
    }

    /// Number of columns (width) in the image.
    pub fn cols(&self) -> usize {
        self.data.first().map_or(0, |r| r.len())
    }
}

/// Result returned by an analyzer.
///
/// Analyzers produce structured JSON output that can be stored in the
/// `DataStore`, logged via the event system, or inspected by an LLM.
pub struct AnalyzerOutput {
    /// Structured result data (schema is analyzer-specific).
    pub data: serde_json::Value,
    /// Optional annotated image as raw pixels (e.g. with detected edges drawn).
    /// Format: row-major RGBA, `width x height x 4` bytes.
    pub annotated_image: Option<AnnotatedImage>,
}

/// An annotated image produced by an analyzer.
pub struct AnnotatedImage {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA pixel data.
    pub rgba: Vec<u8>,
}

/// Pure-computation trait for analyzing scan data.
///
/// Analyzers take 2D scan frame data and produce structured results
/// (detected features, measurements, classifications, etc.) without
/// any hardware interaction.  This makes them trivially testable and
/// reusable across different execution contexts (live experiments,
/// offline batch processing, GUI previews).
///
/// # Implementing an Analyzer
///
/// ```ignore
/// struct MyDetector { threshold: f32 }
///
/// impl Analyzer for MyDetector {
///     fn name(&self) -> &str { "my_detector" }
///     fn description(&self) -> &str { "Detects features in scan data" }
///     fn analyze(&self, input: &AnalyzerInput) -> Result<AnalyzerOutput> {
///         // ... pure computation over input.data ...
///     }
/// }
/// ```
pub trait Analyzer: Send + Sync {
    /// Unique identifier, e.g. "cuox_row_detector".
    fn name(&self) -> &str;

    /// Human-readable description for documentation and LLM context.
    fn description(&self) -> &str;

    /// Run the analysis on the given input data.
    fn analyze(&self, input: &AnalyzerInput) -> Result<AnalyzerOutput>;
}
