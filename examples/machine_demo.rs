use rusty_tip::{ActionDriver, MachineRepresentation};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Create client and driver
    let driver = ActionDriver::new("127.0.0.1", 6501)?;

    // let mut machine = MachineRepresentation::new(driver);

    // let client = machine.driver().spm_interface_mut();
    //

    let client = driver.spm_interface_mut();

    client.z_ctrl_withdraw(true, timeout_ms)
    client.motor_start_move(rusty_tip::MotorDirection::XPlus, steps, group, wait)

    client.osci1t_ch_set(24)?;

    Ok(())
}
