use nanonis_rust::{ConnectionConfig, NanonisClient};
use std::error::Error;
use std::time::{Duration, Instant};

/// CH1 Signal Reader Agent
/// 
/// This agent continuously reads Channel 1 (Current) signal from Nanonis
/// and provides real-time monitoring with statistics.
struct Ch1ReaderAgent {
    client: NanonisClient,
    ch1_index: Option<i32>,
    readings: Vec<f32>,
    start_time: Instant,
}

impl Ch1ReaderAgent {
    /// Create new CH1 reader agent
    pub fn new(address: &str) -> Result<Self, Box<dyn Error>> {
        // Configure connection with appropriate timeouts for signal reading
        let config = ConnectionConfig {
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(2),  // Shorter for signal reading
            write_timeout: Duration::from_secs(2),
        };

        println!("Connecting to Nanonis at {}...", address);
        let mut client = NanonisClient::with_config(address, config)?;
        client.set_debug(false); // Keep quiet for continuous reading

        println!("Connected successfully!");

        Ok(Self {
            client,
            ch1_index: None,
            readings: Vec::new(),
            start_time: Instant::now(),
        })
    }

    /// Initialize the agent by finding CH1 signal index
    pub fn initialize(&mut self) -> Result<(), Box<dyn Error>> {
        println!("Searching for CH1/Current signal...");
        
        // Get all available signals
        let signals = self.client.get_signal_names()?;
        println!("Found {} total signals", signals.len());

        // Look for CH1 or Current signal
        let search_terms = ["current", "ch1", "input 1"];
        
        for (index, signal) in signals.iter().enumerate() {
            let signal_lower = signal.to_lowercase();
            for term in &search_terms {
                if signal_lower.contains(term) {
                    println!("Found CH1 signal: '{}' at index {}", signal, index);
                    self.ch1_index = Some(index as i32);
                    
                    // Get signal information
                    let (calibration, offset) = self.client.signals_calibr_get(index as i32)?;
                    let (max_range, min_range) = self.client.signals_range_get(index as i32)?;
                    
                    println!("Signal info:");
                    println!("  - Calibration: {} per volt", calibration);
                    println!("  - Offset: {} (physical units)", offset);
                    println!("  - Range: {} to {}", min_range, max_range);
                    
                    return Ok(());
                }
            }
        }

        // If not found, show available signals containing "current" or "input"
        println!("Warning: CH1/Current signal not found directly. Available candidates:");
        for (index, signal) in signals.iter().enumerate() {
            let signal_lower = signal.to_lowercase();
            if signal_lower.contains("current") || signal_lower.contains("input") {
                println!("  {}: {}", index, signal);
            }
        }

        Err("CH1/Current signal not found".into())
    }

    /// Read a single CH1 value
    pub fn read_ch1(&mut self, wait_for_newest: bool) -> Result<f32, Box<dyn Error>> {
        match self.ch1_index {
            Some(index) => Ok(self.client.signals_val_get(index, wait_for_newest)?),
            None => Err("CH1 signal not initialized".into()),
        }
    }

    /// Start continuous monitoring of CH1
    /// 
    /// # Arguments
    /// * `duration_secs` - Duration to monitor in seconds
    /// * `sample_interval_ms` - Interval between samples in milliseconds
    pub fn start_monitoring(&mut self, duration_secs: u64, sample_interval_ms: u64) -> Result<(), Box<dyn Error>> {
        println!("Starting CH1 monitoring for {} seconds (sampling every {}ms)", duration_secs, sample_interval_ms);
        println!("Reading CH1 signal...\n");

        let end_time = Instant::now() + Duration::from_secs(duration_secs);
        let mut sample_count = 0;
        let mut last_print = Instant::now();
        
        self.readings.clear();
        self.start_time = Instant::now();

        while Instant::now() < end_time {
            match self.read_ch1(false) { // Don't wait for newest data for faster sampling
                Ok(value) => {
                    self.readings.push(value);
                    sample_count += 1;

                    // Print updates every second
                    if last_print.elapsed() >= Duration::from_secs(1) {
                        let elapsed = self.start_time.elapsed().as_secs();
                        let avg_value = self.readings.iter().sum::<f32>() / self.readings.len() as f32;
                        let min_value = self.readings.iter().fold(f32::INFINITY, |a, &b| a.min(b));
                        let max_value = self.readings.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
                        
                        println!("[{}s] Current: {:.6}A | Samples: {} | Avg: {:.6}A | Min: {:.6}A | Max: {:.6}A", 
                                elapsed, value, sample_count, avg_value, min_value, max_value);
                        
                        last_print = Instant::now();
                    }
                }
                Err(e) => {
                    eprintln!("Error reading CH1: {}", e);
                    std::thread::sleep(Duration::from_millis(sample_interval_ms));
                    continue;
                }
            }

            std::thread::sleep(Duration::from_millis(sample_interval_ms));
        }

        self.print_final_statistics();
        Ok(())
    }

    /// Print final monitoring statistics
    fn print_final_statistics(&self) {
        if self.readings.is_empty() {
            println!("No successful readings recorded");
            return;
        }

        println!("\nFinal CH1 Monitoring Statistics:");
        println!("{}", "─".repeat(50));
        
        let total_samples = self.readings.len();
        let duration = self.start_time.elapsed();
        let sample_rate = total_samples as f64 / duration.as_secs_f64();
        
        let sum: f32 = self.readings.iter().sum();
        let avg = sum / total_samples as f32;
        let min = self.readings.iter().fold(f32::INFINITY, |a, &b| a.min(b));
        let max = self.readings.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        
        // Calculate standard deviation
        let variance: f32 = self.readings.iter()
            .map(|&x| (x - avg).powi(2))
            .sum::<f32>() / total_samples as f32;
        let std_dev = variance.sqrt();

        println!("Duration: {:.1}s", duration.as_secs_f64());
        println!("Total samples: {}", total_samples);
        println!("Sample rate: {:.1} Hz", sample_rate);
        println!("Average: {:.6}A", avg);
        println!("Minimum: {:.6}A", min);
        println!("Maximum: {:.6}A", max);
        println!("Range: {:.6}A", max - min);
        println!("Standard deviation: {:.6}A", std_dev);
        
        // Show signal stability
        let stability_percent = (std_dev / avg.abs() * 100.0).abs();
        println!("Stability: {:.3}%", stability_percent);
        
        if stability_percent < 1.0 {
            println!("Status: Signal is very stable");
        } else if stability_percent < 5.0 {
            println!("Status: Signal has moderate noise");
        } else {
            println!("Status: Signal is noisy");
        }
    }

    /// Perform a quick CH1 test read to verify functionality
    pub fn test_read(&mut self) -> Result<(), Box<dyn Error>> {
        println!("Performing test read of CH1...");
        
        let value1 = self.read_ch1(false)?;
        println!("CH1 (immediate): {:.6}A", value1);
        
        std::thread::sleep(Duration::from_millis(100));
        
        let value2 = self.read_ch1(true)?;
        println!("CH1 (newest data): {:.6}A", value2);
        
        let diff = (value2 - value1).abs();
        println!("Difference: {:.6}A", diff);
        
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // Initialize environment logger (use RUST_LOG=debug for detailed output)
    env_logger::init();

    println!("CH1 Signal Reader Agent Starting...");
    println!("{}", "═".repeat(50));

    // Create and initialize the agent
    let mut agent = Ch1ReaderAgent::new("127.0.0.1:6501")?;
    agent.initialize()?;

    println!("\nAgent initialized successfully!");
    println!("Available operations:");
    println!("1. Quick test read");
    println!("2. Continuous monitoring (10 seconds)");
    println!("3. Long monitoring (60 seconds)");
    
    // For this example, perform a test read and short monitoring session
    println!("\nRunning quick test...");
    agent.test_read()?;
    
    println!("\nStarting 10-second monitoring session...");
    agent.start_monitoring(10, 100)?; // 10 seconds, 100ms interval (10 Hz)
    
    println!("\nCH1 Reader Agent completed successfully!");
    Ok(())
}