use rusty_tip::NanonisClient;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;

    let channel_slot = 0;
    client.osci1t_ch_set(channel_slot)?;

    // Configure trigger for immediate acquisition (no waiting for events)
    client.osci1t_trig_set(
        0,   // trigger_mode: 0 = Immediate
        1,   // trigger_slope: 1 = Rising edge
        0.0, // trigger_level: 0V
        0.1, // trigger_hysteresis: 0.1V
    )?;

    client.osci1t_run()?;

    for _ in 0..2 {
        let osci_data = client.osci1t_data_get(0)?;
        println!("{osci_data:?}");
    }

    thread::sleep(Duration::from_millis(500));

    Ok(())
}
