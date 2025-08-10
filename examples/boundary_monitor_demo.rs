use nanonis_rust::{BoundaryClassifier, Controller, NanonisClient, RuleBasedPolicy};
use std::error::Error;
use std::time::Duration;

/// Boundary monitoring demo with separated classifier and policy
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging with configurable level
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("Boundary Monitor Demo - Separated Architecture");

    // Connect to Nanonis
    let client = NanonisClient::new("127.0.0.1", 6501)?;

    // Create boundary classifier for bias signal (index 24)
    let classifier = BoundaryClassifier::new(
        String::from("Boundary Classifier"),
        24,  // Signal index
        0.0, // min bound (V)
        2.0, // max bound (V)
    )
    .with_buffer_config(10, 2) // 10 samples, drop first 2
    .with_stability_config(3); // 3 consecutive good for stable

    // Create simple rule-based policy
    let policy = RuleBasedPolicy::new("Simple Rule Policy".to_string());

    // Run monitoring with separated architecture
    let mut controller = Controller::with_client(client, Box::new(classifier), Box::new(policy));
    controller.run_control_loop(2.0, Duration::from_secs(30)).await?;

    Ok(())
}
