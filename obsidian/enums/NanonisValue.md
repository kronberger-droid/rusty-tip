# NanonisValue

**Description**: Protocol data types for all Nanonis communication with automatic conversion.

**Implementation**: 
```rust
#[derive(Debug, Clone)]
pub enum NanonisValue {
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    F32(f32),
    F64(f64),
    String(String),
    ArrayU16(Vec<u16>),
    ArrayI16(Vec<i16>),
    ArrayU32(Vec<u32>),
    ArrayI32(Vec<i32>),
    ArrayF32(Vec<f32>),
    ArrayF64(Vec<f64>),
    ArrayString(Vec<String>),
    Array2DF32(Vec<Vec<f32>>),
}
```

**Notes**: 
- Central type system for [[NanonisClient]] protocol communication
- Includes From/Into conversion traits for all basic types
- Replaces need for separate BiasVoltage/Position wrapper types