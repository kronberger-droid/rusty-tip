use std::error::Error;
use std::time::Duration;

use rusty_tip::{
    NanonisClient, BiasVoltage, MotorDirection, MotorGroup, StepCount, 
    Frequency, Amplitude, MotorAxis, SignalIndex, OscilloscopeIndex,
    SampleCount, TriggerMode, TriggerLevel, TimeoutMs
};

fn main() -> Result<(), Box<dyn Error>> {
    // This example demonstrates the ease of use with automatic type conversions
    println!("Type Conversion Demo - Maximum Ease of Use");
    println!("==========================================");

    let mut client = NanonisClient::builder()
        .address("127.0.0.1")
        .port(6501)
        .debug(false)
        .build()?;

    // 1. Raw integers can be used directly (converted automatically)
    println!("1. Using raw integers (automatically converted):");
    
    // These calls use raw integers that get automatically converted
    client.osci_hr_ch_set(0, 24)?;  // osci_index=0, signal_index=24
    client.osci_hr_samples_set(1000)?;  // samples=1000
    
    println!("   ✓ osci_hr_ch_set(0, 24) - raw integers work!");
    println!("   ✓ osci_hr_samples_set(1000) - usize/i32 conversion!");

    // 2. Type-safe wrappers can be used explicitly
    println!("2. Using explicit type-safe wrappers:");
    
    client.osci_hr_ch_set(OscilloscopeIndex(1), SignalIndex(25))?;
    client.osci_hr_samples_set(SampleCount::new(2000))?;
    
    println!("   ✓ Explicit OscilloscopeIndex and SignalIndex work!");

    // 3. Motor operations with automatic conversions
    println!("3. Motor operations with mixed types:");
    
    // Raw values automatically converted to enums/wrappers
    client.motor_freq_amp_set(1000.0, 5.0, 1)?;  // freq, amp, axis (X=1)
    
    // Explicit type-safe wrappers
    client.motor_freq_amp_set(
        Frequency::hz(1500.0),
        Amplitude::volts(7.5),
        MotorAxis::Y
    )?;
    
    println!("   ✓ motor_freq_amp_set(1000.0, 5.0, 1) - raw values!");
    println!("   ✓ motor_freq_amp_set(Frequency, Amplitude, MotorAxis) - wrappers!");

    // 4. Trigger operations with automatic conversions
    println!("4. Trigger operations:");
    
    // Raw integer converted to TriggerMode enum
    client.osci_hr_trig_mode_set(0)?;  // 0 = Immediate
    
    // Explicit enum
    client.osci_hr_trig_mode_set(TriggerMode::Level)?;
    
    println!("   ✓ osci_hr_trig_mode_set(0) - raw integer to enum!");
    println!("   ✓ osci_hr_trig_mode_set(TriggerMode::Level) - explicit enum!");

    // 5. Duration and timeout conversions
    println!("5. Timeout and duration conversions:");
    
    let timeout_duration = Duration::from_secs(5);
    let timeout_ms = TimeoutMs::from(timeout_duration);
    
    // Both raw milliseconds and Duration work
    let _result1 = client.scan_wait_end_of_scan(5000.into())?;  // raw ms
    let _result2 = client.scan_wait_end_of_scan(timeout_ms)?;   // from Duration
    
    println!("   ✓ scan_wait_end_of_scan(5000) - raw milliseconds!");
    println!("   ✓ scan_wait_end_of_scan(Duration) - from std::time::Duration!");

    // 6. Mixed usage demonstration
    println!("6. Mixed usage in a realistic scenario:");
    
    // Set up oscilloscope with mix of raw values and wrappers
    let osci_idx = 0;           // Raw integer
    let signal_idx = 24;        // Raw integer  
    let samples = 5000;         // Raw integer
    let trigger_level = 1.5;    // Raw float
    
    client.osci_hr_ch_set(osci_idx, signal_idx)?;
    client.osci_hr_samples_set(samples)?;
    client.osci_hr_trig_mode_set(TriggerMode::Level)?;  // Explicit enum
    client.osci_hr_trig_lev_val_set(trigger_level)?;    // Raw float converted
    
    println!("   ✓ Complete oscilloscope setup with mixed types!");

    // 7. Show type safety still works
    println!("7. Type safety demonstration:");
    
    // This would cause a compile error if uncommented:
    // client.osci_hr_trig_mode_set("invalid")?;  // ❌ String not convertible
    // client.osci_hr_samples_set(-1000)?;        // ❌ Would need TryFrom for validation
    
    // But these work fine:
    let mode_val: u16 = 1;
    client.osci_hr_trig_mode_set(mode_val)?; // ✓ Direct conversion from u16
    
    println!("   ✓ Type safety preserved - invalid types rejected at compile time!");

    println!("\nSummary:");
    println!("========");
    println!("• Raw integers/floats automatically convert to wrapper types");
    println!("• Duration converts to TimeoutMs automatically");  
    println!("• Enums provide TryFrom for validated conversions");
    println!("• Type safety prevents invalid values at compile time");
    println!("• Maximum ease of use while maintaining safety!");

    Ok(())
}