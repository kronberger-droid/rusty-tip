use std::time::Duration;

use log::info;
use rusty_tip::{
    Action, ActionChain, ActionDriver, ActionSequence, ExecutionPriority, MachineRepresentation,
    NanonisClient, Position, ScanAction, ScanDirection, SignalIndex,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Create client and driver
    let driver = ActionDriver::new("127.0.0.1", 6501)?;

    let mut machine = MachineRepresentation::new(driver);

    let result = machine.driver().execute(Action::ReadSignal {
        signal: SignalIndex(24),
        wait_for_newest: true,
    })?;

    machine
        .driver()
        .spm_interface_mut()
        .scan_action(ScanAction::Start, ScanDirection::Down)?;

    info!("BadAction result: {result:?}");
    // info!("Executing BadAction recovery sequence");
    // let result = machine.execute_sequence(ActionSequence::BadAction)?;
    // info!("BadAction result: {:?}", result.len());

    // info!("Executing SafeApproach sequence");
    // let result = machine.execute_sequence(ActionSequence::SafeApproach)?;
    // info!("SafeApproach result: {:?}", result.len());

    // info!("Executing EmergencyWithdraw sequence");
    // let result = machine.execute_sequence(ActionSequence::EmergencyWithdraw)?;
    // info!("EmergencyWithdraw result: {:?}", result.len());

    // // Test 2: Execute ActionChain (converted to Custom sequence)
    // info!("Testing ActionChain to Custom sequence conversion...");

    // let chain = ActionChain::new(vec![
    //     Action::ReadBias,
    //     Action::SetBias { voltage: 1.5 },
    //     Action::Wait { duration: std::time::Duration::from_millis(200) },
    //     Action::ReadBias,
    // ]);

    // let result = machine.execute_chain(chain)?;
    // info!("Custom chain result: {:?}", result);

    // // Test 3: Execute with priority and reason
    // info!("Testing execution with priority...");

    // let sequence = ActionSequence::SafeApproach;
    // let result = machine.execute_with_priority(
    //     sequence,
    //     ExecutionPriority::High,
    //     Some("Critical measurement setup".to_string())
    // )?;
    // info!("Priority execution result: {:?}", result.len());

    // // Test 4: Direct driver access when needed
    // info!("Testing direct driver access...");

    // let position = machine.driver().client_mut().folme_xy_pos_get(true)?;
    // info!("Current position via direct access: {:?}", position);

    // let bias_result = machine.driver().execute(Action::ReadBias)?;
    // info!("Direct bias reading: {:?}", bias_result);

    // // Test 5: Mix of approaches
    // info!("Testing mixed usage patterns...");

    // // Built-in sequence
    // machine.execute_sequence(ActionSequence::EmergencyWithdraw)?;

    // // Custom chain
    // let adjustment_chain = ActionChain::new(vec![
    //     Action::MovePiezoRelative {
    //         delta: Position::new(1e-9, 1e-9)
    //     },
    //     Action::Wait { duration: std::time::Duration::from_millis(100) },
    // ]);
    // machine.execute_chain(adjustment_chain)?;

    // // Direct action
    // machine.driver().execute(Action::SetBias { voltage: 2.0 })?;

    // info!("=== Demo completed successfully ===");
    Ok(())
}
