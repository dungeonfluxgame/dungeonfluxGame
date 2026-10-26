//! Exact-preview share consent. This contract has no private narrative, money or media URLs.
use crate::wire_integration::SecretReveal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharePerson {
    pub account_id: String,
    pub public_name: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareDraft {
    pub title: String,
    pub highlights: Vec<String>,
    pub people: Vec<SharePerson>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharePreview {
    pub session_id: String,
    pub revision: String,
    pub draft: ShareDraft,
    pub date: String,
    pub duration_minutes: u64,
    pub consented_accounts: Vec<String>,
    pub can_publish: bool,
    pub can_consent: bool,
    pub candidates: Vec<SharePerson>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareConsent {
    pub revision: String,
    pub agrees: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateShare {
    pub revision: String,
    pub expiry_days: u16,
    pub allow_indexing: bool,
    pub allow_embeds: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareLinkView {
    pub link_id: String,
    pub expires_at_ms: u64,
    pub revoked: bool,
    pub allow_indexing: bool,
    pub allow_embeds: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareIssued {
    pub link: ShareLinkView,
    pub url: SecretReveal,
}
