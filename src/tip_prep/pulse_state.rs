use std::collections::VecDeque;

use crate::controller_types::{
    PolaritySign, PulseMethod, RandomPolaritySwitch,
};

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
            } => (*voltage as f64, *polarity, random_polarity_switch.clone()),
            PulseMethod::Stepping {
                voltage_bounds,
                polarity,
                random_polarity_switch,
                ..
            } => (
                voltage_bounds.0 as f64,
                *polarity,
                random_polarity_switch.clone(),
            ),
            PulseMethod::Linear {
                voltage_bounds,
                polarity,
                random_polarity_switch,
                ..
            } => (
                voltage_bounds.0 as f64,
                *polarity,
                random_polarity_switch.clone(),
            ),
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
            PulseMethod::Fixed { voltage, .. } => *voltage as f64,
            PulseMethod::Stepping { voltage_bounds, .. } => {
                voltage_bounds.0 as f64
            }
            PulseMethod::Linear { voltage_bounds, .. } => {
                voltage_bounds.0 as f64
            }
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
                && self.pulse_count % switch.switch_every_n_pulses == 0
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

        sign * method.max_voltage() as f64
    }

    /// Update pulse voltage magnitude based on the latest freq_shift reading.
    pub fn update_voltage(
        &mut self,
        method: &PulseMethod,
        freq_shift: Option<f64>,
    ) {
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
                            && self.freq_shift_history.len()
                                > self.cycles_without_change
                        {
                            let n = self.cycles_without_change;
                            let sum: f64 = self
                                .freq_shift_history
                                .iter()
                                .skip(1)
                                .take(n)
                                .sum();
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
                                (
                                    change.abs() > *threshold_value as f64,
                                    change >= 0.0,
                                )
                            }
                            None => (true, true),
                        }
                    }
                    None => (true, true),
                };

                if significant && positive_change {
                    self.cycles_without_change = 0;
                    self.current_voltage = voltage_bounds.0 as f64;
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
                    let step_size = (voltage_bounds.1 - voltage_bounds.0)
                        as f64
                        / *voltage_steps as f64;
                    let new_voltage = (self.current_voltage + step_size)
                        .min(voltage_bounds.1 as f64);
                    if new_voltage > self.current_voltage {
                        log::info!(
                            "Stepping pulse voltage: {:.3}V -> {:.3}V",
                            self.current_voltage,
                            new_voltage
                        );
                        self.current_voltage = new_voltage;
                    } else {
                        log::debug!(
                            "Pulse voltage already at maximum: {:.3}V",
                            voltage_bounds.1
                        );
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
                    if fs < linear_clamp.0 as f64 || fs > linear_clamp.1 as f64
                    {
                        self.current_voltage = voltage_bounds.1 as f64;
                        log::info!(
                            "Linear pulse: freq_shift {:.2} Hz outside range [{:.2}, {:.2}] Hz -> using max voltage {:.2}V",
                            fs,
                            linear_clamp.0,
                            linear_clamp.1,
                            voltage_bounds.1
                        );
                    } else {
                        let slope = (voltage_bounds.1 - voltage_bounds.0)
                            as f64
                            / (linear_clamp.1 - linear_clamp.0) as f64;
                        let intercept = voltage_bounds.0 as f64
                            - slope * linear_clamp.0 as f64;
                        self.current_voltage = slope * fs + intercept;
                        log::info!(
                            "Linear pulse: freq_shift {:.2} Hz in range [{:.2}, {:.2}] Hz -> calculated voltage {:.2}V",
                            fs,
                            linear_clamp.0,
                            linear_clamp.1,
                            self.current_voltage
                        );
                    }
                }

                self.last_freq_shift = freq_shift;
            }
        }
    }
}
