//! Opaque 128-bit identities of the media catalog (planning/asset-storage-research.md 3.3, 6.6).
//! Values are never generated here: an `IdSource` supplied by composition mints them, so tests are
//! deterministic and production draws operating-system entropy at the edge.

use std::fmt;

/// Why bytes or text were refused as an identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaIdError {
    InvalidLength { actual: usize },
    Zero,
    NotLowercaseHex,
}

macro_rules! media_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            /// Accepts exactly 16 bytes with at least one nonzero byte.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, MediaIdError> {
                nonzero_id(bytes).map(Self)
            }

            /// Parses exactly 32 lowercase hexadecimal digits.
            pub fn from_hex(text: &str) -> Result<Self, MediaIdError> {
                parse_hex_id(text).map(Self)
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }

            /// Lowercase hexadecimal, the only form used in object keys and audit lines.
            pub fn to_hex(&self) -> String {
                hex(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.to_hex())
            }
        }
    };
}

media_id!(
    /// One catalog asset row (random; never derived from content).
    AssetId
);
media_id!(
    /// One admission of work for an asset; the same value in F's `ProviderAttempt` and the hub (D22).
    AttemptId
);
media_id!(
    /// One object write; keeps two writers of one attempt from ever sharing a key.
    WriteId
);
media_id!(
    /// A step-lease claim; fresh on every claim, so an older claim is always distinguishable.
    ClaimToken
);
media_id!(
    /// One upload ticket.
    UploadId
);
media_id!(
    /// One export archive.
    ExportId
);
media_id!(
    /// One personal character template. TODO(O2-06): move to df-types when its lock frees.
    CharacterTemplateId
);

/// Mints identities. Composition supplies operating-system entropy; tests supply a sequence.
pub trait IdSource: Send + Sync {
    /// Sixteen bytes with at least one nonzero byte.
    fn next_id_bytes(&self) -> [u8; 16];
}

/// Mints any media id from the source; a zero draw is redrawn, never returned.
pub fn mint<T>(source: &dyn IdSource, from: fn(&[u8]) -> Result<T, MediaIdError>) -> T {
    loop {
        if let Ok(id) = from(&source.next_id_bytes()) {
            return id;
        }
    }
}

fn nonzero_id(bytes: &[u8]) -> Result<[u8; 16], MediaIdError> {
    let value: [u8; 16] = bytes.try_into().map_err(|_| MediaIdError::InvalidLength {
        actual: bytes.len(),
    })?;
    if value == [0; 16] {
        return Err(MediaIdError::Zero);
    }
    Ok(value)
}

fn parse_hex_id(text: &str) -> Result<[u8; 16], MediaIdError> {
    if text.len() != 32 {
        return Err(MediaIdError::InvalidLength { actual: text.len() });
    }
    let mut value = [0_u8; 16];
    for (index, pair) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let high = hex_digit(pair[0]).ok_or(MediaIdError::NotLowercaseHex)?;
        let low = hex_digit(pair[1]).ok_or(MediaIdError::NotLowercaseHex)?;
        value[index] = (high << 4) | low;
    }
    nonzero_id(&value)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Lowercase hexadecimal of any byte string.
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// A deterministic source for tests: 1, 2, 3, ... in the last eight bytes.
#[cfg(any(test, feature = "test-fakes"))]
pub struct SequentialIds(std::sync::atomic::AtomicU64);

#[cfg(any(test, feature = "test-fakes"))]
impl SequentialIds {
    pub fn new() -> Self {
        Self(std::sync::atomic::AtomicU64::new(0))
    }
}

#[cfg(any(test, feature = "test-fakes"))]
impl Default for SequentialIds {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-fakes"))]
impl IdSource for SequentialIds {
    fn next_id_bytes(&self) -> [u8; 16] {
        // Relaxed: only uniqueness of the counter matters, no other memory is published through it.
        let next = self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let mut bytes = [0_u8; 16];
        bytes[8..].copy_from_slice(&next.to_be_bytes());
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_round_trips_through_lowercase_hex() {
        let id = AssetId::from_bytes(&[0xab; 16]).unwrap();
        assert_eq!(id.to_hex(), "ab".repeat(16));
        assert_eq!(AssetId::from_hex(&id.to_hex()).unwrap(), id);
    }

    #[test]
    fn zero_short_and_uppercase_ids_are_refused() {
        assert_eq!(AssetId::from_bytes(&[0; 16]), Err(MediaIdError::Zero));
        assert_eq!(
            AssetId::from_bytes(&[1; 15]),
            Err(MediaIdError::InvalidLength { actual: 15 })
        );
        assert_eq!(
            AssetId::from_hex(&"AB".repeat(16)),
            Err(MediaIdError::NotLowercaseHex)
        );
        assert_eq!(AssetId::from_hex(&"0".repeat(32)), Err(MediaIdError::Zero));
    }

    #[test]
    fn the_sequential_source_never_repeats() {
        let ids = SequentialIds::new();
        let first = mint(&ids, AttemptId::from_bytes);
        let second = mint(&ids, AttemptId::from_bytes);
        assert_ne!(first, second);
    }
}
