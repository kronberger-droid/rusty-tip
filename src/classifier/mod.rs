//! Classifiers: verdicts on measurement data.
//!
//! A classifier answers a question about a payload (is this tip sharp, is
//! this frame usable) and is consulted where a routine decides what to do
//! next, so its verdicts belong in the audit trail: run them through
//! [`Rt::classify`](crate::routine::Rt::classify), which logs model,
//! version, latency and the verdict itself.
//!
//! The trait separates *what a classifier answers* from *how it is
//! reached*. [`HttpClassifier`] talks to a Python sidecar over HTTP (one
//! verdict struct + an endpoint URL per new model, no transport code);
//! [`MockClassifier`] scripts verdicts for tests; local Rust classifiers
//! implement the trait directly.

pub mod http;
pub mod mock;

pub use http::HttpClassifier;
pub use mock::MockClassifier;

use serde::{Deserialize, Serialize};

use crate::frame::ToNpyPayload;
use crate::spm_error::SpmError;

/// Identity of the model behind a classifier, for the event log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub version: String,
}

/// A verdict-producing model, local or remote.
///
/// `classify` takes `&mut self` so implementations may keep state across
/// calls (drift estimates, degradation tracking); stateless classifiers
/// simply don't use it.
pub trait Classifier {
    /// What this classifier consumes. Anything npy-encodable works, so
    /// one transport carries frames today and line scans or signal
    /// windows later.
    type Input: ToNpyPayload;

    /// What it answers. `Serialize` so [`Rt::classify`]
    /// (crate::routine::Rt::classify) can put the real verdict into the
    /// event log, not a summary.
    type Verdict: Serialize;

    /// The model producing the verdicts. Known before the first call:
    /// remote transports resolve it at connect time.
    fn model(&self) -> &ModelInfo;

    fn classify(&mut self, input: &Self::Input) -> Result<Self::Verdict, SpmError>;
}
