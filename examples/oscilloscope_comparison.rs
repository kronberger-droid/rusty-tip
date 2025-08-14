use rusty_tip::NanonisClient;
use std::error::Error;
use std::time::Duration;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    
    println!("Oscilloscope Types Comparison");
    println!("============================");
    
    // Test signal indices
    let signal_index = 0; // Current signal
    
    println!("Testing different oscilloscope types for signal {}", signal_index);
    println!();
    
    // Test 1: Osci1T (known working)
    test_osci1t(&mut client, signal_index)?;
    
    // Test 2: Osci2T (2-channel)
    test_osci2t(&mut client, signal_index)?;
    
    // Test 3: OsciHR (high resolution)
    test_osci_hr(&mut client, signal_index)?;
    
    Ok(())
}

fn test_osci1t(client: &mut NanonisClient, signal_index: i32) -> Result<(), Box<dyn Error>> {
    println!("--- Testing Osci1T (1-Channel) ---");
    
    // Configure Osci1T
    client.osci1t_ch_set(signal_index)?;
    
    // Get available timebases
    let (current_timebase, timebases) = client.osci1t_timebase_get()?;
    println!("  Available timebases: {:?}", timebases);
    println!("  Current timebase index: {} = {:.6}s", current_timebase, timebases.get(current_timebase as usize).unwrap_or(&0.0));
    
    // Configure trigger
    client.osci1t_trig_set(
        0,   // trigger_mode: 0 = Immediate
        1,   // trigger_slope: 1 = Rising edge
        0.0, // trigger_level: 0V
        0.1, // trigger_hysteresis: 0.1V
    )?;
    
    // Start oscilloscope
    client.osci1t_run()?;
    
    // Allow initialization
    std::thread::sleep(Duration::from_millis(500));
    
    // Collect data
    let (t0, dt, size, data) = client.osci1t_data_get(0)?;
    
    println!("  Results:");
    println!("    t0: {:.6}s", t0);
    println!("    dt: {:.6}s ({:.1} Hz)", dt, 1.0 / dt);
    println!("    Size: {}", size);
    println!("    Data length: {}", data.len());
    println!("    First 5 values: {:?}", &data[0..5.min(data.len())]);
    println!("    Status: {}", if data.len() > 0 { "✓ SUCCESS" } else { "✗ NO DATA" });
    println!();
    
    Ok(())
}

fn test_osci2t(client: &mut NanonisClient, signal_index: i32) -> Result<(), Box<dyn Error>> {
    println!("--- Testing Osci2T (2-Channel) ---");
    
    // Configure Osci2T with same signal on both channels
    client.osci2t_ch_set(signal_index, signal_index)?;
    
    // Get available timebases
    let (current_timebase, timebases) = client.osci2t_timebase_get()?;
    println!("  Available timebases: {:?}", timebases);
    println!("  Current timebase index: {} = {:.6}s", current_timebase, timebases.get(current_timebase as usize).unwrap_or(&0.0));
    
    // Set oversampling
    client.osci2t_oversampl_set(5)?; // 1 sample (no averaging)
    
    // Start oscilloscope
    client.osci2t_run()?;
    
    // Allow initialization
    std::thread::sleep(Duration::from_millis(500));
    
    // Collect data
    let (t0, dt, data_a, data_b) = client.osci2t_data_get(0)?;
    
    println!("  Results:");
    println!("    t0: {:.6}s", t0);
    println!("    dt: {:.6}s ({:.1} Hz)", dt, 1.0 / dt);
    println!("    Channel A length: {}", data_a.len());
    println!("    Channel B length: {}", data_b.len());
    if !data_a.is_empty() {
        println!("    Channel A first 5: {:?}", &data_a[0..5.min(data_a.len())]);
    }
    if !data_b.is_empty() {
        println!("    Channel B first 5: {:?}", &data_b[0..5.min(data_b.len())]);
    }
    println!("    Status: {}", if data_a.len() > 0 { "✓ SUCCESS" } else { "✗ NO DATA" });
    println!();
    
    Ok(())
}

fn test_osci_hr(client: &mut NanonisClient, signal_index: i32) -> Result<(), Box<dyn Error>> {
    println!("--- Testing OsciHR (High Resolution) ---");
    
    let osci_index = 0;
    
    // Try different configurations to see what works
    let configs = [
        ("Basic config", 256, 1, 0, 1),     // samples, oversampl, trig_mode, arm_mode
        ("More samples", 512, 1, 0, 1),
        ("Less oversampling", 256, 0, 0, 1),
        ("Single shot", 256, 1, 0, 0),     // arm_mode = 0 (single shot)
        ("Level trigger", 256, 1, 1, 1),   // trig_mode = 1 (level trigger)
    ];
    
    for (config_name, samples, oversampl, trig_mode, arm_mode) in configs {
        println!("  Testing {}: samples={}, oversampl={}, trig_mode={}, arm_mode={}", 
                 config_name, samples, oversampl, trig_mode, arm_mode);
        
        // Configure OsciHR
        client.osci_hr_ch_set(osci_index, signal_index)?;
        client.osci_hr_samples_set(samples)?;
        client.osci_hr_oversampl_set(oversampl)?;
        client.osci_hr_trig_mode_set(trig_mode)?;
        client.osci_hr_trig_arm_mode_set(arm_mode)?;
        client.osci_hr_calibr_mode_set(osci_index, 1)?; // Calibrated values
        client.osci_hr_pre_trig_set(0, 0.0)?; // No pre-trigger
        
        // If level trigger, set trigger parameters
        if trig_mode == 1 {
            client.osci_hr_trig_lev_ch_set(signal_index)?; // Trigger on same signal
            client.osci_hr_trig_lev_val_set(0.0)?; // Trigger level 0V
            client.osci_hr_trig_lev_hyst_set(0.1)?; // Hysteresis 0.1V
            client.osci_hr_trig_lev_slope_set(0)?; // Rising edge
        }
        
        // Start oscilloscope
        client.osci_hr_run()?;
        
        // Wait for initialization
        std::thread::sleep(Duration::from_millis(1000));
        
        // Try to collect data with different modes
        for data_mode in [0, 1] { // 0 = Current, 1 = Next trigger
            let mode_name = if data_mode == 0 { "Current" } else { "Next trigger" };
            
            match client.osci_hr_osci_data_get(osci_index, data_mode, 2.0) {
                Ok((timestamp, time_delta, data, timeout)) => {
                    println!("    {} - {}: {} samples, dt={:.6}s, timeout={}", 
                             config_name, mode_name, data.len(), time_delta, timeout);
                    
                    if data.len() > 0 {
                        println!("      First 3 values: {:?}", &data[0..3.min(data.len())]);
                        println!("      ✓ SUCCESS with {}", config_name);
                        return Ok(()); // Found working configuration
                    }
                }
                Err(e) => {
                    println!("    {} - {}: ERROR - {}", config_name, mode_name, e);
                }
            }
        }
        
        println!("    {} - No data collected", config_name);
        println!();
    }
    
    println!("  ✗ OsciHR: No working configuration found");
    println!();
    
    Ok(())
}