use log::info;
use rusty_tip::{Action, ActionChain, ActionDriver, NanonisClient, NanonisValue, Position};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Connect to Nanonis with debug enabled
    let client = NanonisClient::new("127.0.0.1", 6501)?;
    let mut driver = ActionDriver::new(client)?;

    let result = driver.execute_chain(ActionChain::new(vec![
        Action::MovePiezoRelative {
            delta: Position::new(20e-9, 20e-9),
        },
        Action::AutoApproach,
    ]))?;

    info!("{result:?}");

    Ok(())
}
