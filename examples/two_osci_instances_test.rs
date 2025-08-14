use rusty_tip::NanonisClient;
use std::error::Error;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    
    println!("Two OsciHR Instances Test");
    println!("========================");
    println!("Testing multiple oscilloscope instances on single TCP connection");
    println!();
    
    // Signal configuration
    let signal_a_index = 0;   // Current signal
    let signal_b_index = 24;  // Bias voltage signal
    
    println!("Configuration:");
    println!("  OsciHR Instance 0: Signal {} (Current)", signal_a_index);
    println!("  OsciHR Instance 1: Signal {} (Bias)", signal_b_index);
    println!();
    
    // Configure both OsciHR instances
    setup_osci_instances(&mut client, signal_a_index, signal_b_index)?;
    
    // Test 1: Basic functionality - can both instances run?
    test_basic_functionality(&mut client)?;
    
    // Test 2: Timing synchronization analysis
    test_timing_synchronization(&mut client)?;
    
    // Test 3: Continuous acquisition performance
    test_continuous_performance(&mut client)?;
    
    Ok(())
}

fn setup_osci_instances(
    client: &mut NanonisClient, 
    signal_a: i32, 
    signal_b: i32
) -> Result<(), Box<dyn Error>> {
    println!("Setting up OsciHR instances...");
    
    // Configure instance 0 with proper settings for data acquisition
    client.osci_hr_ch_set(0, signal_a)?;
    client.osci_hr_samples_set(256)?; // Match working Osci1T example
    client.osci_hr_oversampl_set(1)?; // Some oversampling for better precision
    client.osci_hr_trig_mode_set(0)?; // Immediate mode
    client.osci_hr_trig_arm_mode_set(1)?; // Continuous
    client.osci_hr_calibr_mode_set(0, 1)?; // Calibrated values
    client.osci_hr_pre_trig_set(0, 0.0)?; // No pre-trigger samples
    
    println!("  Instance 0 configured for signal {}", signal_a);
    
    // Configure instance 1 with identical settings
    client.osci_hr_ch_set(1, signal_b)?;
    client.osci_hr_samples_set(256)?; // Match working Osci1T example
    client.osci_hr_oversampl_set(1)?; // Some oversampling for better precision
    client.osci_hr_trig_mode_set(0)?; // Immediate mode
    client.osci_hr_trig_arm_mode_set(1)?; // Continuous
    client.osci_hr_calibr_mode_set(1, 1)?; // Calibrated values
    client.osci_hr_pre_trig_set(0, 0.0)?; // No pre-trigger samples
    
    println!("  Instance 1 configured for signal {}", signal_b);
    
    // Start both instances
    client.osci_hr_run()?;
    println!("  Both instances started");
    
    // Allow more time for initialization and first acquisition
    std::thread::sleep(Duration::from_millis(1000));
    
    Ok(())
}

fn test_basic_functionality(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("\n--- Test 1: Basic Functionality ---");
    
    // Try to collect data from both instances
    println!("Testing data collection from both instances...");
    
    // Instance 0
    let start_0 = Instant::now();
    let result_0 = client.osci_hr_osci_data_get(0, 1, 5.0);
    let duration_0 = start_0.elapsed();
    
    match result_0 {
        Ok((timestamp_0, dt_0, data_0, timeout_0)) => {
            println!("  Instance 0: SUCCESS");
            println!("    Timestamp: {}", timestamp_0);
            println!("    dt: {:.6} ms", dt_0 * 1000.0);
            println!("    Samples: {}", data_0.len());
            println!("    Timeout: {}", timeout_0);
            println!("    Collection time: {:?}", duration_0);
        }
        Err(e) => {
            println!("  Instance 0: FAILED - {}", e);
            return Err(e.into());
        }
    }
    
    // Instance 1
    let start_1 = Instant::now();
    let result_1 = client.osci_hr_osci_data_get(1, 1, 5.0);
    let duration_1 = start_1.elapsed();
    
    match result_1 {
        Ok((timestamp_1, dt_1, data_1, timeout_1)) => {
            println!("  Instance 1: SUCCESS");
            println!("    Timestamp: {}", timestamp_1);
            println!("    dt: {:.6} ms", dt_1 * 1000.0);
            println!("    Samples: {}", data_1.len());
            println!("    Timeout: {}", timeout_1);
            println!("    Collection time: {:?}", duration_1);
        }
        Err(e) => {
            println!("  Instance 1: FAILED - {}", e);
            return Err(e.into());
        }
    }
    
    println!("  ✓ Both instances working independently");
    
    Ok(())
}

fn test_timing_synchronization(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("\n--- Test 2: Timing Synchronization Analysis ---");
    
    let num_tests = 50;
    let mut timing_data = Vec::new();
    
    println!("Collecting {} synchronized samples...", num_tests);
    
    for i in 0..num_tests {
        let acquisition_start = Instant::now();
        
        // Collect from instance 0
        let instance_0_start = Instant::now();
        let (ts0, dt0, data0, timeout0) = client.osci_hr_osci_data_get(0, 1, 2.0)?;
        let instance_0_duration = instance_0_start.elapsed();
        
        // Collect from instance 1 immediately after
        let instance_1_start = Instant::now();
        let (ts1, dt1, data1, timeout1) = client.osci_hr_osci_data_get(1, 1, 2.0)?;
        let instance_1_duration = instance_1_start.elapsed();
        
        let total_acquisition_time = acquisition_start.elapsed();
        let inter_instance_gap = instance_1_start - instance_0_start;
        
        if timeout0 || timeout1 {
            println!("  Warning: Timeout on iteration {}", i);
            continue;
        }
        
        // Log details for first few iterations
        if i < 3 {
            println!("  Iteration {}: Instance 0: {} samples, dt={:.6}ms", i, data0.len(), dt0 * 1000.0);
            println!("  Iteration {}: Instance 1: {} samples, dt={:.6}ms", i, data1.len(), dt1 * 1000.0);
        }
        
        // Parse timestamps for comparison
        let hw_ts0 = parse_timestamp(&ts0).unwrap_or(0.0);
        let hw_ts1 = parse_timestamp(&ts1).unwrap_or(0.0);
        
        timing_data.push(TimingMeasurement {
            iteration: i,
            hw_timestamp_0: hw_ts0,
            hw_timestamp_1: hw_ts1,
            hw_dt_0: dt0,
            hw_dt_1: dt1,
            sw_duration_0: instance_0_duration,
            sw_duration_1: instance_1_duration,
            inter_instance_gap,
            total_acquisition_time,
            samples_0: data0.len(),
            samples_1: data1.len(),
        });
        
        if (i + 1) % 10 == 0 {
            println!("  Completed {}/{} measurements", i + 1, num_tests);
        }
    }
    
    analyze_timing_data(&timing_data);
    
    Ok(())
}

fn test_continuous_performance(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("\n--- Test 3: Continuous Performance Test ---");
    
    let test_duration = Duration::from_secs(10);
    let start_time = Instant::now();
    let mut cycle_count = 0;
    let mut total_samples = 0;
    
    println!("Running continuous acquisition for {:?}...", test_duration);
    
    while start_time.elapsed() < test_duration {
        let cycle_start = Instant::now();
        
        // Rapid sequential collection
        let (ts0, dt0, data0, timeout0) = client.osci_hr_osci_data_get(0, 1, 1.0)?;
        let (ts1, dt1, data1, timeout1) = client.osci_hr_osci_data_get(1, 1, 1.0)?;
        
        if !timeout0 && !timeout1 {
            total_samples += data0.len() + data1.len();
            cycle_count += 1;
            
            // Log sample details occasionally
            if cycle_count % 100 == 0 {
                println!("  Cycle {}: Instance 0: {} samples, dt={:.3}ms | Instance 1: {} samples, dt={:.3}ms", 
                    cycle_count, data0.len(), dt0 * 1000.0, data1.len(), dt1 * 1000.0);
            }
        } else {
            if timeout0 { println!("  Timeout on instance 0, cycle {}", cycle_count); }
            if timeout1 { println!("  Timeout on instance 1, cycle {}", cycle_count); }
        }
        
        let cycle_duration = cycle_start.elapsed();
        
        if cycle_count % 20 == 0 {
            println!("  Cycle {}: {:?} per dual acquisition", cycle_count, cycle_duration);
        }
    }
    
    let actual_duration = start_time.elapsed();
    let acquisition_rate = cycle_count as f64 / actual_duration.as_secs_f64();
    let sample_rate = total_samples as f64 / actual_duration.as_secs_f64();
    
    println!("\nContinuous Performance Results:");
    println!("  Total cycles: {}", cycle_count);
    println!("  Total samples: {}", total_samples);
    println!("  Actual duration: {:?}", actual_duration);
    println!("  Acquisition rate: {:.2} dual-acquisitions/sec", acquisition_rate);
    println!("  Combined sample rate: {:.1} samples/sec", sample_rate);
    println!("  Effective rate per signal: {:.1} samples/sec", sample_rate / 2.0);
    
    if sample_rate / 2.0 >= 1000.0 {
        println!("  ✓ Target 1kHz rate ACHIEVED");
    } else {
        println!("  ⚠ Target 1kHz rate not reached");
    }
    
    Ok(())
}

#[derive(Debug)]
struct TimingMeasurement {
    iteration: usize,
    hw_timestamp_0: f64,
    hw_timestamp_1: f64,
    hw_dt_0: f64,
    hw_dt_1: f64,
    sw_duration_0: Duration,
    sw_duration_1: Duration,
    inter_instance_gap: Duration,
    total_acquisition_time: Duration,
    samples_0: usize,
    samples_1: usize,
}

fn analyze_timing_data(data: &[TimingMeasurement]) {
    if data.is_empty() {
        println!("  No timing data to analyze");
        return;
    }
    
    println!("\nTiming Analysis Results:");
    
    // Hardware timing consistency
    let hw_dt_0_avg = data.iter().map(|d| d.hw_dt_0).sum::<f64>() / data.len() as f64;
    let hw_dt_1_avg = data.iter().map(|d| d.hw_dt_1).sum::<f64>() / data.len() as f64;
    
    println!("  Hardware timing consistency:");
    println!("    Instance 0 avg dt: {:.6} ms ({:.1} Hz)", hw_dt_0_avg * 1000.0, 1.0 / hw_dt_0_avg);
    println!("    Instance 1 avg dt: {:.6} ms ({:.1} Hz)", hw_dt_1_avg * 1000.0, 1.0 / hw_dt_1_avg);
    
    // Software timing analysis
    let sw_gap_avg = data.iter().map(|d| d.inter_instance_gap.as_micros() as f64).sum::<f64>() / data.len() as f64;
    let sw_total_avg = data.iter().map(|d| d.total_acquisition_time.as_micros() as f64).sum::<f64>() / data.len() as f64;
    
    let mut sw_gaps: Vec<u128> = data.iter().map(|d| d.inter_instance_gap.as_micros()).collect();
    sw_gaps.sort();
    let sw_gap_median = sw_gaps[sw_gaps.len() / 2];
    let sw_gap_max = sw_gaps[sw_gaps.len() - 1];
    
    println!("  Software timing synchronization:");
    println!("    Average gap between instances: {:.1} μs", sw_gap_avg);
    println!("    Median gap: {} μs", sw_gap_median);
    println!("    Maximum gap: {} μs", sw_gap_max);
    println!("    Total acquisition time: {:.1} μs", sw_total_avg);
    
    // Synchronization quality assessment
    if sw_gap_avg < 100.0 {
        println!("    ✓ Synchronization: EXCELLENT (< 100μs gap)");
    } else if sw_gap_avg < 500.0 {
        println!("    ✓ Synchronization: GOOD (< 500μs gap)");
    } else if sw_gap_avg < 1000.0 {
        println!("    ⚠ Synchronization: MODERATE (< 1ms gap)");
    } else {
        println!("    ✗ Synchronization: POOR (> 1ms gap)");
    }
    
    // Hardware timestamp correlation
    let hw_ts_diffs: Vec<f64> = data.iter()
        .filter(|d| d.hw_timestamp_0 > 0.0 && d.hw_timestamp_1 > 0.0)
        .map(|d| (d.hw_timestamp_1 - d.hw_timestamp_0).abs())
        .collect();
    
    if !hw_ts_diffs.is_empty() {
        let hw_ts_diff_avg = hw_ts_diffs.iter().sum::<f64>() / hw_ts_diffs.len() as f64;
        println!("  Hardware timestamp correlation:");
        println!("    Average timestamp difference: {:.6} ms", hw_ts_diff_avg * 1000.0);
        
        if hw_ts_diff_avg < 0.001 {
            println!("    ✓ Hardware timestamps: WELL SYNCHRONIZED");
        } else if hw_ts_diff_avg < 0.01 {
            println!("    ⚠ Hardware timestamps: MODERATELY SYNCHRONIZED");
        } else {
            println!("    ✗ Hardware timestamps: POORLY SYNCHRONIZED");
        }
    }
    
    println!("\n--- Summary ---");
    println!("Multi-instance capability: ✓ CONFIRMED");
    println!("Both OsciHR instances can run on single TCP connection");
    println!("Timing gap between instances: {:.1} μs average", sw_gap_avg);
    println!("Hardware timing precision: {:.3} ms per sample", hw_dt_0_avg * 1000.0);
}

fn parse_timestamp(ts_str: &str) -> Option<f64> {
    ts_str.trim().parse::<f64>().ok()
        .or_else(|| {
            // Try to extract numeric part if parse fails
            let numeric: String = ts_str.chars()
                .filter(|c| c.is_numeric() || *c == '.' || *c == '-')
                .collect();
            numeric.parse::<f64>().ok()
        })
}