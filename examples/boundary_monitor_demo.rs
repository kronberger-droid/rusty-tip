use nanonis_rust::{Controller, NanonisClient, RuleBasedPolicy, BoundaryClassifier};
use std::error::Error;
use std::time::Duration;

/// Boundary monitoring demo with separated classifier and policy
fn main() -> Result<(), Box<dyn Error>> {
    println!("Boundary Monitor Demo - Separated Architecture");

    // Connect to Nanonis
    let client = NanonisClient::new("127.0.0.1:6501")?;

    // Create boundary classifier for bias signal (index 24)
    let classifier = BoundaryClassifier::new(
        "Bias Boundary Classifier".to_string(),
        24,  // Signal index
        0.0, // min bound (V)
        2.0, // max bound (V)
    )
    .with_buffer_config(10, 2) // 10 samples, drop first 2
    .with_stability_config(3); // 3 consecutive good for stable

    // Create simple rule-based policy
    let policy = RuleBasedPolicy::new("Simple Rule Policy".to_string());

    // Run monitoring with separated architecture
    let mut controller = Controller::with_client(
        client, 
        Box::new(classifier), 
        Box::new(policy)
    );
    controller.run_control_loop(2.0, Duration::from_secs(30))?;

    Ok(())
}
