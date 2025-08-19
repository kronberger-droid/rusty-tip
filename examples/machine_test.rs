use std::error::Error;

use rusty_tip::{
    BiasVoltage, NanonisClient, ScanFrame, Position, MotorDirection, MotorGroup, StepCount,
    Frequency, Amplitude, MotorAxis, SignalIndex
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut client = NanonisClient::builder()
        .address("127.0.0.1")
        .port(6501)
        .debug(true)
        .build()?;

    // Demonstrate type-safe bias voltage setting
    println!("Setting bias to 1.5V using type-safe wrapper...");
    client.set_bias(BiasVoltage(1.5))?;

    // Read current bias
    let current_bias = client.get_bias()?;
    println!("Current bias voltage: {:.3}V", current_bias.0);

    // Demonstrate type-safe signal operations
    println!("Finding signal index for 'bias'...");
    if let Some(bias_index) = client.find_signal_index("bias")? {
        println!("Found bias signal at index: {}", bias_index.0);
        
        // Get signal calibration using type-safe index
        let (calibration, offset) = client.signals_calibr_get(bias_index)?;
        println!("Signal calibration: {}, offset: {}", calibration, offset);
        
        // Get signal range using type-safe index
        let (max_range, min_range) = client.signals_range_get(bias_index)?;
        println!("Signal range: {} to {}", min_range, max_range);
    }

    // Demonstrate type-safe motor operations
    println!("Testing motor frequency/amplitude with type-safe wrappers...");
    let (freq, amp) = client.motor_freq_amp_get(MotorAxis::X)?;
    println!("Motor X-axis - Frequency: {:.2} Hz, Amplitude: {:.2} V", freq.0, amp.0);

    // Set new frequency and amplitude using type-safe wrappers
    client.motor_freq_amp_set(
        Frequency::hz(1000.0),
        Amplitude::volts(5.0),
        MotorAxis::X
    )?;
    println!("Set motor X-axis to 1000 Hz, 5.0 V");

    // Demonstrate type-safe scan frame operations
    println!("Getting scan frame using type-safe wrapper...");
    let scan_frame = client.scan_frame_get()?;
    println!("Scan frame center: ({:.6}, {:.6}) m", 
             scan_frame.center.x, scan_frame.center.y);
    println!("Scan frame size: {:.6} x {:.6} m, angle: {:.1}°",
             scan_frame.width_m, scan_frame.height_m, scan_frame.angle_deg);

    // Create and set a new scan frame using type-safe constructor
    let new_frame = ScanFrame::new(
        Position::new(0.0, 0.0),  // Center at origin
        1e-6,                     // 1 μm width
        1e-6,                     // 1 μm height
        0.0                       // 0° angle
    );
    client.scan_frame_set(new_frame)?;
    println!("Set new scan frame: 1μm x 1μm at origin");

    // Demonstrate scan buffer configuration
    let (channel_indexes, pixels, lines) = client.scan_buffer_get()?;
    println!("Scan buffer: {} channels, {}x{} pixels", 
             channel_indexes.len(), pixels, lines);

    println!("Type-safe API demonstration completed successfully!");

    Ok(())
}
