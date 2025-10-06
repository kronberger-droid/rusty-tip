**Description**: Complete experiment result containing both action outcome and synchronized time-windowed TCP logger data.

**Implementation**: 
```rust
pub struct ExperimentData {
    pub action_result: ActionResult,
    pub tcp_data: Vec<TimestampedFrame>,
    pub action_start: Instant,
    pub action_end: Instant,
    pub total_duration: Duration,
}
```

**Notes**: 
- **Complete experiment record** - Combines action execution with full data context
- **Time-windowed data** - Contains pre/during/post action signal measurements
- **Analysis ready** - Built-in methods for filtering data by action phases
- **Export friendly** - Structured for CSV, JSON, or binary data export

**Key Methods**:
- `data_during_action()` - Get signal data captured during action execution
- `data_before_action(duration)` - Get pre-action baseline data
- `data_after_action(duration)` - Get post-action response data

**Experiment Workflow**:
1. **ActionDriver** starts buffered data collection
2. **Action execution** with precise start/end timing
3. **Data query** extracts relevant time window from buffer
4. **ExperimentData** packages everything for analysis

**Analysis Capabilities**:
- **Signal baselines** - Pre-action steady-state measurements
- **Action response** - Real-time signal changes during execution  
- **Recovery dynamics** - Post-action signal evolution
- **Timing precision** - Correlate signal changes with exact action timing

**Use Cases**:
- **Bias pulse spectroscopy** - Voltage pulse + current response
- **Approach curves** - Z-movement + tunneling current
- **Tip shaping** - Voltage pulses + conductance evolution
- **General SPM experiments** - Any action requiring signal correlation

**Relationships**:
- Contains [[ActionResult]] from executed action
- Contains vector of [[TimestampedFrame]] from time window
- Produced by [[ActionDriver]] execute_with_data_collection methods
- Used for experiment analysis, plotting, and data export