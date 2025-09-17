use rusty_tip::{
    stability,
    types::{DataToGet, TriggerConfig},
    ActionDriver, NanonisClient,
};
use std::{error::Error, time::Duration};
use textplots::{Chart, Plot};

fn main() -> Result<(), Box<dyn Error>> {
    let mut driver = ActionDriver::new("127.0.0.1", 6501)?;

    let signal = rusty_tip::SignalIndex(0);

    // driver.execute(rusty_tip::Action::Withdraw {
    //     wait_until_finished: true,
    //     timeout: Duration::from_secs(5),
    // })?;

    let mut client = NanonisClient::new("127.0.0.1", 6502)?;

    client.osci1t_trig_set(1, 0, 49.0e-12, 0.0)?;

    let (_, _, _, data) = client.osci1t_data_get(0)?;

    println!("{data:?}");

    // if let Some(osci_data) = driver.read_oscilloscope(
    //     signal,
    //     // Some(TriggerConfig::new(
    //     //     rusty_tip::types::OsciTriggerMode::Level,
    //     //     rusty_tip::TriggerSlope::Falling,
    //     //     49.0e-12,
    //     //     0.0,
    //     // )),
    //     None,
    //     rusty_tip::types::DataToGet::Current,
    // )? {
    //     let frame: Vec<(f32, f32)> = osci_data
    //         .data
    //         .iter()
    //         .enumerate()
    //         .map(|(i, &value)| (i as f32 * osci_data.dt as f32, value as f32))
    //         .collect();

    //     let max_time = (osci_data.size - 1) as f64 * osci_data.dt;

    //     Chart::new(140, 60, 0.0, max_time as f32)
    //         .lineplot(&textplots::Shape::Lines(&frame))
    //         .nice();

    //     let is_stable = stability::trend_analysis_stability(&osci_data.data);

    //     println!("{is_stable}");
    // };

    Ok(())
}
