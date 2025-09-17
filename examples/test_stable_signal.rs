use rusty_tip::{stability, ActionDriver};
use std::{error::Error, time::Duration};

fn main() -> Result<(), Box<dyn Error>> {
    let mut driver = ActionDriver::new("127.0.0.1", 6501)?;

    let _freq_shift = rusty_tip::SignalIndex(76);

    let z_pos = rusty_tip::SignalIndex(30);

    driver.execute(rusty_tip::Action::Withdraw {
        wait_until_finished: true,
        timeout: Duration::from_secs(5),
    })?;

    driver.execute(rusty_tip::Action::AutoApproach {
        wait_until_finished: true,
        timeout: Duration::from_secs(300),
    })?;

    if let Some(osci_data) = driver.read_oscilloscope_with_stability(
        z_pos,
        // Some(TriggerConfig::new(
        //     rusty_tip::types::OsciTriggerMode::Level,
        //     rusty_tip::TriggerSlope::Falling,
        //     49.0e-12,
        //     0.0,
        // )),
        None,
        rusty_tip::types::DataToGet::Stable {
            readings: 1,
            timeout: Duration::from_secs(2),
        },
        stability::trend_analysis_stability,
    )? {
        // Use the new plotting function - much cleaner!
        rusty_tip::plot_values(&osci_data.data, Some("Z-Position Oscilloscope Data"), None, None)?;

        let is_stable = stability::trend_analysis_stability(&osci_data.data);
        println!("Stability result: {}", is_stable);
    };

    Ok(())
}
