//! Scripted classifier for tests, in the spirit of `MockController`.

use std::collections::VecDeque;
use std::marker::PhantomData;

use serde::Serialize;

use super::{Classifier, ModelInfo};
use crate::frame::ToNpyPayload;
use crate::spm_error::SpmError;

/// Returns scripted verdicts in order and records the metadata of every
/// payload it was shown.
pub struct MockClassifier<I, V> {
    model: ModelInfo,
    verdicts: VecDeque<Result<V, SpmError>>,
    /// Metadata of each classified payload, in call order.
    pub seen: Vec<serde_json::Value>,
    _marker: PhantomData<fn(&I)>,
}

impl<I: ToNpyPayload, V: Serialize> MockClassifier<I, V> {
    pub fn new() -> Self {
        Self {
            model: ModelInfo {
                name: "mock".into(),
                version: "0".into(),
            },
            verdicts: VecDeque::new(),
            seen: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Queue a verdict for the next unanswered `classify` call.
    pub fn push_verdict(mut self, verdict: V) -> Self {
        self.verdicts.push_back(Ok(verdict));
        self
    }

    /// Queue a failure, e.g. `SpmError::ClassifierUnavailable` to test a
    /// routine's fallback path.
    pub fn push_error(mut self, error: SpmError) -> Self {
        self.verdicts.push_back(Err(error));
        self
    }

    pub fn with_model(mut self, name: &str, version: &str) -> Self {
        self.model = ModelInfo {
            name: name.into(),
            version: version.into(),
        };
        self
    }
}

impl<I: ToNpyPayload, V: Serialize> Default for MockClassifier<I, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: ToNpyPayload, V: Serialize> Classifier for MockClassifier<I, V> {
    type Input = I;
    type Verdict = V;

    fn model(&self) -> &ModelInfo {
        &self.model
    }

    fn classify(&mut self, input: &I) -> Result<V, SpmError> {
        self.seen.push(input.metadata());
        self.verdicts.pop_front().unwrap_or_else(|| {
            Err(SpmError::Routine(
                "MockClassifier: no verdict scripted for this call".into(),
            ))
        })
    }
}
