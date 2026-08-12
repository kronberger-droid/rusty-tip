//! Typed scan frames.
//!
//! A [`Frame`] is one grabbed scan image: flat row-major `f32` pixels plus
//! the metadata every consumer keeps needing — channel, dimensions, scan
//! direction, and (when the frame came off an instrument) its physical
//! geometry. It replaces the untyped JSON blob that used to travel through
//! the `DataStore`.
//!
//! Frames are also the canonical classifier input: [`ToNpyPayload`] encodes
//! the pixels as an npy array (numpy-native, lossless) with the metadata
//! riding alongside as JSON.

use serde::Serialize;

use nanonis_rs::Position;
use nanonis_rs::scan::ScanFrame;

use crate::spm_error::SpmError;

/// Physical placement of a frame in the scan field, as reported by the
/// instrument (`scan_frame_get`).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FrameGeometry {
    pub center: Position,
    pub width_m: f32,
    pub height_m: f32,
    pub angle_deg: f32,
}

impl From<ScanFrame> for FrameGeometry {
    fn from(f: ScanFrame) -> Self {
        Self {
            center: f.center,
            width_m: f.width_m,
            height_m: f.height_m,
            angle_deg: f.angle_deg,
        }
    }
}

/// One grabbed scan image.
///
/// Serialization carries the metadata only: `data` is skipped, since the
/// event log wants geometry and shape, while the pixel payload travels as
/// npy via [`ToNpyPayload`].
#[derive(Debug, Clone, Serialize)]
pub struct Frame {
    /// Channel name from the scan buffer (e.g. "Z", "Current").
    pub channel_name: String,
    /// Row-major pixels, `rows * cols` long.
    #[serde(skip)]
    pub data: Vec<f32>,
    pub rows: usize,
    pub cols: usize,
    /// `true` if grabbed from an upward scan.
    pub direction_up: bool,
    /// Physical geometry, when the frame came off an instrument.
    /// Frames built from files carry `None`.
    pub geometry: Option<FrameGeometry>,
}

impl Frame {
    /// Build a frame from nested rows, flattening them.
    ///
    /// Fails if the rows are ragged (unequal lengths).
    pub fn from_rows(
        channel_name: impl Into<String>,
        rows: Vec<Vec<f32>>,
    ) -> Result<Self, SpmError> {
        let n_rows = rows.len();
        let n_cols = rows.first().map_or(0, |r| r.len());
        if let Some(bad) = rows.iter().position(|r| r.len() != n_cols) {
            return Err(SpmError::Routine(format!(
                "Frame::from_rows: row {} has {} columns, expected {}",
                bad,
                rows[bad].len(),
                n_cols
            )));
        }
        Ok(Self {
            channel_name: channel_name.into(),
            data: rows.into_iter().flatten().collect(),
            rows: n_rows,
            cols: n_cols,
            direction_up: true,
            geometry: None,
        })
    }

    pub fn with_direction(mut self, up: bool) -> Self {
        self.direction_up = up;
        self
    }

    pub fn with_geometry(mut self, geometry: FrameGeometry) -> Self {
        self.geometry = Some(geometry);
        self
    }

    /// Attach synthetic geometry from a uniform calibration, for frames
    /// loaded from files where only the pixel size is known: centered at
    /// the origin, unrotated, `width = cols * m_per_px`.
    pub fn with_uniform_calibration(self, m_per_px: f64) -> Self {
        let geometry = FrameGeometry {
            center: Position::new(0.0, 0.0),
            width_m: (self.cols as f64 * m_per_px) as f32,
            height_m: (self.rows as f64 * m_per_px) as f32,
            angle_deg: 0.0,
        };
        self.with_geometry(geometry)
    }

    /// One row of pixels.
    pub fn row(&self, i: usize) -> &[f32] {
        &self.data[i * self.cols..(i + 1) * self.cols]
    }

    /// Pixel at (row, col).
    pub fn at(&self, row: usize, col: usize) -> f32 {
        self.data[row * self.cols + col]
    }

    /// Physical pixel size in metres, `(x, y)`, if geometry is known.
    pub fn m_per_px(&self) -> Option<(f64, f64)> {
        let g = self.geometry.as_ref()?;
        if self.cols == 0 || self.rows == 0 {
            return None;
        }
        Some((
            g.width_m as f64 / self.cols as f64,
            g.height_m as f64 / self.rows as f64,
        ))
    }
}

/// Data that can be shipped to a classifier: an npy-encoded array plus
/// JSON metadata that rides alongside it.
///
/// [`Frame`] implements this; future payload kinds (line scans, signal
/// windows) implement it too, and every transport generic over
/// `ToNpyPayload` carries them unchanged.
pub trait ToNpyPayload {
    /// The array as npy v1.0 bytes (little-endian f32, C order).
    fn npy_bytes(&self) -> Vec<u8>;
    /// Metadata describing the array, sent as JSON alongside the payload.
    fn metadata(&self) -> serde_json::Value;
}

impl ToNpyPayload for Frame {
    fn npy_bytes(&self) -> Vec<u8> {
        npy_f32(&self.data, &[self.rows, self.cols])
    }

    fn metadata(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// Encode a flat f32 slice as an npy v1.0 array of the given shape
/// (little-endian, C order).
pub fn npy_f32(data: &[f32], shape: &[usize]) -> Vec<u8> {
    let shape_str = match shape {
        [n] => format!("({},)", n),
        _ => format!(
            "({})",
            shape
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let mut header = format!(
        "{{'descr': '<f4', 'fortran_order': False, 'shape': {}, }}",
        shape_str
    );
    // The npy spec wants magic + version + length field + header padded to
    // a multiple of 64 bytes, header terminated by a newline.
    let unpadded = 10 + header.len() + 1;
    header.push_str(&" ".repeat(unpadded.div_ceil(64) * 64 - unpadded));
    header.push('\n');

    let mut out = Vec::with_capacity(10 + header.len() + data.len() * 4);
    out.extend_from_slice(b"\x93NUMPY\x01\x00");
    out.extend_from_slice(&(header.len() as u16).to_le_bytes());
    out.extend_from_slice(header.as_bytes());
    for v in data {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_2x3() -> Frame {
        Frame::from_rows("Z", vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]).unwrap()
    }

    #[test]
    fn from_rows_flattens_and_indexes() {
        let f = frame_2x3();
        assert_eq!((f.rows, f.cols), (2, 3));
        assert_eq!(f.row(1), &[4.0, 5.0, 6.0]);
        assert_eq!(f.at(0, 2), 3.0);
    }

    #[test]
    fn ragged_rows_are_rejected() {
        let err = Frame::from_rows("Z", vec![vec![1.0, 2.0], vec![3.0]]).unwrap_err();
        assert!(err.to_string().contains("row 1"));
    }

    #[test]
    fn npy_layout_matches_the_spec() {
        let f = frame_2x3();
        let bytes = f.npy_bytes();
        assert_eq!(&bytes[..8], b"\x93NUMPY\x01\x00");
        let hlen = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
        assert_eq!((10 + hlen) % 64, 0);
        let header = std::str::from_utf8(&bytes[10..10 + hlen]).unwrap();
        assert!(header.contains("'descr': '<f4'"));
        assert!(header.contains("'shape': (2, 3)"));
        assert!(header.ends_with('\n'));
        // Payload: 6 little-endian f32s follow the header.
        let payload = &bytes[10 + hlen..];
        assert_eq!(payload.len(), 6 * 4);
        assert_eq!(f32::from_le_bytes(payload[..4].try_into().unwrap()), 1.0);
    }

    #[test]
    fn metadata_carries_shape_but_not_pixels() {
        let f = frame_2x3().with_uniform_calibration(1e-9);
        let meta = f.metadata();
        assert_eq!(meta["rows"], 2);
        assert_eq!(meta["channel_name"], "Z");
        assert!(meta.get("data").is_none());
        assert!((meta["geometry"]["width_m"].as_f64().unwrap() - 3e-9).abs() < 1e-15);
    }

    #[test]
    fn m_per_px_comes_from_geometry() {
        assert_eq!(frame_2x3().m_per_px(), None);
        let (x, y) = frame_2x3()
            .with_uniform_calibration(2e-9)
            .m_per_px()
            .unwrap();
        assert!((x - 2e-9).abs() < 1e-15 && (y - 2e-9).abs() < 1e-15);
    }
}
