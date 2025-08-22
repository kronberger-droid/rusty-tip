use rusty_tip::NanonisClient;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;

    client.auto_approach_and_wait()?;

    client.bias_sweep_props_set(number_of_steps, period_ms, autosave, save_dialog_box)

    Ok(())
}
