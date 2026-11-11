//! Instrument behaviour the mock reproduces on purpose, for tests that judge
//! an outcome against the truth rather than the mock's own report of it.
//!
//! Drift compensation and the stable read both work in the regime the plain
//! mock offers: constant drift, white noise, a compensation response of
//! exactly one, no PLL rail. The instrument is not in that regime. These
//! models put back what a review of both features found missing:
//!
//! - **Z drift** ([`DriftScenario`]): creep that dies out exponentially or as
//!   `1/t` after a Z move, on a simulated clock that advances with every
//!   streamed sample, coloured noise, a compensation response that is not
//!   exactly one and lags, and a compensation that saturates and latches.
//! - **Frequency shift** ([`FreqShiftNoise`] plus a rail): the PLL's blue
//!   noise, recorded noise replayed, and a rail that pins the reading at a
//!   fixed value with no scatter at all, which is what a PLL out of range or
//!   out of lock streams.
//!
//! Every model is deterministic for a seed, so an outcome test is a fixed
//! number, not a flaky one.

use std::sync::Arc;

use super::Rng;

/// How Z drifts with no compensation applied, as a function of the time
/// since the last Z move, and how the instrument answers compensation.
///
/// The drift is `drift_m_s + creep_m_s * exp(-t / creep_tau_s) +
/// log_creep_m / (t + log_creep_t0_s)`, the constant part a piezo settles
/// to plus the two usual shapes of creep. A run starts `start_s` after the
/// move, and the clock advances by one sample period per streamed sample
/// plus `set_overhead_s` per compensation write, so a scenario covers
/// minutes of creep in milliseconds of test time.
#[derive(Debug, Clone, PartialEq)]
pub struct DriftScenario {
    /// The drift once creep has died out, m/s.
    pub drift_m_s: f64,
    /// Exponential creep at the moment of the Z move, m/s.
    pub creep_m_s: f64,
    /// Time constant of the exponential creep, s.
    pub creep_tau_s: f64,
    /// Logarithmic creep: `log_creep_m / (t + log_creep_t0_s)` m/s.
    pub log_creep_m: f64,
    /// Offset that keeps the `1/t` creep finite at the move, s.
    pub log_creep_t0_s: f64,
    /// Time from the Z move to the first streamed sample, s.
    pub start_s: f64,
    /// Drift added by one m/s of compensation velocity: its sign is the
    /// convention under test, its size is the instrument's gain error.
    pub response: f64,
    /// Time a new compensation velocity takes to act, s.
    pub latency_s: f64,
    /// Scatter on each streamed sample.
    pub noise: ZNoise,
    /// Stream rate, Hz.
    pub rate_hz: f64,
    /// Time a compensation write costs on the simulated clock, s.
    pub set_overhead_s: f64,
    /// Compensation displacement at which the axis saturates, m. Once it
    /// does, compensation on Z stops and `z_saturated` latches until it is
    /// switched off.
    pub saturation_m: Option<f64>,
}

impl DriftScenario {
    /// A constant drift at 1 kHz with no noise, latency, overhead or
    /// saturation: the plain mock's behaviour.
    pub fn constant(drift_m_s: f64, response: f64) -> Self {
        Self {
            drift_m_s,
            creep_m_s: 0.0,
            creep_tau_s: 1.0,
            log_creep_m: 0.0,
            log_creep_t0_s: 1.0,
            start_s: 0.0,
            response,
            latency_s: 0.0,
            noise: ZNoise::None,
            rate_hz: 1000.0,
            set_overhead_s: 0.0,
            saturation_m: None,
        }
    }

    /// Add exponential creep of `creep_m_s` at the move, dying out with
    /// time constant `tau_s`.
    pub fn creep(mut self, creep_m_s: f64, tau_s: f64) -> Self {
        self.creep_m_s = creep_m_s;
        self.creep_tau_s = tau_s;
        self
    }

    /// Add `1/t` creep: `amplitude_m / (t + t0_s)` m/s.
    pub fn log_creep(mut self, amplitude_m: f64, t0_s: f64) -> Self {
        self.log_creep_m = amplitude_m;
        self.log_creep_t0_s = t0_s;
        self
    }

    /// Start the run `start_s` after the Z move.
    pub fn starting_after(mut self, start_s: f64) -> Self {
        self.start_s = start_s;
        self
    }

    /// Let a new compensation velocity act only `latency_s` after it is set.
    ///
    /// With a latency the velocity in effect comes from the writes the mock
    /// has seen, starting from zero, so a `drift_comp` set directly on the
    /// observations does not act until it is written through
    /// `drift_comp_set`. Without one the mock reads the controller's state
    /// each sample, as the plain mock always has.
    pub fn latency(mut self, latency_s: f64) -> Self {
        self.latency_s = latency_s;
        self
    }

    pub fn noise(mut self, noise: ZNoise) -> Self {
        self.noise = noise;
        self
    }

    pub fn rate(mut self, rate_hz: f64) -> Self {
        self.rate_hz = rate_hz;
        self
    }

    /// Charge `overhead_s` of simulated time per compensation write, for the
    /// gaps between bursts a real run spends talking to the controller.
    pub fn set_overhead(mut self, overhead_s: f64) -> Self {
        self.set_overhead_s = overhead_s;
        self
    }

    /// Saturate Z once compensation has moved it by `displacement_m`.
    pub fn saturates_at(mut self, displacement_m: f64) -> Self {
        self.saturation_m = Some(displacement_m);
        self
    }

    /// The uncompensated drift `t_s` after the Z move, m/s.
    pub fn drift_at(&self, t_s: f64) -> f64 {
        self.drift_m_s
            + self.creep_m_s * (-t_s / self.creep_tau_s).exp()
            + self.log_creep_m / (t_s + self.log_creep_t0_s)
    }

    /// The drift left with `vz_m_s` of compensation acting, `t_s` after the
    /// Z move: what an outcome test calls the truth.
    pub fn residual_at(&self, t_s: f64, vz_m_s: f64) -> f64 {
        self.drift_at(t_s) + self.response * vz_m_s
    }
}

/// Scatter on a streamed Z sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZNoise {
    None,
    /// Independent samples, std dev in m.
    White {
        sigma_m: f64,
    },
    /// First-order correlated noise with stationary std dev `sigma_m` and
    /// correlation time `tau_s`: the slow wander of Z under feedback that
    /// makes neighbouring blocks of a burst agree more than they should.
    Ar1 {
        sigma_m: f64,
        tau_s: f64,
    },
    /// A random walk, `sigma_m_per_sqrt_s` m after one second.
    RandomWalk {
        sigma_m_per_sqrt_s: f64,
    },
}

/// Where the Z simulation is, beyond its scenario.
#[derive(Debug, Clone)]
pub(super) struct ZState {
    /// Simulated time since the Z move, s.
    pub t_s: f64,
    pub z: f64,
    ar: f64,
    walk: f64,
    /// Compensation velocities with the time each starts to act, oldest
    /// first. The last one whose time has come is in effect.
    pending: Vec<(f64, f64)>,
    /// Displacement compensation has accumulated, m.
    displacement_m: f64,
}

impl ZState {
    pub fn new(scenario: &DriftScenario) -> Self {
        Self {
            t_s: scenario.start_s,
            z: 1e-7,
            ar: 0.0,
            walk: 0.0,
            pending: vec![(f64::NEG_INFINITY, 0.0)],
            displacement_m: 0.0,
        }
    }

    /// A compensation write: costs the overhead, and the velocity it leaves
    /// in effect (zero while switched off) acts after the latency. Switching
    /// off re-arms a saturated axis, which also starts its displacement
    /// over.
    pub fn on_set(&mut self, scenario: &DriftScenario, vz_in_effect: f64, enabled: bool) {
        self.t_s += scenario.set_overhead_s;
        self.pending
            .push((self.t_s + scenario.latency_s, vz_in_effect));
        if !enabled {
            self.displacement_m = 0.0;
        }
    }

    fn vz_acting(&mut self) -> f64 {
        let now = self.t_s;
        let current = self.pending.iter().rposition(|(from, _)| *from <= now);
        // Older entries can no longer matter; keep the one in effect.
        if let Some(i) = current
            && i > 0
        {
            self.pending.drain(..i);
        }
        self.pending.first().map_or(0.0, |(_, vz)| *vz)
    }

    /// One streamed sample. `vz_now` overrides the latency schedule with the
    /// velocity the controller holds this instant, which is what a scenario
    /// without latency means. Returns the value and whether the axis
    /// saturated on this step.
    pub fn sample(
        &mut self,
        scenario: &DriftScenario,
        saturated: bool,
        vz_now: Option<f64>,
        rng: &mut Rng,
    ) -> (f64, bool) {
        let dt = 1.0 / scenario.rate_hz;
        let vz = match (saturated, vz_now) {
            (true, _) => 0.0,
            (false, Some(vz)) => vz,
            (false, None) => self.vz_acting(),
        };
        let rate = scenario.residual_at(self.t_s, vz);
        self.z += rate * dt;
        self.t_s += dt;
        self.displacement_m += vz * dt;
        let now_saturated = !saturated
            && scenario
                .saturation_m
                .is_some_and(|limit| self.displacement_m.abs() >= limit);

        // One normal per sample whatever the noise, so a scenario's samples
        // do not depend on which noise it picked.
        let n = rng.normal();
        let noise = match scenario.noise {
            ZNoise::None => 0.0,
            ZNoise::White { sigma_m } => n * sigma_m,
            ZNoise::Ar1 { sigma_m, tau_s } => {
                let a = (-dt / tau_s).exp();
                self.ar = a * self.ar + (1.0 - a * a).sqrt() * sigma_m * n;
                self.ar
            }
            ZNoise::RandomWalk { sigma_m_per_sqrt_s } => {
                self.walk += sigma_m_per_sqrt_s * dt.sqrt() * n;
                self.walk
            }
        };
        (self.z + noise, now_saturated)
    }
}

/// Scatter on the frequency shift within a batch.
#[derive(Debug, Clone, PartialEq)]
pub enum FreqShiftNoise {
    /// Independent samples, std dev in Hz.
    White { sigma_hz: f64 },
    /// Blue noise, as the PLL's demodulator delivers it: first-order moving
    /// average `w[k] - c * w[k-1]`, scaled to std dev `sigma_hz`. Neighbours
    /// anticorrelate (lag-1 correlation `-c / (1 + c²)`), so a batch mean is
    /// better than its std dev suggests and a fitted slope scatters less
    /// than white noise of the same size would make it.
    Blue { sigma_hz: f64, c: f64 },
    /// Recorded deviations from a steady reading, replayed in order and
    /// wrapping around.
    Replay(Arc<[f64]>),
}

/// Where the frequency-shift noise is.
#[derive(Debug, Clone, Default)]
pub(super) struct FreqShiftState {
    last_w: f64,
    position: usize,
}

impl FreqShiftState {
    /// The next sample's deviation from the tip model's value, Hz.
    pub fn next(&mut self, noise: &FreqShiftNoise, rng: &mut Rng) -> f64 {
        match noise {
            FreqShiftNoise::White { sigma_hz } => rng.normal() * sigma_hz,
            FreqShiftNoise::Blue { sigma_hz, c } => {
                let w = rng.normal();
                let e = (w - c * self.last_w) / (1.0 + c * c).sqrt();
                self.last_w = w;
                e * sigma_hz
            }
            FreqShiftNoise::Replay(samples) => {
                if samples.is_empty() {
                    return 0.0;
                }
                let v = samples[self.position % samples.len()];
                self.position += 1;
                v
            }
        }
    }
}
