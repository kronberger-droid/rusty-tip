use rusty_tip::NanonisClient;
use std::error::Error;
use std::time::Instant;

fn timed_call<T, E>(
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<(T, std::time::Duration), E> {
    let start = Instant::now();
    let result = operation()?;
    let duration = start.elapsed();
    Ok((result, duration))
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut client = NanonisClient::new("127.0.0.1", 6501)?;
    let mut buffer = Vec::new();
    let mut durations = Vec::new();

    for i in 0..10 {
        let (values, duration) = timed_call(|| client.signals_val_get((0..=10).collect(), true))?;

        buffer.push(values);
        durations.push(duration);
        println!("Call {}: {:?} ({} μs)", i, duration, duration.as_micros());
    }

    // Calculate statistics
    let total: std::time::Duration = durations.iter().sum();
    let avg = total / durations.len() as u32;
    let min = durations.iter().min().unwrap();
    let max = durations.iter().max().unwrap();

    println!("\n--- Statistics ---");
    println!("Total time: {:?}", total);
    println!("Average: {:?} ({} μs)", avg, avg.as_micros());
    println!("Min: {:?} ({} μs)", min, min.as_micros());
    println!("Max: {:?} ({} μs)", max, max.as_micros());
    println!("Calls per second: {:.2}", 1.0 / avg.as_secs_f64());

    Ok(())
}
