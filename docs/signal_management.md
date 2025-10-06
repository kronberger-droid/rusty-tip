# Signal Management Implementation Guide

## Overview

This document outlines the implementation of a unified signal management system for the `rusty-tip` library to solve the complexity of dual signal indexing systems in Nanonis SPM control.

## Problem Statement

### Dual Signal Indexing Challenge

Nanonis systems use **two separate signal indexing systems** that create significant complexity:

1. **Full Signal Registry (0-127)**
   - Accessed via `Signals.NamesGet` command
   - Used by most Nanonis commands (`signals_vals_get`, `signal_val_get`, etc.)
   - Maps signal names to their complete system indices
   - Example: "Bias (V)" might be at index 24

2. **TCP Logger Signal Slots (0-23)**  
   - Configured via `tcplog_chs_set(vec![0, 1, 2])`
   - Used by BufferedTCPReader and all data collection operations
   - Limited to 24 "slots" in the Signals Manager
   - **Critical Issue**: No programmatic way to determine which full signal (0-127) is assigned to each slot (0-23)

### Current Problems

```rust
// ❌ Current approach - fragile and error-prone
let experiment_data = driver.execute_chain_with_data_collection(
    actions,
    Duration::from_millis(200),
    Duration::from_millis(1000),
)?;

// Hard-coded assumptions about what's in each slot
let channel_names = ["Bias (V)", "Current (nA)", "Z Position (nm)"];
// ↑ No validation that slot 0 actually contains bias voltage!
```

**Issues:**
- **Hard-coded assumptions**: Code assumes slot 0=Bias, slot 1=Current without validation
- **No verification**: Can't confirm desired signals are actually available in logger slots  
- **Manual guesswork**: Users must manually figure out signal-to-slot mappings
- **Brittle plotting**: Signal names in plots don't correspond to actual data channels
- **Setup dependency**: Code breaks when signal assignments change between setups

## Solution Architecture

### Core Design Principles

1. **User-Friendly Interface**: Use signal names instead of cryptic indices
2. **Automatic Validation**: Verify that requested signals are available
3. **Flexible Configuration**: Support different signal assignments across setups
4. **Graceful Degradation**: Provide helpful error messages and suggestions
5. **Future-Proof**: Ready for RT signal query integration when available

### Component Overview

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────────┐
│   SignalManager │    │ BufferedTCPReader│    │ NamedDataCollection │
│                 │    │                  │    │                     │
│ • Full registry │────▶ • Signal validation│────▶ • Name-based API    │
│ • Slot mapping  │    │ • Metadata       │    │ • Auto plotting     │
│ • Name resolution│    │ • Enhanced frames│    │ • Unit awareness    │
└─────────────────┘    └──────────────────┘    └─────────────────────┘
          │                       │                        │
          ▼                       ▼                        ▼
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────────┐
│ Configuration   │    │ Signal Validation│    │ Enhanced Examples   │
│                 │    │                  │    │                     │
│ • Default slots │    │ • Fuzzy matching │    │ • tip_shaper_named  │
│ • Custom mapping│    │ • Error reporting│    │ • Auto unit scaling │
│ • JSON config   │    │ • Suggestions    │    │ • Smart plotting    │
└─────────────────┘    └──────────────────┘    └─────────────────────┘
```

## Implementation Components

### 1. SignalManager (`src/signal_manager.rs`)

The central component that manages all signal-related operations.

```rust
pub struct SignalManager {
    client: Arc<Mutex<NanonisClient>>,
    
    // Full signal registry cache (0-127)
    full_registry: HashMap<i32, String>,        // index -> name
    name_to_index: HashMap<String, i32>,        // normalized_name -> index
    
    // TCP Logger slot mapping (0-23)
    slot_assignments: HashMap<u8, Option<String>>, // slot -> signal_name
    
    // Configuration
    config: SignalManagerConfig,
    cache_timestamp: Instant,
    cache_ttl: Duration,
}

pub struct SignalManagerConfig {
    pub default_slot_mapping: HashMap<u8, String>,
    pub fuzzy_matching_threshold: f32,
    pub cache_ttl_seconds: u64,
    pub auto_refresh: bool,
}
```

#### Core Methods

```rust
impl SignalManager {
    // Registry management
    pub fn new(client: Arc<Mutex<NanonisClient>>) -> Result<Self, NanonisError>;
    pub fn refresh_full_registry(&mut self) -> Result<(), NanonisError>;
    pub fn get_signal_index_by_name(&self, name: &str) -> Result<Option<i32>, NanonisError>;
    pub fn get_signal_name_by_index(&self, index: i32) -> Result<Option<String>, NanonisError>;
    
    // TCP Logger slot management
    pub fn get_slot_signal_name(&self, slot: u8) -> Option<&String>;
    pub fn find_signal_slot(&self, signal_name: &str) -> Option<u8>;
    pub fn validate_slot_assignments(&self, requested: &[&str]) -> SlotValidationResult;
    
    // Smart resolution
    pub fn resolve_signal_names(&self, names: &[&str]) -> Result<Vec<ResolvedSignal>, NanonisError>;
    pub fn suggest_similar_signals(&self, partial_name: &str) -> Vec<SuggestionMatch>;
    pub fn normalize_signal_name(name: &str) -> String;  // "bias" -> "bias"
}

pub struct ResolvedSignal {
    pub requested_name: String,
    pub resolved_name: String,
    pub full_index: Option<i32>,
    pub slot: Option<u8>,
    pub confidence: f32,
}

pub struct SlotValidationResult {
    pub available_signals: Vec<ResolvedSignal>,
    pub missing_signals: Vec<String>,
    pub suggestions: Vec<SuggestionMatch>,
}

pub struct SuggestionMatch {
    pub signal_name: String,
    pub similarity: f32,
    pub available_in_slot: Option<u8>,
}
```

### 2. Enhanced BufferedTCPReader

Integration with signal management for validation and metadata.

```rust
impl BufferedTCPReader {
    // Enhanced constructor with signal validation
    pub fn new_with_signal_validation(
        address: &str,
        port: u16,
        requested_signals: &[&str],
        signal_manager: &SignalManager,
        buffer_params: BufferParameters,
    ) -> Result<(Self, SignalMappingReport), NanonisError>;
    
    // Signal-aware data queries  
    pub fn get_data_by_signal_name(
        &self,
        signal_name: &str,
        time_range: TimeRange,
    ) -> Result<Vec<(f32, f32)>, NanonisError>;
    
    pub fn get_enriched_frames(
        &self,
        time_range: TimeRange,
    ) -> Result<Vec<EnrichedSignalFrame>, NanonisError>;
}

pub struct SignalMappingReport {
    pub configured_signals: Vec<ConfiguredSignal>,
    pub warnings: Vec<String>,
    pub slot_utilization: f32,  // Percentage of used slots
}

pub struct ConfiguredSignal {
    pub name: String,
    pub slot: u8,
    pub full_index: Option<i32>,
    pub units: Option<String>,
    pub validation_status: ValidationStatus,
}

pub enum ValidationStatus {
    Confirmed,      // Signal verified in slot
    Assumed,        // Using default mapping
    Missing,        // Signal not found anywhere
    Conflicted,     // Multiple signals with similar names
}
```

### 3. Enhanced Data Structures

#### EnrichedSignalFrame

```rust
pub struct EnrichedSignalFrame {
    pub timestamp: Instant,
    pub relative_time: Duration,
    pub frame_counter: u32,
    pub signals: Vec<SignalDataPoint>,
}

pub struct SignalDataPoint {
    pub name: String,
    pub value: f64,
    pub slot: u8,
    pub units: Option<String>,
    pub quality: DataQuality,
}

pub enum DataQuality {
    Good,
    Saturated,
    NoSignal,
    Invalid,
}
```

#### NamedChainExperimentData

```rust
pub struct NamedChainExperimentData {
    pub base_data: ChainExperimentData,
    pub signal_metadata: Vec<SignalMetadata>,
    pub mapping_report: SignalMappingReport,
    pub timing_analysis: TimingAnalysis,
}

impl NamedChainExperimentData {
    // Name-based data access
    pub fn get_signal_data(&self, signal_name: &str) -> Result<Vec<(f32, f64)>, NanonisError>;
    pub fn get_signal_data_during_action(&self, signal_name: &str, action_index: usize) -> Result<Vec<(f32, f64)>, NanonisError>;
    
    // Advanced plotting
    pub fn plot_signals_by_name(&self, signal_names: &[&str]) -> Result<(), NanonisError>;
    pub fn auto_plot_with_units(&self) -> Result<(), NanonisError>;
    pub fn plot_action_correlation(&self, signal_name: &str) -> Result<(), NanonisError>;
    
    // Analysis
    pub fn get_signal_statistics(&self, signal_name: &str) -> Result<SignalStatistics, NanonisError>;
    pub fn detect_signal_anomalies(&self) -> Vec<SignalAnomaly>;
}

pub struct SignalStatistics {
    pub mean: f64,
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
    pub range: f64,
    pub sample_count: usize,
    pub data_quality: f32,  // Percentage of good data points
}
```

### 4. Configuration System

#### Default Signal Slot Mapping

Since RT signal queries are not currently available, we use configurable defaults:

```rust
pub fn default_logger_slot_mapping() -> HashMap<u8, &'static str> {
    [
        (0, "Bias (V)"),
        (1, "Current (A)"),
        (2, "Z (m)"),
        (3, "X (m)"),
        (4, "Y (m)"),
        (5, "Phase (deg)"),
        (6, "Amplitude (m)"),
        (7, "Frequency Shift (Hz)"),
        (8, "Excitation (V)"),
        (9, "LI X (V)"),
        (10, "LI Y (V)"),
        (11, "LI R (V)"),
        (12, "LI Phase (deg)"),
        // ... additional common signals
    ].into_iter().collect()
}
```

#### JSON Configuration File

```json
{
  "signal_slot_mapping": {
    "0": "Bias (V)",
    "1": "Current (A)", 
    "2": "Z (m)",
    "3": "X (m)",
    "4": "Y (m)"
  },
  "signal_aliases": {
    "bias": "Bias (V)",
    "current": "Current (A)",
    "z": "Z (m)",
    "position": "Z (m)",
    "height": "Z (m)"
  },
  "fuzzy_matching": {
    "enabled": true,
    "threshold": 0.7
  },
  "units": {
    "Bias (V)": "V",
    "Current (A)": "nA",
    "Z (m)": "nm"
  }
}
```

## Usage Examples

### Basic Named Signal Collection

```rust
// ✅ New approach - robust and user-friendly
use rusty_tip::{ActionDriver, SignalManager};

// Initialize signal management
let mut signal_manager = SignalManager::new(client.clone())?;
signal_manager.refresh_full_registry()?;

// Execute with human-readable signal names
let experiment_data = driver.execute_chain_with_named_signals(
    tip_shaping_actions,
    &["Bias (V)", "Current (A)", "Z Position"],  // Clear, readable names
    Duration::from_millis(200),
    Duration::from_millis(1000),
)?;

// Automatic validation and error reporting:
// ✅ "Bias (V)" -> Found in slot 0 (index 24)
// ✅ "Current (A)" -> Found in slot 1 (index 0)  
// ⚠️  "Z Position" -> Not found, did you mean "Z (m)" in slot 2?
```

### Advanced Signal Analysis

```rust
// Plot specific signals with automatic unit scaling
experiment_data.plot_signals_by_name(&["Bias (V)", "Current (A)"])?;

// Get detailed statistics for a signal
let bias_stats = experiment_data.get_signal_statistics("Bias (V)")?;
println!("Bias voltage: {:.3} ± {:.3} V", bias_stats.mean, bias_stats.std_dev);

// Analyze signal during specific actions
let pulse_current = experiment_data.get_signal_data_during_action("Current (A)", 1)?;  // Action 1 = PulseRetract
println!("Current during pulse: {} data points", pulse_current.len());

// Detect anomalies automatically
let anomalies = experiment_data.detect_signal_anomalies();
for anomaly in anomalies {
    println!("⚠️  Anomaly in {}: {} at {:.3}s", 
             anomaly.signal_name, anomaly.description, anomaly.timestamp);
}
```

### Fuzzy Signal Matching

```rust
// Smart signal resolution with fuzzy matching
let experiment_data = driver.execute_chain_with_named_signals(
    actions,
    &["bias", "current", "z pos"],  // Fuzzy/partial names
    pre_duration,
    post_duration,
)?;

// System automatically resolves:
// "bias" -> "Bias (V)" (exact match)
// "current" -> "Current (A)" (exact match)  
// "z pos" -> "Z (m)" (fuzzy match, 85% confidence)
```

### Custom Configuration

```rust
// Load custom signal mapping
let config = SignalManagerConfig::from_file("my_setup.json")?;
let signal_manager = SignalManager::with_config(client.clone(), config)?;

// Override specific mappings
signal_manager.set_slot_mapping(vec![
    (0, "My Custom Bias"),
    (1, "My Current Signal"),
    (2, "Z Scanner"),
])?;
```

## Implementation Phases

### Phase 1: Core Infrastructure ✅ Ready
- [x] Protocol `*+c` parsing support (completed)
- [ ] SignalManager implementation
- [ ] Basic slot mapping configuration
- [ ] Signal name normalization and fuzzy matching

### Phase 2: BufferedTCPReader Enhancement
- [ ] Signal validation in constructor
- [ ] EnrichedSignalFrame with metadata
- [ ] Name-based data query methods
- [ ] SignalMappingReport generation

### Phase 3: Named Data Collection
- [ ] NamedChainExperimentData structure
- [ ] execute_chain_with_named_signals method
- [ ] Signal-aware plotting functions
- [ ] Statistical analysis methods

### Phase 4: Advanced Features
- [ ] JSON configuration file support
- [ ] Anomaly detection algorithms
- [ ] Multi-signal correlation analysis
- [ ] Real-time signal validation

### Phase 5: Documentation and Examples
- [ ] Update tip_shaper example with named signals
- [ ] Create comprehensive signal analysis example
- [ ] Performance benchmarking documentation
- [ ] Migration guide for existing code

## Migration Strategy

### Step 1: Gradual Adoption
Maintain backwards compatibility while adding new features:

```rust
// Old API still works
let experiment_data = driver.execute_chain_with_data_collection(actions, pre, post)?;

// New API provides enhanced functionality  
let named_data = driver.execute_chain_with_named_signals(actions, signals, pre, post)?;
```

### Step 2: Deprecation Warnings
```rust
#[deprecated(since = "0.3.0", note = "Use execute_chain_with_named_signals instead")]
pub fn execute_chain_with_data_collection(...) -> Result<ChainExperimentData, NanonisError> {
    // Implementation with deprecation warning
}
```

### Step 3: Migration Tools
```rust
impl ChainExperimentData {
    // Convert old data to new format
    pub fn to_named_data(&self, signal_names: &[String]) -> NamedChainExperimentData;
}
```

## Performance Considerations

### Caching Strategy
- **Signal registry cache**: TTL-based, refreshed every 5 minutes
- **Slot mapping cache**: Persistent until manual refresh
- **Name normalization cache**: In-memory HashMap for O(1) lookups

### Memory Usage
- **Minimal overhead**: ~1KB per 1000 signal names in cache
- **Efficient storage**: Signal metadata stored once, referenced by frames
- **Configurable limits**: Maximum cache size and age limits

### Latency Impact
- **Name resolution**: <1ms for cached signals
- **Validation**: <5ms for 24 signal validation
- **Fuzzy matching**: <10ms for similarity search across 127 signals

## Future Enhancements

### RT Signal Query Integration
When `Signals.AddRTGet` becomes available:
```rust
impl SignalManager {
    pub fn refresh_rt_assignments(&mut self) -> Result<(), NanonisError> {
        // Query actual RT signal assignments
        // Replace hardcoded mapping with dynamic discovery
    }
}
```

### Machine Learning Integration
```rust
impl NamedChainExperimentData {
    pub fn predict_signal_behavior(&self, signal_name: &str) -> Result<SignalPrediction, NanonisError>;
    pub fn classify_experiment_type(&self) -> ExperimentType;
    pub fn recommend_signal_analysis(&self) -> Vec<AnalysisRecommendation>;
}
```

### Advanced Analysis Features
- **Cross-signal correlation analysis**
- **Frequency domain analysis** 
- **Real-time anomaly detection**
- **Automated experiment characterization**

## Error Handling

### Common Error Scenarios

```rust
// Signal not found
Err(SignalNotFound { 
    requested: "Bais (V)", 
    suggestions: vec!["Bias (V)"] 
})

// Signal not in logger slots
Err(SignalNotAvailableInLogger { 
    signal: "Internal PLL Phase", 
    available_slots: vec![0, 1, 2] 
})

// Ambiguous signal name
Err(AmbiguousSignal { 
    requested: "Current", 
    matches: vec!["Current (A)", "Current Setpoint (A)"] 
})
```

### Error Recovery
- **Automatic suggestions** for typos and partial names
- **Graceful degradation** when signals are missing
- **Detailed error context** with available alternatives

## Conclusion

This unified signal management system transforms the user experience from error-prone index manipulation to intuitive name-based signal handling. By providing automatic validation, fuzzy matching, and comprehensive error reporting, it significantly reduces the complexity of SPM data collection and analysis.

The architecture is designed to be:
- **Immediately useful** with hardcoded mappings
- **Flexible** for different experimental setups  
- **Extensible** for future RT signal query integration
- **Performance-conscious** for high-frequency data collection

Implementation can proceed incrementally, maintaining full backwards compatibility while adding powerful new capabilities for modern SPM experimental workflows.