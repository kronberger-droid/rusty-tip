use nanonis_rust::{MachineState, SessionMetadata};
use std::time::SystemTime;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create sample data
    let signal_names = vec!["Current (A)".to_string(), "Bias (V)".to_string(), "Z (m)".to_string()];
    let active_indices = vec![0, 24, 30]; // Current, Bias, Z
    let signal_values = vec![1.23e-12, 1.5, -1.07e-9];
    
    // OLD FORMAT (what we used to write per sample) - HUGE!
    println!("=== OLD FORMAT (per sample) ===");
    let all_128_names = vec![
        "Current (A)", "Interferometer (m)", "Input 3 (V)", "Input 4 (V)", "Input 5 (V)", 
        "Input 6 (V)", "Input 7 (V)", "Input 8 (V)", "Input 9 (V)", "Input 10 (V)",
        // ... (simulate all 128 signal names)
        "Bias (V)", "Output 2 (V)", "Output 3 (V)", "Output 4 (V)", "X (m)", "Y (m)", "Z (m)",
        // Add enough to simulate 128 total names
    ];
    let mut full_names = all_128_names;
    while full_names.len() < 128 {
        full_names.push("Internal Signal (V)");
    }
    
    let old_format = serde_json::json!({
        "primary_signal": 1.23e-12,
        "all_signals": [1.23e-12, 1.5, -1.07e-9],
        "signal_names": full_names, // 128 names every time!
        "timestamp": 1754765292.045,
        "classification": "Good"
    });
    println!("{}", serde_json::to_string_pretty(&old_format)?);
    println!("Size: {} bytes", serde_json::to_string(&old_format)?.len());
    
    println!("\n=== NEW FORMAT ===");
    
    // Metadata file (written ONCE per session)
    let metadata = SessionMetadata {
        session_id: "2025-08-09T19-10-15".to_string(),
        signal_names: vec![
            "Current (A)".to_string(), "Interferometer (m)".to_string(), 
            // ... (all 128 signal names)
            "Bias (V)".to_string(), // index 24
            // ...
            "Z (m)".to_string(), // index 30
            // ... etc
        ],
        active_indices: vec![0, 24, 30],
        primary_signal_index: 0,
        session_start: SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs_f64(),
    };
    
    println!("Metadata file (written once):");
    println!("{}", serde_json::to_string_pretty(&metadata)?);
    println!("Metadata size: {} bytes", serde_json::to_string(&metadata)?.len());
    
    // Sample data (written per sample) - MINIMAL!
    let sample = MachineState {
        primary_signal: 1.23e-12,
        all_signals: Some(vec![1.23e-12, 1.5, -1.07e-9]), // Only 3 active values!
        timestamp: 1754765292.045,
        classification: nanonis_rust::TipState::Good,
        ..Default::default()
    };
    
    println!("\nSample file (per measurement):");
    println!("{}", serde_json::to_string_pretty(&sample)?);
    println!("Sample size: {} bytes", serde_json::to_string(&sample)?.len());
    
    println!("\n=== SIZE COMPARISON ===");
    let old_size = serde_json::to_string(&old_format)?.len();
    let new_size = serde_json::to_string(&sample)?.len();
    let savings = ((old_size - new_size) as f64 / old_size as f64) * 100.0;
    
    println!("Old format: {} bytes per sample", old_size);
    println!("New format: {} bytes per sample", new_size);
    println!("Space savings: {:.1}%", savings);
    println!("For 10,000 samples: {:.1} KB vs {:.1} KB", 
             (old_size * 10000) as f64 / 1024.0, 
             (new_size * 10000) as f64 / 1024.0);
    
    Ok(())
}