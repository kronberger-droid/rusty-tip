use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use rusty_tip::{init_terminal, restore_terminal, SimpleTui};

/// Simple test to verify TUI works with mock data
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create channel for passing frequency shift data to TUI
    let (tx, rx) = mpsc::channel::<f32>();

    // Initialize terminal for TUI
    let mut terminal = init_terminal()?;

    // Create TUI instance
    let tui = SimpleTui::new(rx);

    // Spawn mock data generator thread
    let data_thread = thread::spawn(move || {
        let start_time = Instant::now();
        let mut cycle = 0;

        while start_time.elapsed() < Duration::from_secs(30) {
            // Generate mock frequency shift data
            let time_factor = start_time.elapsed().as_secs_f32();
            let freq_shift = -1.0 + 0.5 * (time_factor * 0.5).sin() + 0.2 * (time_factor * 2.0).cos();

            // Send data to TUI
            if tx.send(freq_shift).is_err() {
                break; // TUI closed
            }

            cycle += 1;
            println!("Cycle {}: freq_shift = {:.3}", cycle, freq_shift);

            thread::sleep(Duration::from_millis(200)); // 5 Hz update rate
        }
    });

    // Run TUI in main thread
    let tui_result = tui.run(&mut terminal);

    // Wait for data thread to finish
    let _ = data_thread.join();

    // Restore terminal
    restore_terminal(&mut terminal)?;

    match tui_result {
        Ok(()) => println!("TUI completed successfully"),
        Err(e) => println!("TUI error: {}", e),
    }

    Ok(())
}