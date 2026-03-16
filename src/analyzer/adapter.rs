use std::sync::Arc;

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
}

impl RunAnalyzer {
    pub fn new(analyzer: Arc<dyn Analyzer>) -> Self {
        Self {
            analyzer,
            calibration_m_per_px: None,
        }
    }

    pub fn with_calibration(mut self, m_per_px: f64) -> Self {
        self.calibration_m_per_px = Some(m_per_px);
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

    fn execute(&self, ctx: &mut ActionContext) -> std::result::Result<ActionOutput, SpmError> {
        // Pull scan frame from the DataStore
        let frame: serde_json::Value = ctx
            .store
            .get_raw("scan_frame")
            .cloned()
            .ok_or_else(|| {
                SpmError::Workflow(
                    "RunAnalyzer: no \"scan_frame\" in DataStore. \
                     Run GrabScanFrame first."
                        .into(),
                )
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
                SpmError::Workflow(
                    "RunAnalyzer: \"scan_frame.data\" missing or not a 2D array".into(),
                )
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
