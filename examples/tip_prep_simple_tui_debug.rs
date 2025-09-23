use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use rusty_tip::{init_terminal, restore_terminal, SimpleTui};

/// Debug version that sends fake data every few seconds to test data flow
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create channel for passing frequency shift data to TUI
    let (tx, rx) = mpsc::channel::<f32>();

    // Initialize terminal for TUI
    let mut terminal = init_terminal()?;

    // Create TUI instance
    let tui = SimpleTui::new(rx);

    // Spawn data generator thread - simulating slow controller updates
    let data_thread = thread::spawn(move || {
        let mut cycle = 0;
        
        // Wait a bit before starting to send data
        thread::sleep(Duration::from_secs(2));
        
        // Send data every 3 seconds to simulate controller behavior
        for i in 0..20 {
            let freq_shift = -1.5 + (i as f32 * 0.1);
            
            if tx.send(freq_shift).is_err() {
                break; // TUI closed
            }
            
            cycle += 1;
            
            // Write to log file instead of stdout to not interfere with TUI
            log::info!("Cycle {}: freq_shift = {:.3}", cycle, freq_shift);
            
            thread::sleep(Duration::from_secs(3)); // Slow updates like real controller
        }
        
        log::info!("Data generator finished");
    });

    // Run TUI in main thread
    let tui_result = tui.run(&mut terminal);

    // Wait for data thread to finish
    let _ = data_thread.join();

    // Restore terminal
    restore_terminal(&mut terminal)?;

    match tui_result {
        Ok(()) => eprintln!("TUI completed successfully"),
        Err(e) => eprintln!("TUI error: {}", e),
    }

    Ok(())
}