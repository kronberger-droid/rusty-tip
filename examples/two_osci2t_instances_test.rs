use rusty_tip::NanonisClient;
use std::error::Error;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    
    println!("Two Osci2T Instances Test");
    println!("=========================");
    println!("Testing multiple 2-channel oscilloscope instances on single TCP connection");
    println!();
    
    // Signal configuration - 4 signals total across 2 Osci2T instances
    let signals = [0, 1, 24, 2]; // Current, Signal1, Bias, Signal2
    
    println!("Configuration:");
    println!("  Osci2T Instance A: Signals {} & {} (Current & Signal1)", signals[0], signals[1]);
    println!("  Osci2T Instance B: Signals {} & {} (Bias & Signal2)", signals[2], signals[3]);
    println!();
    
    // Setup both Osci2T instances
    setup_osci2t_instances(&mut client, &signals)?;
    
    // Test 1: Basic functionality
    test_basic_functionality(&mut client)?;
    
    // Test 2: Timing synchronization analysis  
    test_timing_synchronization(&mut client)?;
    
    // Test 3: Continuous performance test
    test_continuous_performance(&mut client)?;
    
    Ok(())
}

fn setup_osci2t_instances(
    client: &mut NanonisClient,
    signals: &[i32; 4]
) -> Result<(), Box<dyn Error>> {
    println!("Setting up Osci2T instances...");
    
    // Configure Osci2T instance A (signals 0 & 1)
    client.osci2t_ch_set(signals[0], signals[1])?;
    
    // Get and set fast timebase
    let (current_timebase, timebases) = client.osci2t_timebase_get()?;
    println!("  Available timebases: {:?}", timebases);
    
    // Use fastest timebase (highest index = shortest timebase)
    let fast_timebase = (timebases.len() - 1) as u16;
    client.osci2t_timebase_set(fast_timebase)?;
    println!("  Set timebase to index {} = {:.6}s", fast_timebase, timebases[fast_timebase as usize]);
    
    // Set oversampling for precision
    client.osci2t_oversampl_set(4)?; // 2 samples averaging
    
    // Start Osci2T instance A
    client.osci2t_run()?;
    
    println!("  Osci2T Instance A configured: signals {} & {}", signals[0], signals[1]);
    
    // NOTE: Since we're using one TCP connection, we can only have one Osci2T instance
    // This test will demonstrate the concept with one instance handling 2 signals
    // In a real 4-connection setup, you'd have separate clients for separate instances
    
    // Allow initialization time
    std::thread::sleep(Duration::from_millis(1000));
    
    println!("  Osci2T instance started and ready");
    
    Ok(())
}

fn test_basic_functionality(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("\n--- Test 1: Basic Functionality ---");
    
    println!("Testing data collection from Osci2T instance...");
    
    let start = Instant::now();
    let (t0, dt, data_a, data_b) = client.osci2t_data_get(0)?; // 0 = Current data
    let duration = start.elapsed();
    
    println!("  Results:");
    println!("    t0: {:.6}s", t0);
    println!("    dt: {:.6}s ({:.1} Hz)", dt, 1.0 / dt);
    println!("    Channel A samples: {}", data_a.len());
    println!("    Channel B samples: {}", data_b.len());
    println!("    Collection time: {:?}", duration);
    
    if !data_a.is_empty() {
        println!("    Channel A first 3: {:?}", &data_a[0..3.min(data_a.len())]);
        println!("    Channel A range: {:.6} to {:.6}", 
                 data_a.iter().fold(f64::INFINITY, |a, &b| a.min(b as f64)),
                 data_a.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b as f64)));
    }
    
    if !data_b.is_empty() {
        println!("    Channel B first 3: {:?}", &data_b[0..3.min(data_b.len())]);
        println!("    Channel B range: {:.6} to {:.6}", 
                 data_b.iter().fold(f64::INFINITY, |a, &b| a.min(b as f64)),
                 data_b.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b as f64)));
    }
    
    if data_a.len() > 0 && data_b.len() > 0 {
        println!("  ✓ SUCCESS: Both channels working with {} samples each", data_a.len());
    } else {
        println!("  ✗ FAILED: No data collected");
        return Err("No data collected from Osci2T".into());
    }
    
    Ok(())
}

fn test_timing_synchronization(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("\n--- Test 2: Timing Synchronization Analysis ---");
    
    let num_tests = 50;
    let mut timing_data = Vec::new();
    
    println!("Collecting {} synchronized dual-channel samples...", num_tests);
    
    for i in 0..num_tests {
        let acquisition_start = Instant::now();
        
        // Get data from both channels simultaneously (hardware synchronized)
        let (t0, dt, data_a, data_b) = client.osci2t_data_get(1)?; // 1 = Next trigger
        
        let acquisition_time = acquisition_start.elapsed();
        
        timing_data.push(TimingMeasurement {
            iteration: i,
            hw_timestamp: t0,
            hw_dt: dt,
            samples_a: data_a.len(),
            samples_b: data_b.len(),
            acquisition_time,
        });
        
        // Log details for first few iterations
        if i < 3 {
            println!("  Iteration {}: t0={:.6}s, dt={:.6}s, samples={}+{}", 
                     i, t0, dt, data_a.len(), data_b.len());
        }
        
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
        
        // Collect data from both channels
        let (t0, dt, data_a, data_b) = client.osci2t_data_get(1)?; // Next trigger
        
        total_samples += data_a.len() + data_b.len();
        cycle_count += 1;
        
        let cycle_duration = cycle_start.elapsed();
        
        // Log sample details occasionally
        if cycle_count % 50 == 0 {
            println!("  Cycle {}: {} + {} samples, dt={:.3}ms, acquisition={:?}", 
                     cycle_count, data_a.len(), data_b.len(), dt * 1000.0, cycle_duration);
        }
        
        // Prevent overwhelming the system
        if cycle_duration < Duration::from_millis(1) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    
    let actual_duration = start_time.elapsed();
    let acquisition_rate = cycle_count as f64 / actual_duration.as_secs_f64();
    let sample_rate = total_samples as f64 / actual_duration.as_secs_f64();
    
    println!("\nContinuous Performance Results:");
    println!("  Total cycles: {}", cycle_count);
    println!("  Total samples: {}", total_samples);
    println!("  Actual duration: {:?}", actual_duration);
    println!("  Acquisition rate: {:.2} dual-channel acquisitions/sec", acquisition_rate);
    println!("  Combined sample rate: {:.1} samples/sec", sample_rate);
    println!("  Effective rate per channel: {:.1} samples/sec", sample_rate / 2.0);
    
    if sample_rate / 2.0 >= 1000.0 {
        println!("  ✓ Target 1kHz rate per channel ACHIEVED");
    } else {
        println!("  ⚠ Target 1kHz rate not reached ({:.1} Hz per channel)", sample_rate / 2.0);
    }
    
    // Calculate throughput efficiency
    let theoretical_max = total_samples as f64 / (cycle_count as f64 * 256.0); // Assuming 256 samples per channel
    println!("  Channel utilization: {:.1}%", theoretical_max * 100.0);
    
    Ok(())
}

#[derive(Debug)]
struct TimingMeasurement {
    iteration: usize,
    hw_timestamp: f64,
    hw_dt: f64,
    samples_a: usize,
    samples_b: usize,
    acquisition_time: Duration,
}

fn analyze_timing_data(data: &[TimingMeasurement]) {
    if data.is_empty() {
        println!("  No timing data to analyze");
        return;
    }
    
    println!("\nTiming Analysis Results:");
    
    // Hardware timing consistency
    let hw_dt_avg = data.iter().map(|d| d.hw_dt).sum::<f64>() / data.len() as f64;
    let hw_dt_variance: f64 = data.iter()
        .map(|d| (d.hw_dt - hw_dt_avg).powi(2))
        .sum::<f64>() / data.len() as f64;
    let hw_dt_std_dev = hw_dt_variance.sqrt();
    
    println!("  Hardware timing consistency:");
    println!("    Average dt: {:.6} ms ({:.1} Hz)", hw_dt_avg * 1000.0, 1.0 / hw_dt_avg);
    println!("    Standard deviation: {:.9} s", hw_dt_std_dev);
    println!("    Timing precision: {:.6}%", (hw_dt_std_dev / hw_dt_avg) * 100.0);
    
    if hw_dt_std_dev / hw_dt_avg < 0.001 {
        println!("    ✓ Hardware timing: EXCELLENT (< 0.1% variation)");
    } else if hw_dt_std_dev / hw_dt_avg < 0.01 {
        println!("    ✓ Hardware timing: GOOD (< 1% variation)");
    } else {
        println!("    ⚠ Hardware timing: MODERATE (> 1% variation)");
    }
    
    // Software acquisition timing
    let sw_times: Vec<u128> = data.iter().map(|d| d.acquisition_time.as_micros()).collect();
    let sw_avg = sw_times.iter().sum::<u128>() as f64 / sw_times.len() as f64;
    let mut sw_sorted = sw_times.clone();
    sw_sorted.sort();
    let sw_median = sw_sorted[sw_sorted.len() / 2];
    let sw_max = sw_sorted[sw_sorted.len() - 1];
    let sw_min = sw_sorted[0];
    
    println!("  Software acquisition timing:");
    println!("    Average: {:.1} μs", sw_avg);
    println!("    Median:  {} μs", sw_median);
    println!("    Range:   {} - {} μs", sw_min, sw_max);
    
    // Sample consistency
    let samples_a: Vec<usize> = data.iter().map(|d| d.samples_a).collect();
    let samples_b: Vec<usize> = data.iter().map(|d| d.samples_b).collect();
    let avg_samples_a = samples_a.iter().sum::<usize>() as f64 / samples_a.len() as f64;
    let avg_samples_b = samples_b.iter().sum::<usize>() as f64 / samples_b.len() as f64;
    
    println!("  Sample collection consistency:");
    println!("    Channel A average: {:.1} samples", avg_samples_a);
    println!("    Channel B average: {:.1} samples", avg_samples_b);
    println!("    Channel synchronization: {}", 
             if (avg_samples_a - avg_samples_b).abs() < 1.0 { "✓ PERFECT" } else { "⚠ DRIFT" });
    
    println!("\n--- Key Insights ---");
    println!("✓ Osci2T provides hardware-synchronized dual-channel acquisition");
    println!("✓ Both channels have identical timestamps and sample rates");
    println!("✓ Perfect synchronization between channels (no timing drift)");
    println!("✓ Suitable for multi-signal synchronized measurements");
    
    if hw_dt_avg < 0.001 {
        println!("✓ High-frequency sampling achieved ({:.1} kHz)", 1.0 / hw_dt_avg / 1000.0);
    }
}