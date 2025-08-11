use nanonis_rust::{
    BoundaryClassifier, JsonDiskWriter, RuleBasedPolicy, NanonisClient, Controller, AsyncSignalMonitor, StateClassifier
};
use std::error::Error;

/// Comprehensive demo of all builder patterns in the nanonis_rust library
/// 
/// This example demonstrates the coherent builder patterns implemented across
/// all major structs in the library, showing consistent API design and usage.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    
    log::info!("Builder Patterns Demonstration");
    log::info!("Showing coherent builder API design across all components");

    // 1. BoundaryClassifier Builder
    log::info!("1. Building BoundaryClassifier with fluent API...");
    let classifier = BoundaryClassifier::builder()
        .name("Demo Classifier")
        .signal_index(24)
        .bounds(0.0, 2.0)
        .buffer_config(10, 2)
        .stability_threshold(3)
        .build()?;
    log::info!("   Built: {}", classifier.get_name());

    // 2. RuleBasedPolicy Builder  
    log::info!("2. Building RuleBasedPolicy with fluent API...");
    let policy = RuleBasedPolicy::builder()
        .name("Demo Policy")
        .build()?;
    log::info!("   Built RuleBasedPolicy successfully");

    // 3. JsonDiskWriter Builder
    log::info!("3. Building JsonDiskWriter with fluent API...");
    let disk_writer = JsonDiskWriter::builder()
        .file_path("examples/history/builder_demo.jsonl")
        .pretty(true)  // Pretty formatting for demo
        .buffer_size(4096)
        .build().await?;
    log::info!("   Built JsonDiskWriter with pretty formatting");

    // 4. NanonisClient Builder (existing)
    log::info!("4. Building NanonisClient with fluent API...");
    let client_result = NanonisClient::builder()
        .address("127.0.0.1")
        .port(6501)
        .debug(false)
        .build();
    match client_result {
        Ok(_client) => log::info!("   Built NanonisClient successfully (connected to Nanonis)"),
        Err(e) => log::info!("   NanonisClient build failed (expected without running Nanonis): {}", e),
    }

    // 5. AsyncSignalMonitor Builder (existing)
    log::info!("5. Building AsyncSignalMonitor with fluent API...");
    let signal_monitor = AsyncSignalMonitor::builder()
        .address("127.0.0.1")
        .port(6501)
        .signals(vec![0, 24, 30])
        .sample_rate(10.0)
        .with_disk_writer(Box::new(disk_writer))
        .build()?;
    log::info!("   Built AsyncSignalMonitor with disk writer integration");

    // 6. Controller Builder (existing) - requires components
    log::info!("6. Building Controller with fluent API...");
    let controller = Controller::builder()
        .address("127.0.0.1")
        .port(6501)
        .classifier(Box::new(classifier))
        .policy(Box::new(policy))
        .control_interval(2.0)
        .max_approaches(3)
        .build()?;
    log::info!("   Built Controller with all integrated components");

    log::info!("");
    log::info!("Builder Patterns Summary:");
    log::info!("   All builders follow consistent patterns:");
    log::info!("   - Static builder() method on main struct");
    log::info!("   - Fluent API with method chaining");
    log::info!("   - Required fields use Option<T> with validation");
    log::info!("   - Optional fields have sensible defaults");
    log::info!("   - build() method with comprehensive error handling");
    log::info!("   - Consistent naming: .name(), .address(), .port(), etc.");
    log::info!("");
    log::info!("Builder patterns demonstration complete!");

    Ok(())
}