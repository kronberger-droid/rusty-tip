use rusty_tip::NanonisClient;
use std::error::Error;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logger for timing info
    env_logger::init();

    let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    
    println!("Signal Value Get - Timing Benchmark");
    println!("=====================================");
    
    // Test parameters
    let test_iterations = 1000;
    let signal_indices = vec![0, 1, 2, 24]; // Common signals including bias
    let warmup_iterations = 10;
    
    println!("Test Configuration:");
    println!("- Iterations: {}", test_iterations);
    println!("- Signal indices: {:?}", signal_indices);
    println!("- Warmup iterations: {}", warmup_iterations);
    println!();
    
    // Warmup phase
    println!("Warming up...");
    for _ in 0..warmup_iterations {
        client.signals_val_get(signal_indices.clone(), true)?;
    }
    
    // Timing test with wait_for_newest_data = true
    println!("Testing with wait_for_newest_data = true:");
    let timings_newest = benchmark_signal_reads(&mut client, &signal_indices, test_iterations, true)?;
    print_timing_stats("Wait for newest data", &timings_newest);
    
    // Timing test with wait_for_newest_data = false
    println!("\nTesting with wait_for_newest_data = false:");
    let timings_cached = benchmark_signal_reads(&mut client, &signal_indices, test_iterations, false)?;
    print_timing_stats("Cached data", &timings_cached);
    
    // Single signal vs multiple signals comparison
    println!("\n--- Single vs Multiple Signal Comparison ---");
    
    // Single signal timing (bias voltage at index 24)
    let single_timings = benchmark_single_signal(&mut client, 24, 500, true)?;
    print_timing_stats("Single signal (index 24)", &single_timings);
    
    // Multiple signals timing
    let multi_timings = benchmark_signal_reads(&mut client, &signal_indices, 500, true)?;
    print_timing_stats("Multiple signals", &multi_timings);
    
    // Consistency analysis
    println!("\n--- Timing Consistency Analysis ---");
    analyze_consistency("Wait for newest", &timings_newest);
    analyze_consistency("Cached data", &timings_cached);
    
    // Throughput calculation
    println!("\n--- Throughput Analysis ---");
    calculate_throughput("Wait for newest", &timings_newest, signal_indices.len());
    calculate_throughput("Cached data", &timings_cached, signal_indices.len());
    
    Ok(())
}

fn benchmark_signal_reads(
    client: &mut NanonisClient,
    signal_indices: &[i32],
    iterations: usize,
    wait_for_newest: bool,
) -> Result<Vec<Duration>, Box<dyn Error>> {
    let mut timings = Vec::with_capacity(iterations);
    
    for i in 0..iterations {
        let start = Instant::now();
        client.signals_val_get(signal_indices.to_vec(), wait_for_newest)?;
        let duration = start.elapsed();
        timings.push(duration);
        
        // Progress indicator every 100 iterations
        if (i + 1) % 100 == 0 {
            println!("  Completed {}/{} iterations", i + 1, iterations);
        }
    }
    
    Ok(timings)
}

fn benchmark_single_signal(
    client: &mut NanonisClient,
    signal_index: i32,
    iterations: usize,
    wait_for_newest: bool,
) -> Result<Vec<Duration>, Box<dyn Error>> {
    let mut timings = Vec::with_capacity(iterations);
    
    for _ in 0..iterations {
        let start = Instant::now();
        client.signals_val_get(vec![signal_index], wait_for_newest)?;
        let duration = start.elapsed();
        timings.push(duration);
    }
    
    Ok(timings)
}

fn print_timing_stats(test_name: &str, timings: &[Duration]) {
    let total_time: Duration = timings.iter().sum();
    let avg_time = total_time / timings.len() as u32;
    
    let mut sorted_timings = timings.to_vec();
    sorted_timings.sort();
    
    let min_time = sorted_timings[0];
    let max_time = sorted_timings[sorted_timings.len() - 1];
    let median_time = sorted_timings[sorted_timings.len() / 2];
    let p95_time = sorted_timings[(sorted_timings.len() as f64 * 0.95) as usize];
    let p99_time = sorted_timings[(sorted_timings.len() as f64 * 0.99) as usize];
    
    println!("{} Results:", test_name);
    println!("  Total iterations: {}", timings.len());
    println!("  Average time: {:?}", avg_time);
    println!("  Median time:  {:?}", median_time);
    println!("  Min time:     {:?}", min_time);
    println!("  Max time:     {:?}", max_time);
    println!("  95th percentile: {:?}", p95_time);
    println!("  99th percentile: {:?}", p99_time);
    println!("  Total time:   {:?}", total_time);
}

fn analyze_consistency(test_name: &str, timings: &[Duration]) {
    let avg_micros: f64 = timings.iter().map(|d| d.as_micros() as f64).sum::<f64>() / timings.len() as f64;
    
    let variance: f64 = timings.iter()
        .map(|d| {
            let diff = d.as_micros() as f64 - avg_micros;
            diff * diff
        })
        .sum::<f64>() / timings.len() as f64;
    
    let std_dev = variance.sqrt();
    let coefficient_of_variation = std_dev / avg_micros * 100.0;
    
    println!("{} Consistency:", test_name);
    println!("  Standard deviation: {:.2} μs", std_dev);
    println!("  Coefficient of variation: {:.2}%", coefficient_of_variation);
    
    if coefficient_of_variation < 5.0 {
        println!("  Timing consistency: EXCELLENT (< 5% CV)");
    } else if coefficient_of_variation < 10.0 {
        println!("  Timing consistency: GOOD (5-10% CV)");
    } else if coefficient_of_variation < 20.0 {
        println!("  Timing consistency: MODERATE (10-20% CV)");
    } else {
        println!("  Timing consistency: POOR (> 20% CV)");
    }
}

fn calculate_throughput(test_name: &str, timings: &[Duration], signals_per_read: usize) {
    let avg_time = timings.iter().sum::<Duration>() / timings.len() as u32;
    let reads_per_second = 1.0 / avg_time.as_secs_f64();
    let signals_per_second = reads_per_second * signals_per_read as f64;
    
    println!("{} Throughput:", test_name);
    println!("  Reads per second: {:.2}", reads_per_second);
    println!("  Signals per second: {:.2}", signals_per_second);
    println!("  Time per signal: {:.2} μs", avg_time.as_micros() as f64 / signals_per_read as f64);
}