use rusty_tip::NanonisClient;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;

    client.signal_names_get(true)?;

    let values = client.signals_val_get((0..=127).collect::<Vec<i32>>(), true)?;

    let names = client.signal_names_get(false)?;

    for (index, (value, name)) in values.iter().zip(names).enumerate() {
        println!("{index:3}: {name:25} - {value:>15?}");
    }

    Ok(())
}
