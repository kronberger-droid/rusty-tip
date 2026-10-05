//! Power spectral density by Welch's method, for the `psd` read.
//!
//! The series is cut into segments of a power-of-two length that overlap
//! by half, each has its mean removed and a Hann window applied, and the
//! squared FFT magnitudes are averaged. The result is one-sided and
//! normalised as a density, in the signal's unit squared per hertz, so
//! integrating it over frequency gives back the variance.

use serde::Serialize;

/// A one-sided power spectral density.
#[derive(Debug, Clone, Serialize)]
pub struct Spectrum {
    /// Samples per segment.
    pub segment: usize,
    /// Segments averaged.
    pub averages: usize,
    /// Spacing of the frequency bins, in Hz.
    pub resolution_hz: f64,
    /// Bin frequencies, from 0 to the Nyquist frequency.
    pub freq_hz: Vec<f64>,
    /// Density at each bin, in the signal's unit squared per hertz.
    pub psd: Vec<f64>,
}

/// One peak of a spectrum.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Peak {
    pub freq_hz: f64,
    /// The amplitude density at the peak, the square root of the PSD, in
    /// the signal's unit per root hertz. A pure tone's height depends on
    /// the resolution; its frequency does not.
    pub asd: f64,
}

/// Welch's PSD of `values` sampled at `rate_hz`, in segments of `segment`
/// samples. `segment` has to be a power of two no longer than `values`.
pub fn welch(values: &[f64], rate_hz: f64, segment: usize) -> Spectrum {
    debug_assert!(segment.is_power_of_two() && segment <= values.len());
    // Periodic Hann, the usual choice for spectral estimates.
    let window: Vec<f64> = (0..segment)
        .map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / segment as f64).cos())
        .collect();
    let window_power: f64 = window.iter().map(|w| w * w).sum();

    let bins = segment / 2 + 1;
    let mut power = vec![0.0; bins];
    let mut averages = 0;
    let mut start = 0;
    while start + segment <= values.len() {
        let piece = &values[start..start + segment];
        let mean = piece.iter().sum::<f64>() / segment as f64;
        let mut buffer: Vec<Complex> = piece
            .iter()
            .zip(&window)
            .map(|(v, w)| Complex::real((v - mean) * w))
            .collect();
        fft(&mut buffer);
        for (p, x) in power.iter_mut().zip(&buffer) {
            *p += x.norm_sqr();
        }
        averages += 1;
        start += segment / 2;
    }

    let scale = 1.0 / (averages as f64 * rate_hz * window_power);
    let psd = power
        .iter()
        .enumerate()
        .map(|(k, p)| {
            // Every bin but DC and Nyquist stands for its negative twin too.
            let one_sided = if k == 0 || k == segment / 2 { 1.0 } else { 2.0 };
            p * scale * one_sided
        })
        .collect();
    let resolution_hz = rate_hz / segment as f64;
    Spectrum {
        segment,
        averages,
        resolution_hz,
        freq_hz: (0..bins).map(|k| k as f64 * resolution_hz).collect(),
        psd,
    }
}

/// The segment length to use when none is asked for: 1024, or shorter so
/// that at least seven segments are averaged.
pub fn default_segment(samples: usize) -> usize {
    let mut segment = 1024;
    while segment > 16 && 2 * samples / segment < 8 {
        segment /= 2;
    }
    segment
}

/// The noise floor as an amplitude density: the square root of the median
/// PSD above DC. A peak not well above it is a bump in the noise.
pub fn floor_asd(spectrum: &Spectrum) -> f64 {
    let mut above_dc: Vec<f64> = spectrum.psd.iter().skip(1).copied().collect();
    if above_dc.is_empty() {
        return 0.0;
    }
    above_dc.sort_by(f64::total_cmp);
    above_dc[above_dc.len() / 2].sqrt()
}

/// The `count` highest local maxima above DC, highest first.
pub fn peaks(spectrum: &Spectrum, count: usize) -> Vec<Peak> {
    let psd = &spectrum.psd;
    let mut found: Vec<usize> = (1..psd.len())
        .filter(|&k| psd[k] > psd[k - 1] && psd.get(k + 1).is_none_or(|&next| psd[k] >= next))
        .collect();
    found.sort_by(|&a, &b| psd[b].total_cmp(&psd[a]));
    found
        .into_iter()
        .take(count)
        .map(|k| Peak {
            freq_hz: spectrum.freq_hz[k],
            asd: psd[k].sqrt(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    fn real(re: f64) -> Self {
        Self { re, im: 0.0 }
    }

    fn norm_sqr(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
}

/// In-place radix-2 FFT; the length has to be a power of two.
fn fft(x: &mut [Complex]) {
    let n = x.len();
    // Bit-reversal permutation.
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            x.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (angle * k as f64).sin_cos();
                let a = x[start + k];
                let b = x[start + k + len / 2];
                let t = Complex {
                    re: b.re * c - b.im * s,
                    im: b.re * s + b.im * c,
                };
                x[start + k] = Complex {
                    re: a.re + t.re,
                    im: a.im + t.im,
                };
                x[start + k + len / 2] = Complex {
                    re: a.re - t.re,
                    im: a.im - t.im,
                };
            }
        }
        len *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(amplitude: f64, freq_hz: f64, rate_hz: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| amplitude * (2.0 * std::f64::consts::PI * freq_hz * i as f64 / rate_hz).sin())
            .collect()
    }

    #[test]
    fn the_fft_matches_a_direct_dft() {
        let input: Vec<f64> = (0..16).map(|i| ((i * 7) % 5) as f64 - 1.3).collect();
        let mut fast: Vec<Complex> = input.iter().map(|&v| Complex::real(v)).collect();
        fft(&mut fast);
        for (k, got) in fast.iter().enumerate() {
            let (mut re, mut im) = (0.0, 0.0);
            for (n, v) in input.iter().enumerate() {
                let a = -2.0 * std::f64::consts::PI * (k * n) as f64 / 16.0;
                re += v * a.cos();
                im += v * a.sin();
            }
            assert!(
                (got.re - re).abs() < 1e-9 && (got.im - im).abs() < 1e-9,
                "bin {k}"
            );
        }
    }

    /// The density integrates to the variance: a sine of amplitude A
    /// carries A²/2.
    #[test]
    fn the_density_integrates_to_the_variance() {
        let spectrum = welch(&sine(2.0, 69.0, 1000.0, 16384), 1000.0, 1024);
        let total: f64 = spectrum.psd.iter().sum::<f64>() * spectrum.resolution_hz;
        assert!((total - 2.0).abs() < 0.02, "{total}");
    }

    /// The 69 Hz line the lab sees shows as the top peak, near 69 Hz.
    #[test]
    fn a_tone_is_the_top_peak_at_its_frequency() {
        let rate = 1000.0;
        let mut values = sine(1.0, 69.0, rate, 8192);
        for (v, small) in values.iter_mut().zip(sine(0.1, 210.0, rate, 8192)) {
            *v += small;
        }
        let spectrum = welch(&values, rate, 1024);
        let top = peaks(&spectrum, 2);
        assert!(
            (top[0].freq_hz - 69.0).abs() <= spectrum.resolution_hz,
            "{top:?}"
        );
        assert!(
            (top[1].freq_hz - 210.0).abs() <= spectrum.resolution_hz,
            "{top:?}"
        );
        assert!(top[0].asd > top[1].asd);
        assert!(
            top[1].asd > 10.0 * floor_asd(&spectrum),
            "tones stand far above the floor"
        );
    }

    #[test]
    fn segments_overlap_by_half_and_shrink_for_short_series() {
        let spectrum = welch(&vec![0.0; 4096], 1000.0, 1024);
        assert_eq!(spectrum.averages, 7);
        assert_eq!(spectrum.freq_hz.len(), 513);
        assert_eq!(spectrum.freq_hz[512], 500.0);
        assert_eq!(default_segment(16384), 1024);
        assert_eq!(default_segment(2000), 256);
    }
}
