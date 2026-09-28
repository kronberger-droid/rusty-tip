//! The last seconds of the data stream, per signal, for live charts.
//!
//! The session sends the stream in pieces ([`StreamSamples`]) while it is
//! idle; this keeps a ring per signal index with times on the session's
//! clock, and hands a chart the points relative to the newest piece, so
//! the newest sample sits at zero and the past runs negative.

use std::collections::{BTreeMap, VecDeque};

use rusty_tip::session::StreamSamples;

/// How much of the stream to keep, in seconds.
const KEEP_S: f64 = 12.0;

#[derive(Debug, Default)]
pub struct Samples {
    rings: BTreeMap<u32, VecDeque<[f64; 2]>>,
    /// When the newest piece was taken, on the session's clock.
    now_s: Option<f64>,
}

impl Samples {
    /// Append a piece and drop what is older than the window.
    pub fn take(&mut self, piece: &StreamSamples) {
        let snap = &piece.snapshot;
        for (column, &index) in snap.columns.iter().zip(&snap.signals) {
            let ring = self.rings.entry(index).or_default();
            for (t, v) in snap.t_s.iter().zip(column) {
                ring.push_back([piece.at_s + t, f64::from(*v)]);
            }
            let cutoff = piece.at_s - KEEP_S;
            while ring.front().is_some_and(|p| p[0] < cutoff) {
                ring.pop_front();
            }
        }
        self.now_s = Some(piece.at_s);
    }

    pub fn clear(&mut self) {
        self.rings.clear();
        self.now_s = None;
    }

    /// Whether anything has been streamed for `index`.
    pub fn has(&self, index: u32) -> bool {
        self.rings.get(&index).is_some_and(|r| !r.is_empty())
    }

    /// The signal's points, time relative to the newest piece (zero and
    /// negative), value scaled by `scale`.
    pub fn points(&self, index: u32, scale: f64) -> impl Iterator<Item = [f64; 2]> + '_ {
        let now = self.now_s.unwrap_or(0.0);
        self.rings
            .get(&index)
            .into_iter()
            .flat_map(move |r| r.iter().map(move |p| [p[0] - now, p[1] * scale]))
    }

    /// The newest value of a signal.
    pub fn latest(&self, index: u32) -> Option<f64> {
        self.rings.get(&index)?.back().map(|p| p[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tip::spm_controller::StreamSnapshot;

    fn piece(at_s: f64, values: &[f32]) -> StreamSamples {
        let n = values.len();
        StreamSamples {
            at_s,
            snapshot: StreamSnapshot {
                signals: vec![3],
                t_s: (0..n).map(|i| -((n - 1 - i) as f64) * 0.001).collect(),
                columns: vec![values.to_vec()],
            },
        }
    }

    #[test]
    fn pieces_append_and_the_window_slides() {
        let mut s = Samples::default();
        s.take(&piece(1.0, &[1.0, 2.0, 3.0]));
        s.take(&piece(1.5, &[4.0]));
        assert!(s.has(3));
        assert!(!s.has(0));
        let pts: Vec<_> = s.points(3, 1.0).collect();
        assert_eq!(pts.len(), 4);
        assert_eq!(pts.last().unwrap(), &[0.0, 4.0], "newest at zero");
        assert!((pts[0][0] - (1.0 - 0.002 - 1.5)).abs() < 1e-9);
        assert_eq!(s.latest(3), Some(4.0));

        s.take(&piece(1.5 + KEEP_S + 0.1, &[5.0]));
        assert_eq!(s.points(3, 1.0).count(), 1, "older than the window is gone");
        assert_eq!(s.points(3, 2.0).next().unwrap()[1], 10.0, "scaled");
        s.clear();
        assert!(!s.has(3));
    }
}
