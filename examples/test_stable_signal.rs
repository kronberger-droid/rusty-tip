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
        // Dynamic scaling based on data range
        let max_time = (osci_data.size - 1) as f64 * osci_data.dt;
        let max_value = osci_data.data.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b.abs()));
        
        // Determine time scale
        let (time_scale, time_unit) = if max_time >= 1.0 {
            (1.0, "s")
        } else if max_time >= 1e-3 {
            (1e3, "ms") 
        } else if max_time >= 1e-6 {
            (1e6, "μs")
        } else if max_time >= 1e-9 {
            (1e9, "ns")
        } else {
            (1e12, "ps")
        };
        
        // Determine value scale  
        let (value_scale, value_unit) = if max_value >= 1.0 {
            (1.0, "")
        } else if max_value >= 1e-3 {
            (1e3, "m")
        } else if max_value >= 1e-6 {
            (1e6, "μ") 
        } else if max_value >= 1e-9 {
            (1e9, "n")
        } else {
            (1e12, "p")
        };
        
        let frame: Vec<(f32, f32)> = osci_data
            .data
            .iter()
            .enumerate()
            .map(|(i, &value)| (
                (i as f32 * osci_data.dt as f32 * time_scale as f32), 
                (value as f32 * value_scale as f32)
            ))
            .collect();

        let scaled_max_time = max_time * time_scale;

        println!("Plotting z-position data (time in {}, value in {}units):", time_unit, value_unit);
        Chart::new(140, 60, 0.0, scaled_max_time as f32)
            .lineplot(&textplots::Shape::Lines(&frame))
            .nice();

    };

    Ok(())
}
