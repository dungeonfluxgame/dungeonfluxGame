//! A deterministic 64-bit digest for the scene builder's identities.
//!
//! It owns `PlateDigest`, an FNV-1a walk that is the same in every run (no random key, no clock,
//! no address). The plate scheduler digests the facts a picture depends on, so a tick can tell
//! "nothing visible changed" (RES9), and the host digests the inputs of a stage request into its
//! `RequestKey` (H.8). It is not a security boundary and never reads the time.

const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over a fixed walk of values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlateDigest(u64);

impl Default for PlateDigest {
    fn default() -> Self {
        Self::new()
    }
}

impl PlateDigest {
    pub fn new() -> Self {
        Self(OFFSET_BASIS)
    }

    pub fn finish(self) -> u64 {
        self.0
    }

    pub fn byte(&mut self, byte: u8) {
        self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(PRIME);
    }

    pub fn number(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.byte(byte);
        }
    }

    /// Text behind its length, so `"ab", "c"` and `"a", "bc"` never collide.
    pub fn text(&mut self, text: &str) {
        self.number(u64::try_from(text.len()).unwrap_or(u64::MAX));
        for byte in text.bytes() {
            self.byte(byte);
        }
    }

    /// A tag byte then the value, so `None` and `Some` never collide.
    pub fn option<T>(&mut self, value: Option<T>, write: impl FnOnce(&mut Self, T)) {
        match value {
            Some(value) => {
                self.byte(1);
                write(self, value);
            }
            None => self.byte(0),
        }
    }
}
