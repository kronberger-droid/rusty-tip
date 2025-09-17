use pico_args::Arguments;
use rusty_tip::NanonisClient;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = Arguments::from_env();

    let input: u32 = args.value_from_str("-i").unwrap_or_else(|_| {
        eprintln!("Usage: cargo run --example get_signals -- -i <index>");
        std::process::exit(1);
    });

    println!("input: {input}");

    if !(0..127).contains(&input) {
        eprintln!("Index must be in the range of 0 to 127")
    }

    let mut client = NanonisClient::new("127.0.0.1", 6501)?;

    client.signal_names_get(true)?;

    let values = client.signals_vals_get((0..=127).collect::<Vec<i32>>(), true)?;

    let names = client.signal_names_get(false)?;

    for (index, (value, name)) in values.iter().zip(names).enumerate() {
        println!("{index:3}: {name:25} - {value:>15?}");
    }

    Ok(())
}
