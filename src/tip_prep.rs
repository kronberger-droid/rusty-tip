use crate::action_driver::ActionDriver;
use crate::actions::{Action, TipCheckMethod, TipState};
use crate::error::NanonisError;
use crate::types::{SignalIndex, TipShape};
use log::info;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

// ============================================================================
// TIP PREPARATION CONSTANTS
// ============================================================================

/// Report progress every N cycles
const STATUS_INTERVAL: usize = 10;

/// Pulse width for tip pulsing during bad_loop (ms)
const PULSE_WIDTH_MS: u64 = 500;

/// Data collection duration before pulse (ms)
const PRE_PULSE_DATA_COLLECTION_MS: u64 = 50;

/// Data collection duration after pulse (ms)
const POST_PULSE_DATA_COLLECTION_MS: u64 = 50;

/// Wait time after clearing TCP buffer to accumulate fresh data (ms)
const BUFFER_CLEAR_WAIT_MS: u64 = 500;

/// Wait time after approach for signal to stabilize before first measurement (ms)
/// After approach, the tip needs time to settle and the signal to stabilize
/// Increase this if you get incorrect initial tip state readings
const POST_APPROACH_SETTLE_TIME_MS: u64 = 2000;

/// Wait time after reposition (during pulse cycles) for signal to stabilize (ms)
/// After repositioning and approach in bad_loop, signal needs time to settle
/// Shorter than initial approach since it's a smaller movement
const POST_REPOSITION_SETTLE_TIME_MS: u64 = 1000;

/// Bias sweep range for stability checking (V)
const STABILITY_BIAS_SWEEP_RANGE: (f32, f32) = (-2.0, 2.0);

/// Number of steps in bias sweep for stability checking
const STABILITY_BIAS_SWEEP_STEPS: u16 = 1000;

/// Period per step in bias sweep for stability checking (ms)
const STABILITY_BIAS_SWEEP_PERIOD_MS: u64 = 200;

/// Maximum duration for stability check (seconds)
const STABILITY_CHECK_MAX_DURATION_SECS: u64 = 100;

/// Loop types based on tip shape - simple and direct
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoopType {
    BadLoop,
    GoodLoop,
    StableLoop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PolaritySign {
    Positive,
    Negative,
}

impl PolaritySign {
    pub fn opposite(&self) -> Self {
        match self {
            PolaritySign::Positive => PolaritySign::Negative,
            PolaritySign::Negative => PolaritySign::Positive,
        }
    }
}

pub struct RandomPolaritySwitch {
    pub switch_every_n_pulses: u32,
}

pub enum PulseMethod {
    Fixed {
        voltage: f32,
        polarity: PolaritySign,
        random_switch: Option<RandomPolaritySwitch>,
    },
    Stepping {
        voltage_bounds: (f32, f32),
        voltage_steps: u16,
        cycles_before_step: u16,
        threshold: Box<dyn Fn(f32) -> f32 + Send + Sync>,
        polarity: PolaritySign,
        random_switch: Option<RandomPolaritySwitch>,
    },
}

impl PulseMethod {
    pub fn stepping_fixed_threshold(
        voltage_bounds: (f32, f32),
        voltage_steps: u16,
        cycles_before_step: u16,
        threshold: f32,
        polarity: PolaritySign,
        random_switch: Option<RandomPolaritySwitch>,
    ) -> PulseMethod {
        let threshold = threshold.abs();
        PulseMethod::Stepping {
            voltage_bounds,
            voltage_steps,
            cycles_before_step,
            threshold: Box::new(move |_| threshold),
            polarity,
            random_switch,
        }
    }
}

pub struct TipControllerConfig {
    pub freq_shift_index: SignalIndex,
    pub sharp_tip_bounds: (f32, f32),
    pub pulse_method: PulseMethod,
    pub allowed_change_for_stable: f32,
    pub max_cycles: Option<usize>,
    pub max_duration: Option<Duration>,
    pub check_stability: bool,
}

impl Default for TipControllerConfig {
    fn default() -> Self {
        Self {
            freq_shift_index: 76u8.into(),
            sharp_tip_bounds: (-2.0, 0.0),
            pulse_method: PulseMethod::Fixed {
                voltage: 4.0,
                polarity: PolaritySign::Positive,
                random_switch: None,
            },
            allowed_change_for_stable: 0.2,
            max_cycles: Some(1000),
            max_duration: Some(Duration::from_secs(3600)), // 1 hour
            check_stability: true,
        }
    }
}

/// Enhanced tip controller with pulse voltage stepping
pub struct TipController {
    driver: ActionDriver,
    config: TipControllerConfig,

    // State tracking
    current_pulse_voltage: f32,
    current_tip_shape: TipShape,
    cycles_without_change: u32,
    cycle_count: u32,

    // Multi-signal history for bias adjustment and analysis
    signal_histories: HashMap<SignalIndex, VecDeque<f32>>,
    max_history_size: usize,

    // Loop termination safeguards
    max_cycles: Option<usize>,
    max_duration: Option<Duration>,
    loop_start_time: Option<std::time::Instant>,

    // Graceful shutdown support
    shutdown_requested: Option<Arc<AtomicBool>>,

    // Polarity tracking
    base_polarity: PolaritySign,
    pulse_count_for_random: u32,
}

impl TipController {
    /// Create new tip controller with basic signal bounds
    pub fn new(driver: ActionDriver, config: TipControllerConfig) -> Self {
        let initial_voltage = match &config.pulse_method {
            PulseMethod::Fixed { voltage, .. } => *voltage,
            PulseMethod::Stepping { voltage_bounds, .. } => voltage_bounds.0,
        };
        let base_polarity = match &config.pulse_method {
            PulseMethod::Fixed { polarity, .. } => *polarity,
            PulseMethod::Stepping { polarity, .. } => *polarity,
        };
        let max_cycles = config.max_cycles;
        let max_duration = config.max_duration;
        Self {
            driver,
            config,
            current_pulse_voltage: initial_voltage,
            current_tip_shape: TipShape::Blunt,
            cycles_without_change: 0,
            cycle_count: 0,
            signal_histories: HashMap::new(),
            max_history_size: 100,
            max_cycles,
            max_duration,
            loop_start_time: None,
            shutdown_requested: None,
            base_polarity,
            pulse_count_for_random: 0,
        }
    }

    /// Set shutdown flag for graceful termination
    pub fn set_shutdown_flag(&mut self, flag: Arc<AtomicBool>) {
        self.shutdown_requested = Some(flag);
    }

    /// Check if this pulse should use opposite polarity
    fn should_use_opposite_polarity(&self) -> bool {
        match &self.config.pulse_method {
            PulseMethod::Stepping {
                random_switch: Some(switch),
                ..
            }
            | PulseMethod::Fixed {
                random_switch: Some(switch),
                ..
            } => {
                // Check if current pulse count is a multiple of switch interval
                self.pulse_count_for_random > 0
                    && self.pulse_count_for_random % switch.switch_every_n_pulses == 0
            }
            _ => false,
        }
    }

    /// Get the signed pulse voltage based on current polarity
    fn get_signed_pulse_voltage(&self) -> f32 {
        let polarity = if self.should_use_opposite_polarity() {
            self.base_polarity.opposite()
        } else {
            self.base_polarity
        };

        match polarity {
            PolaritySign::Positive => self.current_pulse_voltage,
            PolaritySign::Negative => -self.current_pulse_voltage,
        }
    }

    /// Track a signal value in history
    pub fn track_signal(&mut self, signal_index: SignalIndex, value: f32) {
        let history = self.signal_histories.entry(signal_index).or_default();

        // Add new value to front
        history.push_front(value);

        // Maintain size limit
        while history.len() > self.max_history_size {
            history.pop_back();
        }
    }

    /// Get signal change (latest - previous) for a specific signal
    pub fn get_signal_change(&self, signal_index: SignalIndex) -> Option<f32> {
        if let Some(history) = self.signal_histories.get(&signal_index) {
            if history.len() >= 2 {
                Some(history[0] - history[1]) // Latest - Previous
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Get signal history for a specific signal (most recent first)
    pub fn get_signal_history(&self, signal_index: SignalIndex) -> Option<&VecDeque<f32>> {
        self.signal_histories.get(&signal_index)
    }

    pub fn get_last_signal(&self, signal_index: SignalIndex) -> Option<f32> {
        match self.get_signal_history(signal_index) {
            Some(history) => history.iter().last().copied(),
            None => None,
        }
    }

    /// Clear all signal histories
    pub fn clear_all_histories(&mut self) {
        self.signal_histories.clear();
    }

    /// Clear history for a specific signal
    pub fn clear_signal_history(&mut self, signal_index: SignalIndex) {
        self.signal_histories.remove(&signal_index);
    }

    /// Check if current signal represents a significant change from recent stable period
    fn has_significant_change(&self, signal_index: SignalIndex) -> (bool, f32) {
        // Only check for stepping if PulseMethod is Stepping
        let threshold_fn = match &self.config.pulse_method {
            PulseMethod::Stepping { threshold, .. } => threshold,
            PulseMethod::Fixed { .. } => return (false, 0.0), // No stepping for fixed method
        };

        if let Some(history) = self.signal_histories.get(&signal_index) {
            if history.len() < 2 {
                // First signal - consider it a significant change to initialize properly
                (true, 0.0)
            } else {
                // Compare only against signals from the current stable period
                // cycles_without_change tells us how many recent signals were stable
                let stable_period_size =
                    (self.cycles_without_change as usize).min(history.len() - 1);

                if stable_period_size == 0 {
                    // No stable period yet, compare against last signal
                    let current_signal = history[0];
                    let last_signal = history[1];
                    let threshold = threshold_fn(current_signal);

                    log::debug!(
                        "Last signal: {:.3e} | Current threshold: {:.3e}",
                        last_signal,
                        threshold
                    );

                    let change = current_signal - last_signal;
                    let has_change = change.abs() >= threshold;

                    (has_change, change)
                } else {
                    // Compare against mean of current stable period (skip current signal at index 0)
                    let current_signal = history[0];
                    let stable_signals: Vec<f32> = history
                        .iter()
                        .skip(1)
                        .take(stable_period_size)
                        .cloned()
                        .collect();
                    let stable_mean =
                        stable_signals.iter().sum::<f32>() / stable_signals.len() as f32;

                    let threshold = threshold_fn(current_signal);
                    log::debug!(
                        "Current: {:.3e} | Stable mean: {:.3e} | Threshold: {:.3e}",
                        current_signal,
                        stable_mean,
                        threshold
                    );

                    let change = current_signal - stable_mean;
                    let has_change = change.abs() >= threshold;
                    (has_change, change)
                }
            }
        } else {
            // No history yet - consider it a significant change
            (true, 0.0)
        }
    }

    /// Step up the pulse voltage if possible
    fn step_pulse_voltage(&mut self) -> bool {
        // Only step if using stepping method
        let (voltage_bounds, voltage_steps) = match &self.config.pulse_method {
            PulseMethod::Stepping {
                voltage_bounds,
                voltage_steps,
                ..
            } => (*voltage_bounds, *voltage_steps),
            PulseMethod::Fixed { .. } => return false, // No stepping for fixed method
        };

        // Calculate step size
        let step_size = (voltage_bounds.1 - voltage_bounds.0) / voltage_steps as f32;
        let new_pulse = (self.current_pulse_voltage + step_size).min(voltage_bounds.1);

        if new_pulse > self.current_pulse_voltage {
            info!(
                "Stepping pulse voltage: {:.3}V -> {:.3}V",
                self.current_pulse_voltage, new_pulse
            );
            self.current_pulse_voltage = new_pulse;
            self.cycles_without_change = 0; // Reset counter after stepping
            true
        } else {
            log::debug!("Pulse voltage already at maximum: {:.3}V", voltage_bounds.1);
            self.cycles_without_change = 0; // Reset counter even if at max
            false
        }
    }

    /// Update signal history and step pulse voltage if needed
    fn update_pulse_voltage(&mut self) {
        let (is_significant, change) = self.has_significant_change(self.config.freq_shift_index);

        // Get cycles_before_step from config, or use 0 for Fixed method
        let cycles_before_step = match &self.config.pulse_method {
            PulseMethod::Stepping {
                cycles_before_step,
                voltage_bounds,
                ..
            } => (*cycles_before_step, *voltage_bounds),
            PulseMethod::Fixed { .. } => return, // No stepping for fixed method
        };

        // Check for significant change and respond accordingly
        if is_significant && change >= 0.0 {
            // Positive significant change - reset to minimum voltage
            self.cycles_without_change = 0;
            self.current_pulse_voltage = cycles_before_step.1 .0; // voltage_bounds.0 (min)
            log::debug!(
                "Positive significant change detected, resetting pulse voltage to minimum: {:.3}V",
                self.current_pulse_voltage
            );
        } else if is_significant {
            log::warn!("Negative significant change detected!");
            self.cycles_without_change += 1;

            // Check if we need to step the pulse voltage
            if self.cycles_without_change >= cycles_before_step.0 as u32 {
                self.step_pulse_voltage();
            }
        } else {
            // No significant change
            self.cycles_without_change += 1;

            // Check if we need to step the pulse voltage
            if self.cycles_without_change >= cycles_before_step.0 as u32 {
                self.step_pulse_voltage();
            }
        }
    }
}

impl TipController {
    /// Main control loop - with pulse voltage stepping
    pub fn run(&mut self) -> Result<(), NanonisError> {
        self.pre_loop_initialization()?;
        self.loop_start_time = Some(std::time::Instant::now());

        while self.current_tip_shape != TipShape::Stable {
            // Check cycle limit
            if let Some(max) = self.max_cycles {
                if self.cycle_count >= max as u32 {
                    return Err(NanonisError::TimeoutWithContext {
                        context: format!("Max cycles ({}) exceeded", max),
                    });
                }
            }

            // Check wall-clock timeout
            if let Some(max_dur) = self.max_duration {
                if let Some(start_time) = self.loop_start_time {
                    if start_time.elapsed() > max_dur {
                        return Err(NanonisError::TimeoutWithContext {
                            context: format!("Max duration ({:?}) exceeded", max_dur),
                        });
                    }
                }
            }

            // Check shutdown flag
            if let Some(flag) = &self.shutdown_requested {
                if flag.load(Ordering::SeqCst) {
                    info!("Shutdown requested at cycle {}", self.cycle_count);
                    return Err(NanonisError::Shutdown);
                }
            }

            // Execute one control cycle
            self.cycle_count += 1;

            // Periodic status report
            if self.cycle_count % STATUS_INTERVAL as u32 == 0 {
                if let Some(start_time) = self.loop_start_time {
                    let elapsed = start_time.elapsed();
                    info!(
                        "Status: cycle={}, state={:?}, pulse_v={:.2}V, elapsed={:.1}s",
                        self.cycle_count,
                        self.current_tip_shape,
                        self.current_pulse_voltage,
                        elapsed.as_secs_f32()
                    );
                }
            }

            // Execute based on state
            match self.current_tip_shape {
                TipShape::Blunt => {
                    self.bad_loop()?;
                    continue;
                }
                TipShape::Sharp => {
                    self.good_loop()?;
                    continue;
                }
                TipShape::Stable => {
                    info!(
                        "STABLE achieved after {} cycles! Final pulse voltage: {:.3}V",
                        self.cycle_count, self.current_pulse_voltage
                    );
                    break;
                }
            }
        }
        Ok(())
    }

    /// Bad loop - execute recovery sequence with stable signal monitoring
    /// Sequence: capture_stable_before → pulse → capture_stable_after → withdraw → move → approach → check
    fn bad_loop(&mut self) -> Result<(), NanonisError> {
        let signed_voltage = self.get_signed_pulse_voltage();

        // Log if using opposite polarity
        if self.should_use_opposite_polarity() {
            info!(
                "Random polarity switch: using {:?} for this pulse ({:.3}V)",
                self.base_polarity.opposite(),
                signed_voltage
            );
        }

        log::debug!(
            "Executing PulseRetract Sequence with pulse = {:.3} V",
            signed_voltage
        );

        self.driver
            .run(Action::PulseRetract {
                pulse_width: Duration::from_millis(PULSE_WIDTH_MS),
                pulse_height_v: signed_voltage,
            })
            .with_data_collection(
                Duration::from_millis(PRE_PULSE_DATA_COLLECTION_MS),
                Duration::from_millis(POST_PULSE_DATA_COLLECTION_MS),
            )
            .execute()?;

        // Increment pulse counter for random switching
        self.pulse_count_for_random += 1;

        log::debug!("Repositioning...");

        self.driver
            .run(Action::SafeReposition {
                x_steps: 2,
                y_steps: 2,
            })
            .go()?;

        // Wait for signal to stabilize after reposition/approach
        log::debug!(
            "Waiting {}ms for signal to stabilize after reposition...",
            POST_REPOSITION_SETTLE_TIME_MS
        );
        std::thread::sleep(Duration::from_millis(POST_REPOSITION_SETTLE_TIME_MS));

        // let amplitude_reached: bool = self
        //     .driver
        //     .run(Action::ReachedTargedAmplitude)
        //     .expecting()?;
        let amplitude_reached = true;

        if amplitude_reached {
            let tip_state: TipState = self
                .driver
                .run(Action::CheckTipState {
                    method: TipCheckMethod::SignalBounds {
                        signal: self.config.freq_shift_index,
                        bounds: self.config.sharp_tip_bounds,
                    },
                })
                .expecting()?;

            self.current_tip_shape = tip_state.shape;

            // Track the frequency shift signal if available
            if let Some(freq_shift_value) = tip_state
                .measured_signals
                .get(&self.config.freq_shift_index)
                .copied()
            {
                self.track_signal(self.config.freq_shift_index, freq_shift_value);
            } else {
                log::warn!(
                    "CheckTipState did not return frequency shift signal (index: {})",
                    self.config.freq_shift_index.0 .0
                );
            }

            // Update pulse voltage based on signal changes (stepping logic)
            self.update_pulse_voltage();
        } else {
            log::debug!("Amplitude not reached. Assuming blunt tip");
            self.current_tip_shape = TipShape::Blunt;
        }

        Ok(())
    }

    /// Good loop - monitoring, increment good count
    fn good_loop(&mut self) -> Result<(), NanonisError> {
        // If stability checking is disabled, mark tip as stable immediately
        if !self.config.check_stability {
            info!("Stability checking disabled - marking tip as stable");
            self.current_tip_shape = TipShape::Stable;
            return Ok(());
        }

        // Otherwise, perform bias sweep to check stability
        let stability_result: crate::actions::StabilityResult = self
            .driver
            .run(Action::CheckTipStability {
                method: crate::actions::TipStabilityMethod::BiasSweepResponse {
                    signal: self.config.freq_shift_index,
                    bias_range: STABILITY_BIAS_SWEEP_RANGE,
                    sweep_steps: STABILITY_BIAS_SWEEP_STEPS,
                    period: Duration::from_millis(STABILITY_BIAS_SWEEP_PERIOD_MS),
                    allowed_signal_change: self.config.allowed_change_for_stable,
                },
                max_duration: Duration::from_secs(STABILITY_CHECK_MAX_DURATION_SECS),
                abort_on_damage_signs: false,
            })
            .expecting()?;

        // Track signal values from stability monitoring (use last measured value)
        if let Some(signal_values) = stability_result
            .measured_values
            .get(&self.config.freq_shift_index)
        {
            if let Some(&last_value) = signal_values.last() {
                self.track_signal(self.config.freq_shift_index, last_value);
            }
        }

        // Update tip shape based on stability result
        self.current_tip_shape = if stability_result.is_stable {
            TipShape::Stable
        } else {
            self.driver
                .run(Action::SafeReposition {
                    x_steps: 2,
                    y_steps: 2,
                })
                .go()?;

            self.driver
                .run(Action::CheckTipState {
                    method: TipCheckMethod::SignalBounds {
                        signal: self.config.freq_shift_index,
                        bounds: self.config.sharp_tip_bounds,
                    },
                })
                .expecting()?
        };

        Ok(())
    }

    fn pre_loop_initialization(&mut self) -> Result<(), NanonisError> {
        log::debug!("Running pre loop initialization");

        self.driver.client_mut().set_bias(-500e-3)?;
        self.driver.client_mut().z_ctrl_setpoint_set(100e-12)?;

        info!("Executing Initial Approach");

        self.driver
            .run(Action::AutoApproach {
                wait_until_finished: true,
                timeout: Duration::from_secs(600), // 10 minutes timeout for approach
            })
            .go()?;

        // Clear TCP buffer to discard any stale data from before approach
        log::debug!("Clearing TCP buffer to get fresh frequency shift data");
        self.driver.clear_tcp_buffer();

        // Wait briefly for fresh data to accumulate in buffer
        std::thread::sleep(Duration::from_millis(BUFFER_CLEAR_WAIT_MS));

        // Wait for signal to stabilize after approach
        info!(
            "Waiting {}s for signal to stabilize after approach...",
            POST_APPROACH_SETTLE_TIME_MS as f32 / 1000.0
        );
        std::thread::sleep(Duration::from_millis(POST_APPROACH_SETTLE_TIME_MS));

        let initial_tip_state: TipState = self
            .driver
            .run(Action::CheckTipState {
                method: TipCheckMethod::SignalBounds {
                    signal: self.config.freq_shift_index,
                    bounds: self.config.sharp_tip_bounds,
                },
            })
            .expecting()?;

        info!(
            "Current tip shape: {:?} (confidence: {:.3})",
            initial_tip_state.shape, initial_tip_state.confidence
        );

        self.current_tip_shape = initial_tip_state.shape;

        Ok(())
    }
}
