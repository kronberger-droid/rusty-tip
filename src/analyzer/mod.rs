pub mod cuox_rows;

pub use cuox_rows::CuoxRowDetector;

use crate::frame::Frame;
use crate::spm_error::SpmError;

type Result<T> = std::result::Result<T, SpmError>;

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

/// Pure-computation trait for analyzing scan frames.
///
/// Analyzers take a [`Frame`] and produce structured results (detected
/// features, measurements) without any hardware interaction, which makes
/// them trivially testable and reusable across execution contexts (live
/// experiments, offline batch processing, GUI previews). Routines call
/// them as plain functions on frames grabbed via the scan handle.
///
/// # Implementing an Analyzer
///
/// ```ignore
/// struct MyDetector { threshold: f32 }
///
/// impl Analyzer for MyDetector {
///     fn name(&self) -> &str { "my_detector" }
///     fn description(&self) -> &str { "Detects features in scan data" }
///     fn analyze(&self, frame: &Frame) -> Result<AnalyzerOutput> {
///         // ... pure computation over frame.data ...
///     }
/// }
/// ```
pub trait Analyzer: Send + Sync {
    /// Unique identifier, e.g. "cuox_row_detector".
    fn name(&self) -> &str;

    /// Human-readable description for documentation and LLM context.
    fn description(&self) -> &str;

    /// Run the analysis on the given frame.
    fn analyze(&self, frame: &Frame) -> Result<AnalyzerOutput>;
}
