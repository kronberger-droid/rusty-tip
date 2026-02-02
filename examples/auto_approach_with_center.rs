use rusty_tip::Action;
use std::{error::Error, time::Duration};

fn main() -> Result<(), Box<dyn Error>> {
    let mut driver = rusty_tip::ActionDriver::new("127.0.0.1", 6501)?;

    driver
        .run(Action::AutoApproach {
            wait_until_finished: true,
            timeout: Duration::from_secs(50),
            center_freq_shift: true,
        })
        .go()?;

    Ok(())
}
