//! Public link secrets: 256 random bits, base64url, checksum and digest-only storage.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub struct LinkToken([u8; 32]);
impl LinkToken {
    pub fn generate() -> Result<Self, df_customer::ServiceError> {
        let random = df_customer::SecretToken::generate()
            .map_err(|_| df_customer::ServiceError::RandomUnavailable)?;
        let text = random.text();
        let mut bytes = [0; 32];
        for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = u8::from_str_radix(
                std::str::from_utf8(pair)
                    .map_err(|_| df_customer::ServiceError::RandomUnavailable)?,
                16,
            )
            .map_err(|_| df_customer::ServiceError::RandomUnavailable)?;
        }
        Ok(Self(bytes))
    }
    pub fn text(&self) -> String {
        format!(
            "dfs_{}_{}",
            URL_SAFE_NO_PAD.encode(self.0),
            URL_SAFE_NO_PAD.encode(&Sha256::digest(self.0)[..6])
        )
    }
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() != 56
            || !text.is_ascii()
            || !text.starts_with("dfs_")
            || text.as_bytes()[47] != b'_'
        {
            return None;
        }
        let bytes: [u8; 32] = URL_SAFE_NO_PAD.decode(&text[4..47]).ok()?.try_into().ok()?;
        if !bool::from(
            URL_SAFE_NO_PAD
                .encode(&Sha256::digest(bytes)[..6])
                .as_bytes()
                .ct_eq(text[48..].as_bytes()),
        ) {
            return None;
        }
        Some(Self(bytes))
    }
    pub fn digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"df_session_share_v1\0");
        digest.update(self.0);
        digest.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn share_tokens_round_trip_and_reject_changed_checksums() {
        let token = LinkToken::generate().unwrap();
        let text = token.text();
        assert_eq!(LinkToken::parse(&text).unwrap().digest(), token.digest());
        let mut bytes = text.into_bytes();
        bytes[55] = if bytes[55] == b'A' { b'B' } else { b'A' };
        assert!(LinkToken::parse(std::str::from_utf8(&bytes).unwrap()).is_none());
    }
}
