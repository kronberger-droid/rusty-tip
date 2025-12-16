/// Nanonis signal index (0-127) for TCP protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SignalIndex(u8);

impl SignalIndex {
    pub fn new(index: u8) -> Self {
        Self(index)
    }

    pub fn get(&self) -> u8 {
        self.0
    }
}

impl From<SignalIndex> for i32 {
    fn from(idx: SignalIndex) -> i32 {
        idx.0 as i32
    }
}

impl From<u8> for SignalIndex {
    fn from(index: u8) -> Self {
        SignalIndex(index)
    }
}
