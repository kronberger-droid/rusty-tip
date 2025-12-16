/// Nanonis signal index (0-127)
///
/// Represents a signal index as used in the Nanonis TCP protocol.
/// This is the low-level protocol type, separate from rusty-tip's high-level Signal type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SignalIndex(u8);

impl SignalIndex {
    /// Create a new SignalIndex
    pub fn new(index: u8) -> Self {
        Self(index)
    }

    /// Get the raw index value
    pub fn get(&self) -> u8 {
        self.0
    }
}

// Conversion for protocol usage (i32 is what Nanonis protocol uses)
impl From<SignalIndex> for i32 {
    fn from(idx: SignalIndex) -> i32 {
        idx.0 as i32
    }
}

// Allow creating from u8
impl From<u8> for SignalIndex {
    fn from(index: u8) -> Self {
        SignalIndex(index)
    }
}
