use rusty_tip::NanonisClient;
use std::error::Error;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    
    // Setup oscilloscope
    client.osci1t_run()?;
    client.osci1t_ch_set(0)?;
    
    println!("=== Oscilloscope Continuity Analysis ===\n");
    
    // 1. Timing Benchmark
    println!("1. TCP Call Timing Benchmark:");
    timing_benchmark(&mut client)?;
    
    println!("\n{}\n", "=".repeat(50));
    
    // 2. Trigger Investigation
    println!("2. Trigger Investigation:");
    trigger_investigation(&mut client)?;
    
    println!("\n{}\n", "=".repeat(50));
    
    // 3. Buffer Overlap Analysis
    println!("3. Buffer Overlap Analysis:");
    buffer_overlap_analysis(&mut client)?;
    
    println!("\n{}\n", "=".repeat(50));
    
    // 4. Signal Continuity Analysis
    println!("4. Signal Continuity Analysis:");
    signal_continuity_analysis(&mut client)?;
    
    println!("\n{}\n", "=".repeat(50));
    
    // 5. Polling Strategy Optimization
    println!("5. Polling Strategy Optimization:");
    polling_strategy_test(&mut client)?;
    
    Ok(())
}

fn timing_benchmark(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    const NUM_CALLS: usize = 20;
    let mut call_times = Vec::new();
    
    println!("Measuring {} consecutive osci1t_data_get() calls...", NUM_CALLS);
    
    let overall_start = Instant::now();
    
    for i in 0..NUM_CALLS {
        let start = Instant::now();
        let _data = client.osci1t_data_get(0)?;
        let duration = start.elapsed();
        call_times.push(duration);
        
        if i < 5 || i >= NUM_CALLS - 5 {
            println!("  Call {}: {:?}", i + 1, duration);
        } else if i == 5 {
            println!("  ... (showing first/last 5 calls)");
        }
    }
    
    let total_time = overall_start.elapsed();
    let avg_time = call_times.iter().sum::<Duration>() / call_times.len() as u32;
    let min_time = call_times.iter().min().unwrap();
    let max_time = call_times.iter().max().unwrap();
    
    println!("\nTiming Statistics:");
    println!("  Total time: {:?}", total_time);
    println!("  Average call time: {:?}", avg_time);
    println!("  Min call time: {:?}", min_time);
    println!("  Max call time: {:?}", max_time);
    println!("  Effective polling rate: {:.1} Hz", NUM_CALLS as f64 / total_time.as_secs_f64());
    
    Ok(())
}

fn trigger_investigation(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("Investigating trigger settings...");
    
    // Check current trigger settings
    match client.osci1t_trig_get() {
        Ok(trigger_info) => {
            println!("  Current trigger info: {:?}", trigger_info);
        }
        Err(e) => {
            println!("  Error getting trigger info: {}", e);
        }
    }
    
    // Get multiple data samples to see timing behavior
    println!("\nSample acquisition timestamps:");
    for i in 0..5 {
        let start = Instant::now();
        match client.osci1t_data_get(0) {
            Ok((timestamp, _, _, data)) => {
                let call_time = start.elapsed();
                println!("  Sample {}: timestamp={}, samples={}, call_time={:?}", 
                         i + 1, timestamp, data.len(), call_time);
            }
            Err(e) => {
                println!("  Sample {}: Error - {}", i + 1, e);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    
    Ok(())
}

fn buffer_overlap_analysis(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("Analyzing buffer overlap between consecutive calls...");
    
    const NUM_BUFFERS: usize = 5;
    let mut buffers = Vec::new();
    let mut timestamps = Vec::new();
    
    // Collect multiple buffers rapidly
    for i in 0..NUM_BUFFERS {
        let (timestamp, _, _, data) = client.osci1t_data_get(0)?;
        buffers.push(data);
        timestamps.push(timestamp);
        println!("  Buffer {}: timestamp={}, samples={}", i + 1, timestamp, buffers[i].len());
    }
    
    // Analyze overlaps between consecutive buffers
    println!("\nBuffer overlap analysis:");
    for i in 0..NUM_BUFFERS - 1 {
        let buf1 = &buffers[i];
        let buf2 = &buffers[i + 1];
        
        // Check if last samples of buf1 match first samples of buf2
        let overlap_size = find_overlap(buf1, buf2);
        let time_diff = timestamps[i + 1] - timestamps[i];
        
        println!("  Buffer {} -> {}: overlap={} samples, time_diff={:.3}ms", 
                 i + 1, i + 2, overlap_size, time_diff);
    }
    
    Ok(())
}

fn find_overlap(buf1: &[f64], buf2: &[f64]) -> usize {
    let check_size = 10.min(buf1.len()).min(buf2.len());
    
    for overlap in 1..=check_size {
        let buf1_end = &buf1[buf1.len() - overlap..];
        let buf2_start = &buf2[..overlap];
        
        // Check if sequences match (with small tolerance for floating point)
        let matches = buf1_end.iter().zip(buf2_start.iter())
            .all(|(a, b)| (a - b).abs() < 1e-10);
        
        if matches {
            return overlap;
        }
    }
    
    0 // No overlap found
}

fn signal_continuity_analysis(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("Analyzing signal continuity...");
    
    const NUM_SAMPLES: usize = 10;
    let mut all_data = Vec::new();
    let mut sample_times = Vec::new();
    
    // Collect data samples as quickly as possible
    println!("Collecting {} rapid samples...", NUM_SAMPLES);
    let start_time = Instant::now();
    
    for i in 0..NUM_SAMPLES {
        let sample_start = Instant::now();
        let (_, _, _, data) = client.osci1t_data_get(0)?;
        let sample_time = sample_start.elapsed();
        
        all_data.push(data);
        sample_times.push(sample_time);
        
        if i < 3 || i >= NUM_SAMPLES - 3 {
            println!("  Sample {}: {} points, {:?}", i + 1, all_data[i].len(), sample_time);
        } else if i == 3 {
            println!("  ... (showing first/last 3)");
        }
    }
    
    let total_time = start_time.elapsed();
    println!("Total collection time: {:?}", total_time);
    
    // Analyze signal statistics
    println!("\nSignal statistics per buffer:");
    for (i, data) in all_data.iter().enumerate() {
        if !data.is_empty() {
            let mean = data.iter().sum::<f64>() / data.len() as f64;
            let variance = data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / data.len() as f64;
            let std_dev = variance.sqrt();
            let min_val = data.iter().fold(f64::INFINITY, |a, &b| a.min(b));
            let max_val = data.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
            
            println!("  Buffer {}: mean={:.6}, std={:.6}, range=[{:.6}, {:.6}]", 
                     i + 1, mean, std_dev, min_val, max_val);
        }
    }
    
    // Look for discontinuities between buffers
    println!("\nBuffer transition analysis:");
    for i in 0..NUM_SAMPLES - 1 {
        let buf1 = &all_data[i];
        let buf2 = &all_data[i + 1];
        
        if !buf1.is_empty() && !buf2.is_empty() {
            let last_val = buf1[buf1.len() - 1];
            let first_val = buf2[0];
            let jump = (first_val - last_val).abs();
            
            println!("  Buffer {} -> {}: last={:.6}, first={:.6}, jump={:.6}", 
                     i + 1, i + 2, last_val, first_val, jump);
        }
    }
    
    Ok(())
}

fn polling_strategy_test(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("Testing different polling strategies for optimal data continuity...\n");
    
    // Strategy 1: Rapid polling with no delays
    println!("Strategy 1: Rapid Polling (no delays)");
    test_rapid_polling(client)?;
    
    println!("\nStrategy 2: Burst Collection");
    test_burst_collection(client)?;
    
    println!("\nStrategy 3: Measured Intervals");
    test_measured_intervals(client)?;
    
    Ok(())
}

fn test_rapid_polling(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    const SAMPLES: usize = 8;
    let mut gaps = Vec::new();
    let start_time = Instant::now();
    
    println!("  Collecting {} samples as fast as possible...", SAMPLES);
    
    let mut prev_buffer: Option<Vec<f64>> = None;
    
    for i in 0..SAMPLES {
        let (_, _, _, data) = client.osci1t_data_get(0)?;
        
        if let Some(prev) = prev_buffer {
            let gap = analyze_buffer_gap(&prev, &data);
            gaps.push(gap);
            if i <= 3 {
                println!("    Gap {}: {} samples", i, gap);
            }
        }
        
        prev_buffer = Some(data);
    }
    
    let total_time = start_time.elapsed();
    let avg_gap = gaps.iter().sum::<usize>() as f64 / gaps.len() as f64;
    
    println!("  Total time: {:?}", total_time);
    println!("  Average gap: {:.1} samples", avg_gap);
    println!("  Data coverage: {:.1}%", (256.0 / (256.0 + avg_gap)) * 100.0);
    
    Ok(())
}

fn test_burst_collection(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("  Collecting bursts of samples with intervals...");
    
    for burst in 1..=3 {
        println!("    Burst {}: Collecting 3 rapid samples", burst);
        
        let burst_start = Instant::now();
        let mut burst_gaps = Vec::new();
        let mut prev_buffer: Option<Vec<f64>> = None;
        
        for i in 0..3 {
            let (_, _, _, data) = client.osci1t_data_get(0)?;
            
            if let Some(prev) = prev_buffer {
                let gap = analyze_buffer_gap(&prev, &data);
                burst_gaps.push(gap);
            }
            
            prev_buffer = Some(data);
        }
        
        let burst_time = burst_start.elapsed();
        let avg_gap = if !burst_gaps.is_empty() {
            burst_gaps.iter().sum::<usize>() as f64 / burst_gaps.len() as f64
        } else {
            0.0
        };
        
        println!("      Time: {:?}, Avg gap: {:.1} samples", burst_time, avg_gap);
        
        // Wait between bursts
        if burst < 3 {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    
    Ok(())
}

fn test_measured_intervals(client: &mut NanonisClient) -> Result<(), Box<dyn Error>> {
    println!("  Testing different polling intervals...");
    
    let intervals = [10, 50, 100, 200]; // milliseconds
    
    for &interval_ms in &intervals {
        println!("    Interval: {}ms", interval_ms);
        
        let mut gaps = Vec::new();
        let mut prev_buffer: Option<Vec<f64>> = None;
        
        for i in 0..4 {
            let (_, _, _, data) = client.osci1t_data_get(0)?;
            
            if let Some(prev) = prev_buffer {
                let gap = analyze_buffer_gap(&prev, &data);
                gaps.push(gap);
            }
            
            prev_buffer = Some(data);
            
            if i < 3 {
                std::thread::sleep(Duration::from_millis(interval_ms));
            }
        }
        
        let avg_gap = gaps.iter().sum::<usize>() as f64 / gaps.len() as f64;
        println!("      Average gap: {:.1} samples", avg_gap);
    }
    
    Ok(())
}

fn analyze_buffer_gap(buf1: &[f64], buf2: &[f64]) -> usize {
    // Look for the exact position where buf1 ends and buf2 begins
    // This estimates how many samples were missed between buffers
    
    let overlap = find_overlap(buf1, buf2);
    
    if overlap > 0 {
        // If there's overlap, no gap
        0
    } else {
        // Estimate gap based on signal characteristics
        // For now, assume worst case: full buffer worth of missing data
        buf1.len()
    }
}