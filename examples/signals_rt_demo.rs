use rusty_tip::NanonisClient;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to Nanonis
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;

    println!("=== Signals RT Management Demo ===");

    // Get current RT signal assignments
    let (available_signals, current_23, current_24) = client.signals_add_rt_get()?;

    println!("\nAvailable additional RT signals ({} total):", available_signals.len());
    for (i, signal) in available_signals.iter().enumerate() {
        println!("  {}: {}", i, signal);
    }

    println!("\nCurrent assignments:");
    println!("  Internal 23: {}", current_23);
    println!("  Internal 24: {}", current_24);

    // Example: Assign first two available RT signals to Internal 23 and 24
    if available_signals.len() >= 2 {
        println!("\nAssigning RT signals:");
        println!("  Setting Internal 23 to: {}", available_signals[0]);
        println!("  Setting Internal 24 to: {}", available_signals[1]);

        // Assign RT signal indices 0 and 1
        client.signals_add_rt_set(0, 1)?;

        // Verify the assignment
        let (_, new_23, new_24) = client.signals_add_rt_get()?;
        println!("\nVerification - New assignments:");
        println!("  Internal 23: {}", new_23);
        println!("  Internal 24: {}", new_24);

        // Restore original assignments
        println!("\nRestoring original assignments...");
        // Note: This would require finding indices of the original signals
        // For demonstration, we'll show how to do it conceptually
        println!("  (In real usage, you'd find indices for '{}' and '{}')", current_23, current_24);
    } else {
        println!("\nNot enough RT signals available for assignment demo");
    }

    println!("\n=== Signals RT Demo Complete ===");
    Ok(())
}