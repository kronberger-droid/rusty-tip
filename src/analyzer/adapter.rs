use std::sync::Arc;

use crate::action::scan::DEFAULT_SCAN_FRAME_KEY;
use crate::action::{Action, ActionContext, ActionOutput};
use crate::spm_error::SpmError;

use super::{Analyzer, AnalyzerInput};

/// Action adapter that bridges an `Analyzer` into the Action system.
///
/// `RunAnalyzer` pulls scan frame data from the `DataStore` (stored by
/// `GrabScanFrame`), feeds it to the wrapped `Analyzer`, and stores the
/// result back into the `DataStore` under the analyzer's name.
///
/// # DataStore contract
///
/// **Reads** `"scan_frame"` key, expecting:
/// ```json
/// { "channel_name": "Z", "data": [[...], ...] }
/// ```
///
/// **Writes** `"<analyzer_name>"` key with the analyzer's output JSON.
pub struct RunAnalyzer {
    analyzer: Arc<dyn Analyzer>,
    /// Optional calibration override (metres per pixel).
    /// If `None`, the adapter does not provide calibration to the analyzer.
    pub calibration_m_per_px: Option<f64>,
    /// DataStore key to read the scan frame from. Allows pairing two
    /// `GrabScanFrame` + `RunAnalyzer` runs in the same workflow (e.g. one
    /// for the forward scan, one for the backward scan) without the second
    /// grab overwriting the first's data before analysis.
    pub source_key: String,
}

impl RunAnalyzer {
    pub fn new(analyzer: Arc<dyn Analyzer>) -> Self {
        Self {
            analyzer,
            calibration_m_per_px: None,
            source_key: DEFAULT_SCAN_FRAME_KEY.into(),
        }
    }

    pub fn with_calibration(mut self, m_per_px: f64) -> Self {
        self.calibration_m_per_px = Some(m_per_px);
        self
    }

    pub fn with_source_key(mut self, key: impl Into<String>) -> Self {
        self.source_key = key.into();
        self
    }
}

impl Action for RunAnalyzer {
    fn name(&self) -> &str {
        self.analyzer.name()
    }

    fn description(&self) -> &str {
        self.analyzer.description()
    }

    fn execute(
        &self,
        ctx: &mut ActionContext,
    ) -> std::result::Result<ActionOutput, SpmError> {
        // Pull scan frame from the DataStore
        let frame: serde_json::Value = ctx
            .store
            .get_raw(&self.source_key)
            .cloned()
            .ok_or_else(|| {
                SpmError::Workflow(format!(
                    "RunAnalyzer: no \"{}\" in DataStore. Run GrabScanFrame \
                     (or set source_key) first.",
                    self.source_key
                ))
            })?;

        let channel_name = frame
            .get("channel_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let data: Vec<Vec<f32>> = frame
            .get("data")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .ok_or_else(|| {
                SpmError::Workflow(format!(
                    "RunAnalyzer: \"{}.data\" missing or not a 2D array",
                    self.source_key
                ))
            })?;

        let input = AnalyzerInput {
            channel_name,
            data,
            calibration_m_per_px: self.calibration_m_per_px,
        };

        let output = self.analyzer.analyze(&input)?;

        // Store the result under the analyzer's name
        ctx.store.set(self.analyzer.name(), &output.data)?;

        Ok(ActionOutput::Data(output.data))
    }
}
