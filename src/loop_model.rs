//! A Z feedback loop as arithmetic: a PI controller on a log or linear
//! input, a tunnelling current or a frequency shift that depends on the
//! gap, and an actuator that lags its command.
//!
//! This is what the mock runs behind its Z-controller, so the workbench
//! and the step tests can be exercised without a microscope, and it is the
//! seed of the preview that shows what a setting will do before it meets
//! the tip. It is deliberately small: one first-order lag stands in for the
//! amplifier and the piezo, the current is a single exponential in the gap
//! and the frequency shift another. The numbers that make it quantitative
//! for a given microscope, the lag above all, are what a measured step
//! response will fit later.
//!
//! Conventions: `z` grows away from the surface, which sits at
//! [`Plant::surface_m`]; the loop retracts on a positive error, and
//! [`ZLoop::slope`] flips the error for loops whose input falls toward the
//! surface, such as `Frequency (neg)`.

use crate::controllers::{ZLaw, ZLoopInput, ZQuantity};

/// The sample and the surface: what the input signals depend on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plant {
    /// Where the surface is; the gap is `z - surface_m`.
    pub surface_m: f64,
    /// Current at zero gap.
    pub current_at_contact_a: f64,
    /// Decay of the current per metre of gap, twice the inverse decay
    /// length: about `2e10` for a metal.
    pub current_decay_per_m: f64,
    /// Magnitude of the frequency shift at zero gap; the shift is negative
    /// and falls off with the gap.
    pub freq_shift_at_contact_hz: f64,
    /// Decay length of the frequency shift.
    pub freq_shift_decay_m: f64,
}

impl Default for Plant {
    fn default() -> Self {
        Self {
            surface_m: 0.0,
            current_at_contact_a: 1e-6,
            current_decay_per_m: 2e10,
            freq_shift_at_contact_hz: 100.0,
            freq_shift_decay_m: 0.3e-9,
        }
    }
}

impl Plant {
    /// The tunnelling current at `z_m`.
    pub fn current_at(&self, z_m: f64) -> f64 {
        self.current_at_contact_a * (-self.current_decay_per_m * (z_m - self.surface_m)).exp()
    }

    /// The frequency shift at `z_m`.
    pub fn freq_shift_at(&self, z_m: f64) -> f64 {
        -self.freq_shift_at_contact_hz * (-(z_m - self.surface_m) / self.freq_shift_decay_m).exp()
    }
}

/// What the loop reads and writes at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub z_m: f64,
    pub current_a: f64,
    pub freq_shift_hz: f64,
}

/// The loop: its parameters, the plant, and its state.
#[derive(Debug, Clone, PartialEq)]
pub struct ZLoop {
    pub law: ZLaw,
    /// Which signal the loop reads. Anything but a frequency is treated
    /// as the current.
    pub quantity: ZQuantity,
    /// `1.0` retracts on a positive error, `-1.0` on a negative one.
    pub slope: f64,
    pub setpoint: f64,
    /// Metres of Z per unit of error.
    pub p_gain_m: f64,
    /// The integral gain is `p_gain_m / time_constant_s`.
    pub time_constant_s: f64,
    pub enabled: bool,
    /// The actuator's first-order lag. Zero follows the command at once.
    pub lag_s: f64,
    pub plant: Plant,
    /// Where the tip is.
    pub z_m: f64,
    /// Where the controller wants it.
    z_command_m: f64,
    /// Z when the loop was last engaged, the base the PI terms add to.
    z_base_m: f64,
    integral: f64,
}

/// How finely the loop is integrated: a 50 kHz step, faster than any
/// time constant it will be given.
const SUBSTEP_S: f64 = 20e-6;

impl ZLoop {
    /// A loop as a defined Nanonis controller of that name would behave,
    /// engaged at `z_m` with the default plant.
    pub fn for_name(name: &str, z_m: f64) -> Self {
        let input = ZLoopInput::from_name(name);
        Self {
            law: input.law,
            quantity: input.quantity,
            slope: if input.negative { -1.0 } else { 1.0 },
            setpoint: match input.quantity {
                ZQuantity::Frequency => -2.0,
                _ => 50e-12,
            },
            p_gain_m: 200e-12,
            time_constant_s: 1e-3,
            enabled: true,
            lag_s: 100e-6,
            plant: Plant::default(),
            z_m,
            z_command_m: z_m,
            z_base_m: z_m,
            integral: 0.0,
        }
    }

    /// The signals at the current position.
    pub fn sample(&self) -> Sample {
        Sample {
            z_m: self.z_m,
            current_a: self.plant.current_at(self.z_m),
            freq_shift_hz: self.plant.freq_shift_at(self.z_m),
        }
    }

    /// The loop's own input at a position.
    pub fn input_at(&self, z_m: f64) -> f64 {
        match self.quantity {
            ZQuantity::Frequency => self.plant.freq_shift_at(z_m),
            _ => self.plant.current_at(z_m),
        }
    }

    /// The error the PI terms act on, signed so that positive retracts.
    /// A log loop takes the log of the ratio to the setpoint, which needs
    /// both of the same sign; otherwise it falls back to the difference.
    pub fn error(&self, input: f64) -> f64 {
        let raw = match self.law {
            ZLaw::Log if input * self.setpoint > 0.0 => (input / self.setpoint).ln(),
            _ => input - self.setpoint,
        };
        raw * self.slope
    }

    /// Engage the loop where the tip is: the PI terms start from here.
    pub fn engage(&mut self) {
        self.enabled = true;
        self.z_base_m = self.z_m;
        self.z_command_m = self.z_m;
        self.integral = 0.0;
    }

    /// Hold the tip where it is.
    pub fn disengage(&mut self) {
        self.enabled = false;
        self.z_command_m = self.z_m;
    }

    /// Put the tip somewhere with the loop off, as a withdraw or an
    /// approach does.
    pub fn place(&mut self, z_m: f64) {
        self.z_m = z_m;
        self.z_command_m = z_m;
        self.z_base_m = z_m;
        self.integral = 0.0;
    }

    /// Advance by `dt_s` and return the signals afterwards.
    pub fn step(&mut self, dt_s: f64) -> Sample {
        let n = (dt_s / SUBSTEP_S).ceil().max(1.0) as usize;
        let h = dt_s / n as f64;
        for _ in 0..n {
            if self.enabled {
                let e = self.error(self.input_at(self.z_m));
                self.integral += e * h;
                let i_gain = if self.time_constant_s > 0.0 {
                    self.p_gain_m / self.time_constant_s
                } else {
                    0.0
                };
                self.z_command_m = self.z_base_m + self.p_gain_m * e + i_gain * self.integral;
            }
            if self.lag_s > 0.0 {
                self.z_m += (self.z_command_m - self.z_m) * (h / self.lag_s).min(1.0);
            } else {
                self.z_m = self.z_command_m;
            }
        }
        self.sample()
    }

    /// Run until the input sits within `tolerance` of the setpoint (as a
    /// fraction of it) for `hold_s`, or `max_s` passes. Returns the time
    /// it took, or `None` on the budget.
    pub fn settle(&mut self, dt_s: f64, tolerance: f64, hold_s: f64, max_s: f64) -> Option<f64> {
        let mut t = 0.0;
        let mut within = 0.0;
        while t < max_s {
            self.step(dt_s);
            t += dt_s;
            let input = self.input_at(self.z_m);
            let off = ((input - self.setpoint) / self.setpoint).abs();
            within = if off <= tolerance { within + dt_s } else { 0.0 };
            if within >= hold_s {
                return Some(t - hold_s);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_current_loop_settles_on_its_setpoint() {
        let mut lp = ZLoop::for_name("log Current", 1e-9);
        assert_eq!(lp.law, ZLaw::Log);
        let before = lp.sample().current_a;
        assert!(
            before < lp.setpoint,
            "starts far out, current below setpoint"
        );
        let settled = lp.settle(1e-3, 0.01, 0.005, 1.0);
        assert!(settled.is_some(), "settles within a second");
        let after = lp.sample();
        assert!(((after.current_a - 50e-12) / 50e-12).abs() < 0.01);
        assert!(
            after.z_m < 1e-9 && after.z_m > 0.0,
            "moved in, stayed above the surface"
        );
    }

    #[test]
    fn a_setpoint_step_moves_the_tip_the_right_way() {
        let mut lp = ZLoop::for_name("log Current", 1e-9);
        lp.settle(1e-3, 0.01, 0.005, 1.0).unwrap();
        let z_at_50 = lp.z_m;
        lp.setpoint = 100e-12;
        lp.settle(1e-3, 0.01, 0.005, 1.0).unwrap();
        assert!(lp.z_m < z_at_50, "more current means closer");
        let dz = z_at_50 - lp.z_m;
        // ln 2 over 2 kappa: the exponential's own answer.
        let expected = (2.0f64).ln() / lp.plant.current_decay_per_m;
        assert!(
            (dz - expected).abs() / expected < 0.05,
            "{dz} vs {expected}"
        );
    }

    #[test]
    fn a_negative_frequency_loop_retracts_when_the_shift_falls() {
        let mut lp = ZLoop::for_name("Frequency (neg)", 0.5e-9);
        assert_eq!(lp.slope, -1.0);
        assert_eq!(lp.law, ZLaw::Linear);
        let start = lp.sample();
        assert!(
            start.freq_shift_hz < lp.setpoint,
            "closer than the setpoint wants"
        );
        lp.settle(1e-3, 0.02, 0.005, 1.0).expect("settles");
        assert!(lp.z_m > 0.5e-9, "retracted");
        assert!((lp.sample().freq_shift_hz - lp.setpoint).abs() < 0.1);
    }

    #[test]
    fn a_disengaged_loop_holds_and_a_placed_tip_stays() {
        let mut lp = ZLoop::for_name("log Current", 0.4e-9);
        lp.disengage();
        let z = lp.z_m;
        for _ in 0..100 {
            lp.step(1e-3);
        }
        assert_eq!(lp.z_m, z);
        lp.place(2e-9);
        assert_eq!(lp.step(1e-3).z_m, 2e-9);
        lp.engage();
        lp.settle(1e-3, 0.01, 0.005, 1.0)
            .expect("settles after engaging");
    }

    #[test]
    fn a_faster_time_constant_settles_faster() {
        let mut slow = ZLoop::for_name("log Current", 1e-9);
        slow.time_constant_s = 5e-3;
        let mut fast = ZLoop::for_name("log Current", 1e-9);
        fast.time_constant_s = 0.5e-3;
        let slow_t = slow.settle(1e-4, 0.01, 0.002, 2.0).unwrap();
        let fast_t = fast.settle(1e-4, 0.01, 0.002, 2.0).unwrap();
        assert!(fast_t < slow_t, "{fast_t} < {slow_t}");
    }
}
