use rusty_tip::action_driver;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let mut driver = action_driver::ActionDriver::new("172.0.0.1", 6501)?;

    driver.set_tcp_logger_channels(vec![76])?;

    Ok(())
}
