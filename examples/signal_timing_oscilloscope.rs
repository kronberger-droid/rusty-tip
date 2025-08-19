use rusty_tip::{NanonisClient, OscilloscopeIndex, SignalIndex, SampleCount, TriggerMode};
use std::error::Error;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    
    println!("Oscilloscope-Based Signal Timing Analysis");
    println!("=========================================");
    
    // Test both oscilloscope types
    test_osci1t_timing(&mut client)?;
    test_osci_hr_timing(&mut client)?;
    
    // Compare with regular signal reads
    compare_with_regular_reads(&mut client)?;
    
    Ok(())
}

fn test_osci1t_timing(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("\n--- Oscilloscope 1-Channel (Osci1T) Timing Test ---");
    
    // Configure oscilloscope for signal 24 (bias voltage)
    let signal_index = 24;
    client.osci1t_ch_set(signal_index)?;
    
    // Get available timebases and select a fast one
    let (current_timebase, timebases) = client.osci1t_timebase_get()?;
    println!("Available timebases: {:?}", timebases);
    println!("Current timebase index: {}", current_timebase);
    
    // Use the fastest timebase (usually index 0)
    let fast_timebase_index = 0;
    client.osci1t_timebase_set(fast_timebase_index)?;
    
    // Configure trigger for immediate acquisition
    client.osci1t_trig_set(
        0,   // trigger_mode: 0 = Immediate
        1,   // trigger_slope: 1 = Rising edge
        0.0, // trigger_level: 0V
        0.1, // trigger_hysteresis: 0.1V
    )?;
    
    // Start oscilloscope
    client.osci1t_run()?;
    
    println!("\nCollecting oscilloscope data...");
    
    let mut hardware_timestamps = Vec::new();
    let mut software_timestamps = Vec::new();
    let mut dt_values = Vec::new();
    let num_acquisitions = 100;
    
    for i in 0..num_acquisitions {
        let sw_start = Instant::now();
        let (t0, dt, size, data) = client.osci1t_data_get(1)?; // Wait for next trigger
        let sw_end = Instant::now();
        
        software_timestamps.push(sw_end.duration_since(sw_start));
        hardware_timestamps.push(t0);
        dt_values.push(dt);
        
        if (i + 1) % 20 == 0 {
            println!("  Completed {}/{} acquisitions", i + 1, num_acquisitions);
        }
        
        // Log sample details for first few
        if i < 5 {
            println!("  Acquisition {}: t0={:.6}s, dt={:.9}s, samples={}, first_value={:.6}", 
                     i, t0, dt, size, data.first().unwrap_or(&0.0));
        }
    }
    
    analyze_osci1t_timing(&hardware_timestamps, &software_timestamps, &dt_values);
    
    Ok(())
}

fn test_osci_hr_timing(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("\n--- Oscilloscope High Resolution (OsciHR) Timing Test ---");
    
    // Configure high-resolution oscilloscope
    let osci_index = 0;
    let signal_index = 24; // Bias voltage
    
    client.osci_hr_ch_set(OscilloscopeIndex(osci_index), SignalIndex(signal_index))?;
    
    // Set number of samples
    let samples = 1000;
    client.osci_hr_samples_set(SampleCount::new(samples))?;
    
    // Configure for immediate trigger mode
    client.osci_hr_trig_mode_set(TriggerMode::Immediate)?;
    client.osci_hr_trig_arm_mode_set(1)?; // 1 = Continuous
    
    // Set oversampling for better precision
    client.osci_hr_oversampl_set(1)?; // Higher oversampling
    
    // Start high-resolution oscilloscope
    client.osci_hr_run()?;
    
    println!("\nCollecting high-resolution oscilloscope data...");
    
    let mut acquisitions = Vec::new();
    let num_acquisitions = 50;
    
    for i in 0..num_acquisitions {
        let sw_start = Instant::now();
        let (timestamp, time_delta, data, timeout) = client.osci_hr_osci_data_get(
            osci_index,
            1, // Wait for next trigger
            5.0, // 5 second timeout
        )?;
        let sw_duration = sw_start.elapsed();
        
        if timeout {
            println!("  Warning: Timeout occurred on acquisition {}", i);
            continue;
        }
        
        // Log details for first few before moving timestamp
        if i < 3 {
            println!("  Acquisition {}: timestamp='{}', dt={:.9}s, samples={}", 
                     i, timestamp, time_delta, data.len());
        }
        
        acquisitions.push((timestamp, time_delta, data.len(), sw_duration));
        
        if (i + 1) % 10 == 0 {
            println!("  Completed {}/{} acquisitions", i + 1, num_acquisitions);
        }
    }
    
    analyze_osci_hr_timing(&acquisitions);
    
    Ok(())
}

fn compare_with_regular_reads(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("\n--- Comparison with Regular Signal Reads ---");
    
    let signal_indices = vec![24]; // Bias voltage
    let num_reads = 100;
    
    // Regular signal reads timing
    println!("Testing regular signal reads...");
    let mut regular_timings = Vec::new();
    
    for _ in 0..num_reads {
        let start = Instant::now();
        client.signals_val_get(signal_indices.clone(), true)?;
        regular_timings.push(start.elapsed());
    }
    
    // Calculate statistics
    let avg_regular = regular_timings.iter().sum::<Duration>() / regular_timings.len() as u32;
    let mut sorted_regular = regular_timings.clone();
    sorted_regular.sort();
    let median_regular = sorted_regular[sorted_regular.len() / 2];
    let min_regular = sorted_regular[0];
    let max_regular = sorted_regular[sorted_regular.len() - 1];
    
    println!("\nRegular Signal Reads Results:");
    println!("  Average time: {:?}", avg_regular);
    println!("  Median time:  {:?}", median_regular);
    println!("  Min time:     {:?}", min_regular);
    println!("  Max time:     {:?}", max_regular);
    
    // Calculate coefficient of variation for regular reads
    let avg_micros = avg_regular.as_micros() as f64;
    let variance: f64 = regular_timings.iter()
        .map(|d| {
            let diff = d.as_micros() as f64 - avg_micros;
            diff * diff
        })
        .sum::<f64>() / regular_timings.len() as f64;
    let std_dev = variance.sqrt();
    let cv_regular = std_dev / avg_micros * 100.0;
    
    println!("  Coefficient of variation: {:.2}%", cv_regular);
    
    println!("\n--- Key Insights ---");
    println!("1. Regular signal reads: Variable timing, depends on TCP network latency");
    println!("2. Oscilloscope 1T: Hardware-timed sampling with precise dt between samples");
    println!("3. Oscilloscope HR: High-resolution, batch acquisition with consistent timing");
    println!("4. For precise timestamps, use oscilloscope functions instead of signal_val_get");
    
    Ok(())
}

fn analyze_osci1t_timing(_hw_timestamps: &[f64], sw_timestamps: &[Duration], dt_values: &[f64]) {
    println!("\nOsci1T Timing Analysis:");
    
    // Analyze hardware timing consistency (dt values)
    if !dt_values.is_empty() {
        let avg_dt = dt_values.iter().sum::<f64>() / dt_values.len() as f64;
        let dt_variance: f64 = dt_values.iter()
            .map(|&dt| (dt - avg_dt).powi(2))
            .sum::<f64>() / dt_values.len() as f64;
        let dt_std_dev = dt_variance.sqrt();
        
        println!("  Hardware timing (dt between samples):");
        println!("    Average dt: {:.9} s ({:.3} Hz)", avg_dt, 1.0 / avg_dt);
        println!("    Standard deviation: {:.9} s", dt_std_dev);
        println!("    Consistency: {:.6}% variation", dt_std_dev / avg_dt * 100.0);
        
        if dt_std_dev / avg_dt < 0.001 {
            println!("    Hardware timing: EXCELLENT (< 0.1% variation)");
        } else if dt_std_dev / avg_dt < 0.01 {
            println!("    Hardware timing: GOOD (< 1% variation)");
        } else {
            println!("    Hardware timing: POOR (> 1% variation)");
        }
    }
    
    // Analyze software acquisition timing
    let avg_sw = sw_timestamps.iter().sum::<Duration>() / sw_timestamps.len() as u32;
    let sw_micros: Vec<f64> = sw_timestamps.iter().map(|d| d.as_micros() as f64).collect();
    let avg_sw_micros = sw_micros.iter().sum::<f64>() / sw_micros.len() as f64;
    let sw_variance: f64 = sw_micros.iter()
        .map(|&t| (t - avg_sw_micros).powi(2))
        .sum::<f64>() / sw_micros.len() as f64;
    let sw_std_dev = sw_variance.sqrt();
    
    println!("  Software acquisition timing:");
    println!("    Average acquisition time: {:?}", avg_sw);
    println!("    Standard deviation: {:.2} μs", sw_std_dev);
    println!("    Coefficient of variation: {:.2}%", sw_std_dev / avg_sw_micros * 100.0);
}

fn analyze_osci_hr_timing(acquisitions: &[(String, f64, usize, Duration)]) {
    println!("\nOsciHR Timing Analysis:");
    
    if acquisitions.is_empty() {
        println!("  No successful acquisitions to analyze");
        return;
    }
    
    // Analyze hardware timing (time_delta values)
    let time_deltas: Vec<f64> = acquisitions.iter().map(|(_, dt, _, _)| *dt).collect();
    let sample_counts: Vec<usize> = acquisitions.iter().map(|(_, _, count, _)| *count).collect();
    let sw_times: Vec<Duration> = acquisitions.iter().map(|(_, _, _, sw)| *sw).collect();
    
    // Hardware timing analysis
    if !time_deltas.is_empty() {
        let avg_dt = time_deltas.iter().sum::<f64>() / time_deltas.len() as f64;
        let dt_variance: f64 = time_deltas.iter()
            .map(|&dt| (dt - avg_dt).powi(2))
            .sum::<f64>() / time_deltas.len() as f64;
        let dt_std_dev = dt_variance.sqrt();
        
        println!("  Hardware timing (time_delta between samples):");
        println!("    Average dt: {:.9} s ({:.1} Hz)", avg_dt, 1.0 / avg_dt);
        println!("    Standard deviation: {:.9} s", dt_std_dev);
        println!("    Consistency: {:.6}% variation", dt_std_dev / avg_dt * 100.0);
        
        // Sample count analysis
        let avg_samples = sample_counts.iter().sum::<usize>() as f64 / sample_counts.len() as f64;
        println!("    Average samples per acquisition: {:.1}", avg_samples);
    }
    
    // Software timing analysis
    let avg_sw = sw_times.iter().sum::<Duration>() / sw_times.len() as u32;
    println!("  Software acquisition timing:");
    println!("    Average acquisition time: {:?}", avg_sw);
    
    // Throughput calculation
    if !time_deltas.is_empty() && !sample_counts.is_empty() {
        let avg_dt = time_deltas.iter().sum::<f64>() / time_deltas.len() as f64;
        let avg_samples = sample_counts.iter().sum::<usize>() as f64 / sample_counts.len() as f64;
        let samples_per_second = avg_samples / avg_sw.as_secs_f64();
        
        println!("  Throughput:");
        println!("    Samples per second: {:.1}", samples_per_second);
        println!("    Effective sample rate: {:.1} Hz", 1.0 / avg_dt);
    }
    
    println!("\n  Advantages of OsciHR:");
    println!("    - Hardware-timed sampling eliminates jitter");
    println!("    - Batch acquisition reduces TCP overhead");
    println!("    - Precise timestamps for each sample");
    println!("    - Configurable oversampling and pre-trigger");
}