//! HTTP transport for remote classifiers.
//!
//! # Wire contract
//!
//! - `GET {base}/info` → `200` with `{"name": "...", "version": "..."}`.
//!   Called once at [`HttpClassifier::connect`], so a dead sidecar fails
//!   at routine setup, not forty minutes into a run.
//! - `POST {base}/classify` → the payload's npy bytes as the body
//!   (`Content-Type: application/x-npy`), its JSON metadata in the
//!   `X-Payload-Meta` header, and the verdict as the JSON response body.
//!
//! The reference implementation of the server side ships in
//! `python/classifier_server.py`.
//!
//! # Failure semantics
//!
//! Not reaching the server (refused, timed out, DNS) is
//! [`SpmError::ClassifierUnavailable`], matchable so a routine can fall
//! back to threshold logic. A reachable server answering wrongly (error
//! status, malformed JSON) is [`SpmError::Protocol`]: that is a broken
//! deployment, and falling back would mask it.

use std::marker::PhantomData;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use ureq::Agent;

use super::{Classifier, ModelInfo};
use crate::frame::ToNpyPayload;
use crate::spm_error::SpmError;

/// A classifier reached over HTTP: generic over payload and verdict, so a
/// new Python-side model costs one verdict struct and an endpoint URL.
pub struct HttpClassifier<I, V> {
    base_url: String,
    agent: Agent,
    model: ModelInfo,
    _marker: PhantomData<fn(&I) -> V>,
}

impl<I: ToNpyPayload, V: Serialize + DeserializeOwned> HttpClassifier<I, V> {
    /// Handshake with the sidecar at `base_url` (e.g.
    /// `http://localhost:8000`) and resolve its model identity.
    pub fn connect(base_url: impl Into<String>) -> Result<Self, SpmError> {
        Self::connect_with_timeout(base_url, Duration::from_secs(30))
    }

    /// [`connect`](Self::connect) with an explicit per-request timeout,
    /// which also bounds every later `classify` call.
    pub fn connect_with_timeout(
        base_url: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, SpmError> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let agent = Agent::new_with_config(
            Agent::config_builder()
                .timeout_global(Some(timeout))
                .build(),
        );
        let mut response = agent
            .get(format!("{base_url}/info"))
            .call()
            .map_err(|e| transport_error("/info", e))?;
        let model: ModelInfo = response
            .body_mut()
            .read_json()
            .map_err(|e| SpmError::Protocol(format!("classifier /info: invalid JSON: {e}")))?;
        Ok(Self {
            base_url,
            agent,
            model,
            _marker: PhantomData,
        })
    }
}

impl<I: ToNpyPayload, V: Serialize + DeserializeOwned> Classifier for HttpClassifier<I, V> {
    type Input = I;
    type Verdict = V;

    fn model(&self) -> &ModelInfo {
        &self.model
    }

    fn classify(&mut self, input: &I) -> Result<V, SpmError> {
        let metadata = serde_json::to_string(&input.metadata())
            .map_err(|e| SpmError::Protocol(format!("payload metadata not serializable: {e}")))?;
        let mut response = self
            .agent
            .post(format!("{}/classify", self.base_url))
            .header("Content-Type", "application/x-npy")
            .header("X-Payload-Meta", &metadata)
            .send(&input.npy_bytes()[..])
            .map_err(|e| transport_error("/classify", e))?;
        response
            .body_mut()
            .read_json()
            .map_err(|e| SpmError::Protocol(format!("classifier /classify: invalid JSON: {e}")))
    }
}

/// An answered request with a bad status is a contract violation; not
/// getting an answer at all means the sidecar is unavailable.
fn transport_error(endpoint: &str, e: ureq::Error) -> SpmError {
    match e {
        ureq::Error::StatusCode(code) => {
            SpmError::Protocol(format!("classifier {endpoint}: HTTP {code}"))
        }
        other => SpmError::ClassifierUnavailable(format!("classifier {endpoint}: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Frame;
    use serde::Deserialize;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct RowVerdict {
        usable: bool,
        score: f64,
    }

    /// Serve `responses` on sequential connections, returning what each
    /// request contained (head + body). Minimal HTTP/1.1, `Connection:
    /// close` so the agent reconnects per request.
    fn serve(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for body in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = vec![0u8; 65536];
                let mut req = Vec::new();
                loop {
                    let n = stream.read(&mut buf).unwrap();
                    req.extend_from_slice(&buf[..n]);
                    let head_end = req.windows(4).position(|w| w == b"\r\n\r\n");
                    if let Some(pos) = head_end {
                        let head = String::from_utf8_lossy(&req[..pos]).to_string();
                        let expected = head
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if req.len() >= pos + 4 + expected {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                seen.push(String::from_utf8_lossy(&req).to_string());
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            seen
        });
        (addr, handle)
    }

    #[test]
    fn the_wire_contract_round_trips() {
        let (addr, handle) = serve(vec![
            r#"{"name": "row_finder", "version": "1.2.0"}"#.into(),
            r#"{"usable": true, "score": 0.93}"#.into(),
        ]);

        let mut classifier: HttpClassifier<Frame, RowVerdict> =
            HttpClassifier::connect_with_timeout(&addr, Duration::from_secs(5)).unwrap();
        assert_eq!(classifier.model().name, "row_finder");
        assert_eq!(classifier.model().version, "1.2.0");

        let frame = Frame::from_rows("Z", vec![vec![1.0, 2.0], vec![3.0, 4.0]]).unwrap();
        let verdict = classifier.classify(&frame).unwrap();
        assert_eq!(
            verdict,
            RowVerdict {
                usable: true,
                score: 0.93
            }
        );

        let requests = handle.join().unwrap();
        assert!(requests[0].starts_with("GET /info"));
        assert!(requests[1].starts_with("POST /classify"));
        // Metadata rides in the header, npy magic opens the body.
        assert!(requests[1].contains("X-Payload-Meta") || requests[1].contains("x-payload-meta"));
        assert!(requests[1].contains(r#""channel_name":"Z""#));
        // \x93 is not valid UTF-8, so the lossy view shows only "NUMPY".
        assert!(requests[1].contains("NUMPY"));
    }

    #[test]
    fn an_unreachable_sidecar_fails_at_connect_as_unavailable() {
        // Bind then drop, so the port is known-dead.
        let addr = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let err = match HttpClassifier::<Frame, RowVerdict>::connect_with_timeout(
            &addr,
            Duration::from_secs(1),
        ) {
            Ok(_) => panic!("connect to a dead port must fail"),
            Err(e) => e,
        };
        assert!(matches!(err, SpmError::ClassifierUnavailable(_)), "{err}");
    }

    #[test]
    fn a_malformed_verdict_is_a_protocol_error_not_unavailability() {
        let (addr, handle) = serve(vec![
            r#"{"name": "m", "version": "0"}"#.into(),
            "not json at all".into(),
        ]);
        let mut classifier: HttpClassifier<Frame, RowVerdict> =
            HttpClassifier::connect_with_timeout(&addr, Duration::from_secs(5)).unwrap();
        let frame = Frame::from_rows("Z", vec![vec![0.0]]).unwrap();
        let err = classifier.classify(&frame).unwrap_err();
        assert!(matches!(err, SpmError::Protocol(_)), "{err}");
        handle.join().unwrap();
    }
}
