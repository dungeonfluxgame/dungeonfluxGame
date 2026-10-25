//! Secret tokens, their stored digests, identifiers and keyed hashes.
//!
//! Secret types implement neither `Debug` nor `Display`, so a token, password hash or key cannot
//! reach a log line by accident. Only digests are stored; comparisons are constant-time.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use df_types::{AccountId, OperationId, WebSessionId};

type HmacSha256 = Hmac<Sha256>;

/// The operating system's random source failed; nothing secret was issued.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RandomUnavailable;

pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N], RandomUnavailable> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_| RandomUnavailable)?;
    Ok(bytes)
}

/// Sixteen random bytes forced nonzero, for the opaque role identities.
fn random_identity() -> Result<[u8; 16], RandomUnavailable> {
    let mut bytes = random_bytes::<16>()?;
    // A zero identity is invalid; the chance is 2^-128, and setting one bit keeps it impossible.
    bytes[15] |= 1;
    Ok(bytes)
}

pub(crate) fn new_account_id() -> Result<AccountId, RandomUnavailable> {
    let bytes = random_identity()?;
    AccountId::from_bytes(&bytes).map_err(|_| RandomUnavailable)
}

pub(crate) fn new_session_id() -> Result<WebSessionId, RandomUnavailable> {
    let bytes = random_identity()?;
    WebSessionId::from_bytes(&bytes).map_err(|_| RandomUnavailable)
}

pub(crate) fn new_record_id() -> Result<[u8; 16], RandomUnavailable> {
    random_identity()
}

/// A form's operation id: the issue time in the first eight bytes (big-endian milliseconds) and
/// eight random bytes. The time lets an old id be refused once its receipt has been retired.
pub fn new_operation_id(now_ms: u64) -> Result<OperationId, RandomUnavailable> {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&now_ms.to_be_bytes());
    bytes[8..].copy_from_slice(&random_bytes::<8>()?);
    bytes[15] |= 1;
    OperationId::from_bytes(&bytes).map_err(|_| RandomUnavailable)
}

/// The issue time an operation id carries.
pub fn operation_issued_at_ms(operation: &OperationId) -> u64 {
    let mut time = [0_u8; 8];
    time.copy_from_slice(&operation.as_bytes()[..8]);
    u64::from_be_bytes(time)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
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
    for (index, [high, low]) in encoded.as_chunks::<2>().0.iter().enumerate() {
        bytes[index] = (nibble(*high)? << 4) | nibble(*low)?;
    }
    Some(bytes)
}

/// SHA-256 of a token: what stores keep and look up.
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct TokenDigest([u8; 32]);

impl TokenDigest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A 32-byte bearer secret (session cookie, emailed link). Text form is 64 lowercase hex digits.
pub struct SecretToken([u8; 32]);

impl SecretToken {
    pub fn generate() -> Result<Self, RandomUnavailable> {
        random_bytes::<32>().map(Self)
    }

    /// Parses the text form; anything else is `None` and is treated like an unknown token.
    pub fn parse(text: &str) -> Option<Self> {
        parse_hex::<32>(text).map(Self)
    }

    pub fn text(&self) -> String {
        hex(&self.0)
    }

    pub fn digest(&self) -> TokenDigest {
        TokenDigest(Sha256::digest(self.0).into())
    }
}

/// Per-session (or per pre-session) key for CSRF tokens.
#[derive(Clone)]
pub struct CsrfSecret([u8; 32]);

impl CsrfSecret {
    pub fn generate() -> Result<Self, RandomUnavailable> {
        random_bytes::<32>().map(Self)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The text form carried by the pre-session cookie (64 lowercase hex digits).
    pub fn text(&self) -> String {
        hex(&self.0)
    }

    pub fn parse(text: &str) -> Option<Self> {
        parse_hex::<32>(text).map(Self)
    }

    /// The token a form named `form` must carry: HMAC-SHA256 of the form name, in hex.
    pub fn form_token(&self, form: &str) -> String {
        hex(&keyed_digest(&self.0, form.as_bytes()))
    }

    /// Constant-time check of a submitted form token.
    pub fn accepts(&self, form: &str, submitted: &str) -> bool {
        let Some(submitted) = parse_hex::<32>(submitted) else {
            return false;
        };
        keyed_digest(&self.0, form.as_bytes())
            .ct_eq(&submitted)
            .into()
    }
}

/// HMAC-SHA256 of `data` under `key`.
pub(crate) fn keyed_digest(key: &[u8], data: &[u8]) -> [u8; 32] {
    // HMAC accepts keys of any length, so construction cannot fail.
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key)
        .unwrap_or_else(|_| unreachable!("HMAC takes keys of any length"));
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// SHA-256 of `data` behind a fixed domain tag, for nonsecret principals such as a signup
/// address's receipt key.
pub(crate) fn tagged_digest(tag: &str, data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(tag.as_bytes());
    hasher.update([0]);
    hasher.update(data);
    hasher.finalize().into()
}

/// An Argon2id PHC string. Never logged, never shown.
#[derive(Clone)]
pub struct PasswordHash(String);

impl PasswordHash {
    pub fn from_phc(phc: String) -> Self {
        Self(phc)
    }

    pub fn phc(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_round_trips_through_its_text_form_and_digests_stably() {
        let token = SecretToken::generate().unwrap();
        let text = token.text();
        assert_eq!(text.len(), 64);
        let parsed = SecretToken::parse(&text).unwrap();
        assert!(parsed.digest() == token.digest());
        assert!(SecretToken::parse(&text.to_ascii_uppercase()).is_none());
        assert!(SecretToken::parse("abc").is_none());
    }

    #[test]
    fn a_form_token_is_accepted_only_for_its_own_form_and_secret() {
        let secret = CsrfSecret::from_bytes([7; 32]);
        let token = secret.form_token("login");
        assert!(secret.accepts("login", &token));
        assert!(!secret.accepts("signup", &token));
        assert!(!CsrfSecret::from_bytes([8; 32]).accepts("login", &token));
        assert!(!secret.accepts("login", "not hex"));
    }

    #[test]
    fn an_operation_id_carries_its_issue_time() {
        let operation = new_operation_id(1_234_567).unwrap();
        assert_eq!(operation_issued_at_ms(&operation), 1_234_567);
    }
}
