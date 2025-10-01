use rusty_tip::{action_driver::ActionDriver, TCPLoggerStream};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Create action driver (will attempt to connect to local Nanonis instance)
    let mut driver = ActionDriver::new("127.0.0.1", 6501)?;

    println!("{}", driver.client_mut().pll_amp_ctrl_setpnt_get(1)?);

    Ok(())
}
