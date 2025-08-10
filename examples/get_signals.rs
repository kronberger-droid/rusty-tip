use nanonis_rust::NanonisClient;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;

    client.signal_names_get(true)?;

    let values = client.signals_val_get((0..=127).collect::<Vec<i32>>(), true)?;

    let range = -1e-5..1e-5;

    let non_zero_values: Vec<f32> = values
        .iter()
        .filter(|&v| !range.contains(v))
        .copied()
        .collect();

    // Get indices of non-zero signals for easier identification
    let non_zero_indices: Vec<usize> = values
        .iter()
        .enumerate()
        .filter(|(_, &v)| !range.contains(&v))
        .map(|(i, _)| i)
        .collect();

    println!(
        "Found {} non-zero values out of {} total signals",
        non_zero_values.len(),
        values.len()
    );
    println!("Non-zero signal indices: {:?}", non_zero_indices);
    println!("Non-zero values: {:?}", non_zero_values);

    Ok(())
}
