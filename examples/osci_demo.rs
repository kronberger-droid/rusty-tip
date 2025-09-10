use rusty_tip::actions::{Action, ActionResult};
use rusty_tip::action_driver::ActionDriver;
use rusty_tip::types::{DataToGet, SignalIndex, TriggerConfig, TriggerSlope};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    
    println!("Oscilloscope Demo - Reading oscilloscope data with trigger configuration");
    
    // Create action driver (will attempt to connect to local Nanonis instance)
    let mut driver = match ActionDriver::new("127.0.0.1", 6501) {
        Ok(driver) => driver,
        Err(e) => {
            println!("Failed to connect to Nanonis: {}", e);
            println!("Make sure Nanonis is running and accessible at 127.0.0.1:6501");
            return Ok(());
        }
    };

    // Example 1: Simple oscilloscope read with immediate trigger, current data
    println!("\n=== Example 1: Immediate trigger, current data ===");
    let immediate_trigger = TriggerConfig::immediate();
    let osci_action = Action::ReadOsci {
        signal: SignalIndex(24), // Bias voltage channel
        trigger: Some(immediate_trigger),
        data_to_get: DataToGet::Current,
    };

    match driver.execute(osci_action) {
        Ok(ActionResult::OscilloscopeData(data)) => {
            println!("Oscilloscope data acquired:");
            println!("  Start time (t0): {:.6} s", data.t0);
            println!("  Time step (dt): {:.9} s", data.dt);
            println!("  Sample count: {}", data.size);
            println!("  Sample rate: {:.3} Hz", data.sample_rate());
            println!("  Duration: {:.6} s", data.duration());
            println!("  First 5 data points: {:?}", &data.data[..5.min(data.data.len())]);
        }
        Ok(other) => println!("Unexpected result: {:?}", other),
        Err(e) => println!("Failed to read oscilloscope: {}", e),
    }

    // Example 2: Level trigger at 1V with rising edge, wait for next trigger
    println!("\n=== Example 2: Level trigger at 1V (rising edge), next trigger ===");
    let level_trigger = TriggerConfig::level_trigger(1.0, TriggerSlope::Rising);
    let osci_action = Action::ReadOsci {
        signal: SignalIndex(0), // Current channel
        trigger: Some(level_trigger),
        data_to_get: DataToGet::NextTrigger,
    };

    match driver.execute(osci_action) {
        Ok(ActionResult::OscilloscopeData(data)) => {
            println!("Triggered oscilloscope data acquired:");
            println!("  Sample count: {}", data.size);
            println!("  Time range: {:.6} - {:.6} s", data.t0, data.t0 + data.duration());
            
            // Find min/max values in the data
            let min_val = data.data.iter().fold(f64::INFINITY, |a, &b| a.min(b));
            let max_val = data.data.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
            println!("  Value range: {:.6e} - {:.6e}", min_val, max_val);
        }
        Ok(other) => println!("Unexpected result: {:?}", other),
        Err(e) => println!("Failed to read triggered oscilloscope: {}", e),
    }

    // Example 3: Auto trigger mode, wait for 2 triggers for stable data
    println!("\n=== Example 3: Auto trigger mode, wait 2 triggers ===");
    let auto_trigger = TriggerConfig::auto_trigger();
    let osci_action = Action::ReadOsci {
        signal: SignalIndex(1), // Height/Z channel
        trigger: Some(auto_trigger),
        data_to_get: DataToGet::Wait2Triggers,
    };

    match driver.execute(osci_action) {
        Ok(ActionResult::OscilloscopeData(data)) => {
            println!("Auto-triggered oscilloscope data acquired:");
            println!("  Data points: {}", data.size);
            
            // Calculate some basic statistics
            let mean = data.data.iter().sum::<f64>() / data.data.len() as f64;
            let variance = data.data.iter()
                .map(|&x| (x - mean).powi(2))
                .sum::<f64>() / data.data.len() as f64;
            let std_dev = variance.sqrt();
            
            println!("  Mean value: {:.6e}", mean);
            println!("  Std deviation: {:.6e}", std_dev);
        }
        Ok(other) => println!("Unexpected result: {:?}", other),
        Err(e) => println!("Failed to read auto-triggered oscilloscope: {}", e),
    }

    // Example 4: No trigger configuration (uses existing settings)
    println!("\n=== Example 4: Using existing trigger settings ===");
    let osci_action = Action::ReadOsci {
        signal: SignalIndex(2), // Some other channel
        trigger: None, // Use whatever trigger is already configured
        data_to_get: DataToGet::Current,
    };

    match driver.execute(osci_action) {
        Ok(ActionResult::OscilloscopeData(data)) => {
            println!("Oscilloscope data with existing trigger:");
            println!("  Sample count: {}", data.size);
            
            // Generate time points for plotting/analysis
            let time_points = data.time_points();
            println!("  Time range: {:.6} - {:.6} s", 
                    time_points.first().unwrap_or(&0.0),
                    time_points.last().unwrap_or(&0.0));
        }
        Ok(other) => println!("Unexpected result: {:?}", other),
        Err(e) => println!("Failed to read oscilloscope with existing settings: {}", e),
    }

    println!("\n=== Demo completed ===");
    println!("The ReadOsci action supports:");
    println!("- Configurable trigger modes (Immediate, Level, Auto)"); 
    println!("- Trigger slope selection (Rising/Falling)");
    println!("- Adjustable trigger level and hysteresis");
    println!("- User-controlled data acquisition mode:");
    println!("  * DataToGet::Current - Get current buffer");
    println!("  * DataToGet::NextTrigger - Wait for next trigger");
    println!("  * DataToGet::Wait2Triggers - Wait for 2 triggers (more stable)");
    println!("- Rich data structure with timing information");
    
    Ok(())
}