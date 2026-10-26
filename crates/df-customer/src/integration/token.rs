//! PAT wire format and versioned peppers (integration-api-and-embeds-research 2.2).
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{
    ServiceError,
    tokens::{hex, keyed_digest, random_bytes},
};

/// Opaque public lookup id, never the bearer secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenId(pub [u8; 16]);

impl TokenId {
    pub fn text(self) -> String {
        hex(&self.0)
    }

    pub fn parse(text: &str) -> Option<Self> {
        if text.len() != 32
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        let mut bytes = [0; 16];
        for (out, pair) in bytes.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
            *out = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        }
        Some(Self(bytes))
    }
}

/// Secret-bearing types deliberately have no Debug, Display or serde implementation.
pub struct PersonalToken {
    pub id: TokenId,
    secret: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenParseRefusal { Shape, SecretEncoding, Checksum }

impl PersonalToken {
    pub fn generate() -> Result<Self, ServiceError> {
        Ok(Self {
            id: TokenId(random_bytes()?),
            secret: random_bytes()?,
        })
    }

    pub fn parse(text: &str) -> Result<Self,TokenParseRefusal> {
        // Fixed byte offsets: base64url itself contains underscores.
        if text.len() != 95 || !text.is_ascii() || !text.starts_with("df_pat_v1_") {
            return Err(TokenParseRefusal::Shape);
        }
        if text.as_bytes()[42] != b'_' || text.as_bytes()[86] != b'_' {
            return Err(TokenParseRefusal::Shape);
        }
        let id = TokenId::parse(&text[10..42]).ok_or(TokenParseRefusal::Shape)?;
        let secret: [u8; 32] = URL_SAFE_NO_PAD
            .decode(&text[43..86])
            .map_err(|_|TokenParseRefusal::SecretEncoding)?
            .try_into()
            .map_err(|_|TokenParseRefusal::SecretEncoding)?;
        let check = hex(&Sha256::digest(secret)[..4]);
        // The complete format is 10 + 32 + 1 + 43 + 1 + 8 bytes.
        if !bool::from(check.as_bytes().ct_eq(text[87..].as_bytes())) {
            return Err(TokenParseRefusal::Checksum);
        }
        Ok(Self { id, secret })
    }

    pub fn reveal(&self) -> String {
        format!(
            "df_pat_v1_{}_{}_{}",
            self.id.text(),
            URL_SAFE_NO_PAD.encode(self.secret),
            hex(&Sha256::digest(self.secret)[..4])
        )
    }
}

/// Active plus at most one retiring pepper; key material never enters the store.
pub struct Peppers {
    active: Pepper,
    retiring: Option<Pepper>,
}
struct Pepper {
    id: u32,
    key: [u8; 32],
    expires_at_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PepperRefusal {
    DuplicateId,
    EmptyKey,
}

impl Peppers {
    pub fn new(
        id: u32,
        key: [u8; 32],
        retiring: Option<(u32, [u8; 32], u64)>,
    ) -> Result<Self, PepperRefusal> {
        if key == [0; 32] || retiring.is_some_and(|(_, key, _)| key == [0; 32]) {
            return Err(PepperRefusal::EmptyKey);
        }
        if retiring.is_some_and(|(old, _, _)| old == id) {
            return Err(PepperRefusal::DuplicateId);
        }
        Ok(Self {
            active: Pepper {
                id,
                key,
                expires_at_ms: None,
            },
            retiring: retiring.map(|(id, key, expires)| Pepper {
                id,
                key,
                expires_at_ms: Some(expires),
            }),
        })
    }

    pub fn active_id(&self) -> u32 {
        self.active.id
    }

    pub fn digest(&self, token: &PersonalToken) -> [u8; 32] {
        keyed_digest(&self.active.key, &token.secret)
    }

    pub fn accepts(
        &self,
        token: &PersonalToken,
        key_id: u32,
        digest: &[u8; 32],
        now_ms: u64,
    ) -> bool {
        let pepper = if self.active.id == key_id {
            Some(&self.active)
        } else {
            self.retiring.as_ref().filter(|p| p.id == key_id)
        };
        pepper
            .filter(|p| p.expires_at_ms.is_none_or(|expiry| now_ms < expiry))
            .is_some_and(|p| bool::from(keyed_digest(&p.key, &token.secret).ct_eq(digest)))
    }

    /// Cursors contain only an opaque row id and expiry. The MAC binds their owner and route.
    pub fn cursor(&self, account: &[u8; 16], endpoint: &str, row: TokenId, now_ms: u64) -> String {
        let body = format!("{}.{}", now_ms.saturating_add(super::DAY_MS), row.text());
        let context = format!("df_cursor_v1:{}:{endpoint}:{body}", hex(account));
        format!(
            "{body}.{}",
            hex(&keyed_digest(&self.active.key, context.as_bytes()))
        )
    }

    pub fn cursor_row(
        &self,
        account: &[u8; 16],
        endpoint: &str,
        cursor: &str,
        now_ms: u64,
    ) -> Option<TokenId> {
        if cursor.len() > 128 || !cursor.is_ascii() {
            return None;
        }
        let (body, mac) = cursor.rsplit_once('.')?;
        let (expires, row) = body.split_once('.')?;
        let expires: u64 = expires.parse().ok()?;
        if expires <= now_ms || expires > now_ms.saturating_add(super::DAY_MS) {
            return None;
        }
        let context = format!("df_cursor_v1:{}:{endpoint}:{body}", hex(account));
        let active=bool::from(
            hex(&keyed_digest(&self.active.key, context.as_bytes()))
                .as_bytes()
                .ct_eq(mac.as_bytes()),
        );
        let retiring=self.retiring.as_ref().filter(|p|p.expires_at_ms.is_some_and(|expires|now_ms<expires)).is_some_and(|p|bool::from(hex(&keyed_digest(&p.key,context.as_bytes())).as_bytes().ct_eq(mac.as_bytes())));
        if !(active | retiring) {
            return None;
        }
        TokenId::parse(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pat_round_trip_rejects_malformed_and_checksum_changes() {
        let token = PersonalToken::generate().unwrap();
        let text = token.reveal();
        assert_eq!(text.len(), 95);
        assert!(PersonalToken::parse(&text).is_ok());
        let mut changed = text.into_bytes();
        changed[94] = if changed[94] == b'0' { b'1' } else { b'0' };
        assert!(PersonalToken::parse(std::str::from_utf8(&changed).unwrap()).is_err());
        assert!(PersonalToken::parse("df_pat_v1_short").is_err());
    }
    #[test]
    fn pat_pepper_rotation_accepts_only_configured_keys() {
        let token = PersonalToken::generate().unwrap();
        let old = Peppers::new(1, [1; 32], None).unwrap();
        let new = Peppers::new(2, [2; 32], Some((1, [1; 32], 100))).unwrap();
        assert!(new.accepts(&token, 1, &old.digest(&token), 0));
        assert!(!new.accepts(&token, 2, &old.digest(&token), 0));
        assert!(!new.accepts(&token, 3, &old.digest(&token), 0));
        assert!(!new.accepts(&token, 1, &old.digest(&token), 100));
    }
}
