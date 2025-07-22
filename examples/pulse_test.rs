use nanonis_rust::{NanonisClient, NanonisValue};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut client = NanonisClient::new("127.0.0.1:6501")?;
    client.set_debug(true);

    // Set speed
    client.quick_send(
        "FolMe.SpeedSet",
        &[NanonisValue::F32(3e-9), NanonisValue::U32(1)],
        &["f", "I"],
        &[],
    )?;

    println!("client does work!");
    // Generate random position (equivalent to Python's random.uniform)
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let x_pos = rng.gen_range(0.0..10e-9);
    let y_pos = rng.gen_range(0.0..10e-9);

    println!("Moving to x: {x_pos} y: {y_pos}");

    // Set position
    client.folme_xy_pos_set(x_pos, y_pos, 0)?;

    // Pulse
    client.quick_send(
        "Bias.Pulse",
        &[
            NanonisValue::F32(1.0),
            NanonisValue::F32(1.0),
            NanonisValue::F32(4.0),
            NanonisValue::F32(0.0),
            NanonisValue::F32(0.0),
        ],
        &["f", "f", "f", "f", "f"],
        &[],
    )?;

    // Get signals
    let signals = client.signals_names_get()?;
    println!("Available signals: {signals:?}");

    Ok(())
}
