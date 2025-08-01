use nanonis_rust::{AFMController, NanonisClient, RuleBasedPolicy};
use std::error::Error;
use std::time::Duration;

/// Boundary monitoring demo with Good/Bad/Stable policy decisions
fn main() -> Result<(), Box<dyn Error>> {
    println!("Boundary Monitor Demo - Stability Tracking");

    // Connect and find bias signal
    let mut client = NanonisClient::new("127.0.0.1:6501")?;

    // Create policy: 3 consecutive good decisions → stable
    let policy = RuleBasedPolicy::new(
        "Bias Monitor".to_string(),
        24,
        0.0, // min bound (V)
        2.0, // max bound (V)
    )
    .with_buffer_config(10, 2) // 10 samples, drop first 2
    .with_stability_config(3); // 3 consecutive good for stable

    // Run monitoring with the existing working client
    let mut controller = AFMController::with_client(client, Box::new(policy));
    controller.run_control_loop(24, 2.0, Duration::from_secs(30))?;

    Ok(())
}
