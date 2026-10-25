//! Private loopback launch protocol. Identifiers locate; only the shared secret authorizes.
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::Placement;

pub const ALLOCATION_PATH: &str = "/gameplay/internal/allocate";
pub const ENTRY_PATH: &str = "/gameplay/internal/entry";
pub const SECRET_HEADER: &str = "x-df-runtime-launch";
pub const ENTRY_HEADER: &str = "x-df-runtime-entry";
pub const ENTRY_LIFETIME_SECONDS: u64 = 30;
const MAXIMUM_TOKEN_BYTES: usize = 2048;
type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seat {
    pub account: [u8; 16],
    pub member: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allocation {
    pub session: [u8; 16],
    pub campaign: [u8; 16],
    pub roster: Vec<Seat>,
    /// Opaque runtime display tickets. The portal verifies S1 devices before signing entry.
    pub display_tickets: Vec<[u8; 16]>,
}

impl From<&Placement> for Allocation {
    fn from(placement: &Placement) -> Self {
        Self {
            session: *placement.session.as_bytes(),
            campaign: *placement.campaign.as_bytes(),
            roster: placement
                .roster
                .iter()
                .map(|seat| Seat {
                    account: *seat.account.as_bytes(),
                    member: *seat.member.as_bytes(),
                })
                .collect(),
            // The trusted runtime adapter supplies its own display exchange ticket.
            display_tickets: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allocated {
    pub code: String,
    pub generation: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    Occupied,
    InvalidPlacement,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "principal", rename_all = "snake_case")]
pub enum EntryAudience {
    Player([u8; 16]),
    Display([u8; 16]),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub session: [u8; 16],
    pub campaign: [u8; 16],
    pub audience: EntryAudience,
    pub generation: String,
    pub expires_at_unix_seconds: u64,
}

/// No Debug/Display: callers must never include this value in diagnostics.
#[derive(Clone)]
pub struct LaunchSecret(Vec<u8>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidSecret;

impl LaunchSecret {
    pub fn new(secret: &str) -> Result<Self, InvalidSecret> {
        if !(32..=256).contains(&secret.len())
            || !secret.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(InvalidSecret);
        }
        Ok(Self(secret.as_bytes().to_vec()))
    }

    pub fn header_value(&self) -> &[u8] {
        &self.0
    }

    fn mac(&self, domain: &[u8], value: &[u8]) -> HmacSha256 {
        // HMAC accepts every key length; construction is infallible.
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(&self.0)
            .unwrap_or_else(|_| unreachable!("HMAC accepts every key length"));
        mac.update(domain);
        mac.update(&[0]);
        mac.update(value);
        mac
    }

    pub fn authenticates(&self, offered: &[u8]) -> bool {
        let expected = self
            .mac(b"launch header v1", &self.0)
            .finalize()
            .into_bytes();
        self.mac(b"launch header v1", offered)
            .verify_slice(&expected)
            .is_ok()
    }

    pub fn derive(&self, domain: &[u8], value: &[u8]) -> [u8; 32] {
        self.mac(domain, value).finalize().into_bytes().into()
    }

    pub fn sign(&self, entry: &Entry) -> Option<String> {
        let bytes = serde_json::to_vec(entry).ok()?;
        let signature = self.derive(b"campaign entry v1", &bytes);
        Some(format!(
            "{}.{}",
            crate::secrets::hex(&bytes),
            crate::secrets::hex(&signature)
        ))
    }

    pub fn verify(&self, token: &str, now_unix_seconds: u64) -> Option<Entry> {
        if token.len() > MAXIMUM_TOKEN_BYTES {
            return None;
        }
        let (message, signature) = token.split_once('.')?;
        let bytes = crate::secrets::unhex(message)?;
        let signature = crate::secrets::unhex(signature)?;
        self.mac(b"campaign entry v1", &bytes)
            .verify_slice(&signature)
            .ok()?;
        let entry: Entry = serde_json::from_slice(&bytes).ok()?;
        let remaining = entry
            .expires_at_unix_seconds
            .checked_sub(now_unix_seconds)?;
        (remaining > 0 && remaining <= ENTRY_LIFETIME_SECONDS).then_some(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_entry_refuses_tampering_expiry_and_a_different_secret() {
        let key = LaunchSecret::new(&"a".repeat(32)).unwrap();
        let other = LaunchSecret::new(&"b".repeat(32)).unwrap();
        let entry = Entry {
            session: [1; 16],
            campaign: [2; 16],
            audience: EntryAudience::Player([3; 16]),
            generation: "generation".into(),
            expires_at_unix_seconds: 130,
        };
        let token = key.sign(&entry).unwrap();
        assert_eq!(key.verify(&token, 100), Some(entry));
        assert!(key.verify(&token, 130).is_none());
        assert!(key.verify(&token, 99).is_none());
        assert!(other.verify(&token, 100).is_none());
        assert!(key.verify(&format!("00{token}"), 100).is_none());
        assert!(key.authenticates(key.header_value()));
        assert!(!key.authenticates(other.header_value()));
        assert!(!key.authenticates(b""));
    }
}
