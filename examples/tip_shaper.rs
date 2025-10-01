use std::{thread::sleep, time::Duration};

use rusty_tip::ActionDriver;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut driver = ActionDriver::new("127.0.0.1", 6501)?;

    for _ in 0..10 {
        driver.execute_chain(vec![
            rusty_tip::Action::AutoApproach {
                wait_until_finished: true,
                timeout: Duration::from_secs(10),
            },
            rusty_tip::Action::PulseRetract {
                pulse_width: Duration::from_millis(500),
                pulse_height_v: 5.0,
            },
            rusty_tip::Action::Withdraw {
                wait_until_finished: true,
                timeout: Duration::from_secs(1),
            },
        ])?;

        sleep(Duration::from_secs(1));
    }
    Ok(())
}
