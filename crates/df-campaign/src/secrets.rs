//! Random identities, bearer tokens and their stored digests.
//!
//! TODO(S1-01): these helpers move into `df-types` (request B-2) and `df-customer` re-exports them; until
//! then this module is the single copy this crate uses. Secret types implement neither `Debug` nor
//! `Display`, only digests are stored, and comparisons of secrets are constant-time.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// The operating system's random source failed; nothing secret was issued.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RandomUnavailable;

pub fn random_bytes<const N: usize>() -> Result<[u8; N], RandomUnavailable> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_| RandomUnavailable)?;
    Ok(bytes)
}

/// Sixteen random bytes forced nonzero, for opaque identities.
pub fn random_identity() -> Result<[u8; 16], RandomUnavailable> {
    let mut bytes = random_bytes::<16>()?;
    // A zero identity is invalid; setting one bit keeps it impossible.
    bytes[15] |= 1;
    Ok(bytes)
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

fn parse_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let encoded = text.as_bytes();
    if encoded.len() != N * 2 {
        return None;
    }
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let mut bytes = [0_u8; N];
    let (pairs, _) = encoded.as_chunks::<2>();
    for (index, pair) in pairs.iter().enumerate() {
        bytes[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(bytes)
}

/// SHA-256 of `data` behind a fixed domain tag, so a digest made for one purpose can never match
/// another's.
pub fn tagged_digest(tag: &str, data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(tag.as_bytes());
    hasher.update([0]);
    hasher.update(data);
    hasher.finalize().into()
}

/// HMAC-SHA256 of `data` under `key` and a domain tag.
pub fn keyed_digest(key: &[u8], tag: &str, data: &[u8]) -> [u8; 32] {
    // HMAC accepts keys of any length, so construction cannot fail.
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key)
        .unwrap_or_else(|_| unreachable!("HMAC takes keys of any length"));
    mac.update(tag.as_bytes());
    mac.update(&[0]);
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// A 32-byte bearer secret. Its text form is 64 lowercase hexadecimal digits.
pub struct SecretToken([u8; 32]);

impl SecretToken {
    pub fn generate() -> Result<Self, RandomUnavailable> {
        random_bytes::<32>().map(Self)
    }

    /// Parses the text form; anything else is `None` and is treated like an unknown token.
    pub fn parse(text: &str) -> Option<Self> {
        parse_hex::<32>(text.trim()).map(Self)
    }

    pub fn text(&self) -> String {
        hex(&self.0)
    }

    /// The stored digest of this token under a domain tag.
    pub fn digest(&self, tag: &str) -> [u8; 32] {
        tagged_digest(tag, &self.0)
    }
}

pub const INVITATION_TOKEN_TAG: &str = "df-campaign invitation v1";
pub const DEVICE_SECRET_TAG: &str = "df-campaign display device v1";
pub const DISPLAY_TICKET_TAG: &str = "df-campaign display ticket v1";
pub const ROOM_CODE_TAG: &str = "df-campaign room code v1";
pub const IP_KEY_TAG: &str = "df-campaign ip key v1";

/// Parses an even-length lowercase hexadecimal string into 16 bytes (for identities).
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    let encoded = text.as_bytes();
    if !encoded.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let (pairs, _) = encoded.as_chunks::<2>();
    pairs
        .iter()
        .map(|pair| Some((nibble(pair[0])? << 4) | nibble(pair[1])?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_round_trips_through_its_text_and_digests_per_domain() {
        let token = SecretToken::generate().unwrap();
        let again = SecretToken::parse(&token.text()).unwrap();
        assert_eq!(
            token.digest(INVITATION_TOKEN_TAG),
            again.digest(INVITATION_TOKEN_TAG)
        );
        assert_ne!(
            token.digest(INVITATION_TOKEN_TAG),
            token.digest(DISPLAY_TICKET_TAG)
        );
        assert_eq!(token.text().len(), 64);
    }

    #[test]
    fn malformed_token_text_is_refused() {
        assert!(SecretToken::parse("").is_none());
        assert!(SecretToken::parse(&"g".repeat(64)).is_none());
        assert!(SecretToken::parse(&"a".repeat(63)).is_none());
        assert!(SecretToken::parse(&"A".repeat(64)).is_none());
    }

    #[test]
    fn identities_are_never_zero() {
        for _ in 0..64 {
            assert_ne!(random_identity().unwrap(), [0; 16]);
        }
    }

    #[test]
    fn a_keyed_digest_depends_on_key_tag_and_data() {
        let base = keyed_digest(b"key", "tag", b"data");
        assert_ne!(base, keyed_digest(b"other", "tag", b"data"));
        assert_ne!(base, keyed_digest(b"key", "other", b"data"));
        assert_ne!(base, keyed_digest(b"key", "tag", b"other"));
        assert_eq!(base, keyed_digest(b"key", "tag", b"data"));
    }
}
