# BiasVoltage

**Description**: Type-safe wrapper for bias voltage values with validation.

**Implementation**: 
```rust
pub struct BiasVoltage(pub f32);  // Voltage in Volts

impl BiasVoltage {
    pub fn new(voltage: f32) -> Result<Self, ValueError>;  // Range validation
}
```

**Notes**: 
- Type-safe wrapper prevents unit confusion (voltages vs coordinates)
- Used by [[NanonisClient]] for bias control methods
- Includes automatic range validation (typically ±10V)