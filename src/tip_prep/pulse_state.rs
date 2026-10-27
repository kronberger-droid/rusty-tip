use std::collections::VecDeque;

use crate::controller_types::{PolaritySign, PulseMethod, RandomPolaritySwitch};

/// Mutable state for pulse voltage strategies that evolve across cycles.
pub struct PulseState {
    pub current_voltage: f64,
    pub cycles_without_change: usize,
    pub last_freq_shift: Option<f64>,
    /// Rolling history of freq_shift readings for stable-mean comparison
    pub freq_shift_history: VecDeque<f64>,
    /// Base polarity from config
    pub base_polarity: PolaritySign,
    /// Pulse counter for random polarity switching
    pub pulse_count: u32,
    /// Random polarity switch config (cloned from PulseMethod)
    pub random_switch: Option<RandomPolaritySwitch>,
}

impl PulseState {
    pub fn new(method: &PulseMethod) -> Self {
        let (initial_voltage, base_polarity, random_switch) = match method {
            PulseMethod::Fixed {
                voltage,
                polarity,
                random_polarity_switch,
            } => ((*voltage), *polarity, random_polarity_switch.clone()),
            PulseMethod::Stepping {
                voltage_bounds,
                polarity,
                random_polarity_switch,
                ..
            } => (voltage_bounds.0, *polarity, random_polarity_switch.clone()),
            PulseMethod::Linear {
                voltage_bounds,
                polarity,
                random_polarity_switch,
                ..
            } => (voltage_bounds.0, *polarity, random_polarity_switch.clone()),
        };
        Self {
            current_voltage: initial_voltage,
            cycles_without_change: 0,
            last_freq_shift: None,
            freq_shift_history: VecDeque::with_capacity(100),
            base_polarity,
            pulse_count: 0,
            random_switch,
        }
    }

    /// Reset pulse state after stability failure -- back to minimum voltage.
    pub fn reset(&mut self, method: &PulseMethod) {
        self.current_voltage = match method {
            PulseMethod::Fixed { voltage, .. } => *voltage,
            PulseMethod::Stepping { voltage_bounds, .. } => voltage_bounds.0,
            PulseMethod::Linear { voltage_bounds, .. } => voltage_bounds.0,
        };
        self.cycles_without_change = 0;
        self.last_freq_shift = None;
        self.freq_shift_history.clear();
    }

    /// Get the signed voltage for the next pulse.
    ///
    /// Applies polarity sign and random polarity switching to the magnitude.
    pub fn signed_voltage(&mut self) -> f64 {
        self.pulse_count += 1;

        let effective_polarity = if self.should_use_opposite_polarity() {
            self.base_polarity.opposite()
        } else {
            self.base_polarity
        };

        let sign = match effective_polarity {
            PolaritySign::Positive => 1.0,
            PolaritySign::Negative => -1.0,
        };

        sign * self.current_voltage
    }

    pub fn should_use_opposite_polarity(&self) -> bool {
        if let Some(ref switch) = self.random_switch {
            switch.enabled
                && self.pulse_count > 0
                && self
                    .pulse_count
                    .is_multiple_of(switch.switch_every_n_pulses)
        } else {
            false
        }
    }

    /// Compute the signed max-voltage pulse (used after stability failure).
    ///
    /// Counts as a pulse for random-polarity-switch purposes: increments
    /// `pulse_count` and, if this cycle hits a switch boundary, fires at
    /// the opposite polarity. Matches V1 `execute_max_pulse` semantics.
    pub fn fire_max_pulse_voltage(&mut self, method: &PulseMethod) -> f64 {
        self.pulse_count += 1;

        let effective_polarity = if self.should_use_opposite_polarity() {
            self.base_polarity.opposite()
        } else {
            self.base_polarity
        };

        let sign = match effective_polarity {
            PolaritySign::Positive => 1.0,
            PolaritySign::Negative => -1.0,
        };

        sign * method.max_voltage()
    }

    /// Update pulse voltage magnitude based on the latest freq_shift reading.
    pub fn update_voltage(&mut self, method: &PulseMethod, freq_shift: Option<f64>) {
        match method {
            PulseMethod::Fixed { .. } => {
                // Fixed: voltage never changes
            }

            PulseMethod::Stepping {
                voltage_bounds,
                voltage_steps,
                cycles_before_step,
                threshold_value,
                ..
            } => {
                // Push current reading into history
                if let Some(fs) = freq_shift {
                    self.freq_shift_history.push_front(fs);
                    if self.freq_shift_history.len() > 100 {
                        self.freq_shift_history.pop_back();
                    }
                }

                // Compare current reading against reference value
                let (significant, positive_change) = match freq_shift {
                    Some(current) => {
                        let reference = if self.cycles_without_change > 0
                            && self.freq_shift_history.len() > self.cycles_without_change
                        {
                            let n = self.cycles_without_change;
                            let sum: f64 = self.freq_shift_history.iter().skip(1).take(n).sum();
                            let mean = sum / n as f64;
                            log::debug!(
                                "Current: {:.3e} | Stable mean: {:.3e} | Threshold: {:.3e}",
                                current,
                                mean,
                                threshold_value
                            );
                            Some(mean)
                        } else if let Some(last) = self.last_freq_shift {
                            log::debug!(
                                "Last signal: {:.3e} | Current threshold: {:.3e}",
                                last,
                                threshold_value
                            );
                            Some(last)
                        } else {
                            None
                        };

                        match reference {
                            Some(ref_val) => {
                                let change = current - ref_val;
                                (change.abs() > *threshold_value, change >= 0.0)
                            }
                            None => (true, true),
                        }
                    }
                    None => (true, true),
                };

                if significant && positive_change {
                    self.cycles_without_change = 0;
                    self.current_voltage = voltage_bounds.0;
                    log::debug!(
                        "Positive significant change detected, resetting pulse voltage to minimum: {:.3}V",
                        self.current_voltage
                    );
                } else if significant {
                    log::warn!("Negative significant change detected!");
                    self.cycles_without_change += 1;
                } else {
                    self.cycles_without_change += 1;
                }

                if self.cycles_without_change >= *cycles_before_step as usize {
                    let step_size = (voltage_bounds.1 - voltage_bounds.0) / *voltage_steps as f64;
                    let new_voltage = (self.current_voltage + step_size).min(voltage_bounds.1);
                    if new_voltage > self.current_voltage {
                        log::info!(
                            "Stepping pulse voltage: {:.3}V -> {:.3}V",
                            self.current_voltage,
                            new_voltage
                        );
                        self.current_voltage = new_voltage;
                    } else {
                        log::debug!("Pulse voltage already at maximum: {:.3}V", voltage_bounds.1);
                    }
                    self.cycles_without_change = 0;
                }

                self.last_freq_shift = freq_shift;
            }

            PulseMethod::Linear {
                voltage_bounds,
                linear_clamp,
                ..
            } => {
                if let Some(fs) = freq_shift {
                    self.current_voltage = linear_voltage(fs, *voltage_bounds, *linear_clamp);
                    let inside = (linear_clamp.0..=linear_clamp.1).contains(&fs);
                    log::info!(
                        "Linear pulse: freq_shift {:.2} Hz {} [{:.2}, {:.2}] Hz -> {:.2}V",
                        fs,
                        if inside { "in" } else { "outside" },
                        linear_clamp.0,
                        linear_clamp.1,
                        self.current_voltage
                    );
                }

                self.last_freq_shift = freq_shift;
            }
        }
    }
}

/// The Linear strategy's pulse magnitude for a frequency-shift reading.
///
/// A blunter tip reads further below zero, so the clamp's upper end is the one
/// nearest the sharp window. The gentlest pulse (`voltage_bounds.0`) falls
/// there and rises linearly to the hardest (`voltage_bounds.1`) at the clamp's
/// lower end; outside the clamp the hardest applies. Large pulses reshape the
/// apex unpredictably, so a tip close to sharp is only nudged.
pub fn linear_voltage(
    freq_shift: f64,
    voltage_bounds: (f64, f64),
    linear_clamp: (f64, f64),
) -> f64 {
    let (gentle, hard) = voltage_bounds;
    let (far, near) = linear_clamp;
    if !(far..=near).contains(&freq_shift) {
        return hard;
    }
    gentle + (near - freq_shift) / (near - far) * (hard - gentle)
}

#[cfg(test)]
mod tests {
    use super::linear_voltage;

    const BOUNDS: (f64, f64) = (4.0, 8.0);
    const CLAMP: (f64, f64) = (-5.0, -1.5);

    #[test]
    fn linear_is_gentlest_next_to_the_window_and_hardest_far_from_it() {
        assert_eq!(linear_voltage(-1.5, BOUNDS, CLAMP), 4.0);
        assert_eq!(linear_voltage(-5.0, BOUNDS, CLAMP), 8.0);
        let near = linear_voltage(-1.6, BOUNDS, CLAMP);
        let far = linear_voltage(-4.9, BOUNDS, CLAMP);
        assert!(near < 4.2 && far > 7.8, "near {near} V, far {far} V");
    }

    #[test]
    fn linear_outside_the_clamp_is_the_hardest_pulse() {
        assert_eq!(linear_voltage(-6.0, BOUNDS, CLAMP), 8.0);
        assert_eq!(linear_voltage(76.29, BOUNDS, CLAMP), 8.0);
        assert_eq!(linear_voltage(-76.29, BOUNDS, CLAMP), 8.0);
    }

    #[test]
    fn linear_has_no_step_at_the_far_end_of_the_clamp() {
        let edge = linear_voltage(-5.0, BOUNDS, CLAMP);
        let beyond = linear_voltage(-5.0 - 1e-9, BOUNDS, CLAMP);
        assert_eq!(edge, beyond);
    }
}
