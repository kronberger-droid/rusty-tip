use rusty_tip::{stability, Action, ActionDriver};
use std::{error::Error, time::Duration};
use textplots::{Chart, Plot};

fn main() -> Result<(), Box<dyn Error>> {
    let mut driver = ActionDriver::new("127.0.0.1", 6501)?;

    let _freq_shift = rusty_tip::SignalIndex(76);

    let z_pos = rusty_tip::SignalIndex(30);

    driver.execute(rusty_tip::Action::Withdraw {
        wait_until_finished: true,
        timeout: Duration::from_secs(5),
    })?;

    driver.execute(Action::AutoApproach { wait_until_finished: true })?;
    // driver.execute(Action::Wait { duration: Duration::from_millis(500) }])?;

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
        stability::dual_threshold_stability,
    )? {
        let frame: Vec<(f32, f32)> = osci_data
            .data
            .iter()
            .enumerate()
            .map(|(i, &value)| (i as f32 * osci_data.dt as f32, value as f32))
            .collect();

        let max_time = (osci_data.size - 1) as f64 * osci_data.dt;

        Chart::new(140, 60, 0.0, max_time as f32)
            .lineplot(&textplots::Shape::Lines(&frame))
            .nice();

    };

    Ok(())
}
