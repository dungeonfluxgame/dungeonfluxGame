//! Shared first-party PAT lifecycle; bearer API envelopes stay in the portal.
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretReveal(pub String);
impl std::fmt::Debug for SecretReveal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalTokenView {
    pub token_id: String,
    pub label: String,
    pub scopes: Vec<String>,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub revoked_at_ms: Option<u64>,
    pub last_used_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePersonalToken {
    pub label: String,
    pub scopes: Vec<String>,
    pub expiry_days: u16,
    pub password: Option<SecretReveal>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalTokenIssued {
    pub token: PersonalTokenView,
    pub secret: SecretReveal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokePersonalToken {
    pub password: Option<SecretReveal>,
}
