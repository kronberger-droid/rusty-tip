use std::time::Duration;

use rusty_tip::{ActionDriver, MotorGroup};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Create client and driver
    let driver = ActionDriver::new("127.0.0.1", 6501)?;

    let client = driver().spm_interface_mut();

    client.z_ctrl_withdraw(true, Duration::from_secs(5));

    client.motor_start_move(MotorDirection::XPlus, 2, MotorGroup::Group1, true);

    client.motor_start_move(MotorDirection::YPlus, 2, MotorGroup::Group1, true);

    client.osci1t_ch_set(24)?;

    Ok(())
}
