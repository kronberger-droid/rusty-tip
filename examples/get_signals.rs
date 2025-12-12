use rusty_tip::NanonisClient;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;

    let values = client.signals_vals_get((0..=127).collect::<Vec<i32>>(), true)?;
    let names = client.signal_names_get()?;

    let _better_names = names.iter().map(|name| {
        if let Some(pos) = name.find('(') {
            name[..pos].trim().to_string()
        } else {
            name.trim().to_string()
        }
    });

    let measure_signals = client.signals_meas_names_get()?;

    for (index, name) in measure_signals.iter().enumerate() {
        println!("{index:3}: {name:25}");
    }

    for (index, (value, name)) in values.iter().zip(names).enumerate() {
        println!("{index:3}: {name:25} - {value:>15?}");
    }

    Ok(())
}
