use crate::{
    actions::{Action, ActionChain},
    SignalIndex,
};
use std::time::Duration;

/// Simple enum representing built-in and custom action sequences
#[derive(Debug, Clone)]
pub enum ActionSequence {
    /// Emergency tip recovery sequence
    BadAction,

    /// Safe tip approach with bias zeroing
    SafeApproach,

    /// Emergency withdrawal with bias zeroing
    EmergencyWithdraw,

    /// Custom sequence from ActionChain conversion
    Custom(Vec<Action>),
}

impl ActionSequence {
    /// Convert sequence to ActionChain for execution
    pub fn to_action_chain(self) -> ActionChain {
        match self {
            ActionSequence::BadAction => ActionChain::new(vec![
                // Step 1: Initial approach (if not already approached)
                Action::AutoApproach,
                // Step 2: Pulse operation (simulated for now)
                Action::BiasPulse {
                    wait_until_done: true,
                    pulse_width_s: Duration::from_millis(500),
                    bias_value_v: 3.0,
                    z_controller_hold: 0,
                    pulse_mode: 0,
                },
                // Step 3: Withdraw tip
                Action::Withdraw {
                    wait_until_finished: true,
                    timeout_ms: 5000,
                },
                // Step 4: Move to new position (3nm offset)
                Action::MovePiezoRelative {
                    delta: crate::types::Position::new(3e-9, 3e-9),
                },
                // Step 5: Approach at new position
                Action::AutoApproach,
                // Step 6: Check tip state (read signal names to verify recovery)
                Action::ReadSignal {
                    signal: SignalIndex(0),
                    wait_for_newest: true,
                },
            ]),

            ActionSequence::SafeApproach => ActionChain::new(vec![
                Action::SetBias { voltage: 0.0 },
                Action::Wait {
                    duration: Duration::from_millis(500),
                },
                Action::AutoApproach,
            ]),

            ActionSequence::EmergencyWithdraw => ActionChain::new(vec![
                Action::SetBias { voltage: 0.0 },
                Action::Withdraw {
                    wait_until_finished: true,
                    timeout_ms: 1000,
                },
            ]),

            ActionSequence::Custom(actions) => ActionChain::new(actions),
        }
    }

    /// Get human-readable name for logging
    pub fn name(&self) -> &str {
        match self {
            ActionSequence::BadAction => "Bad Action Recovery",
            ActionSequence::SafeApproach => "Safe Approach",
            ActionSequence::EmergencyWithdraw => "Emergency Withdraw",
            ActionSequence::Custom(_) => "Custom Sequence",
        }
    }
}
