//! Typed first-party lifecycle calls; secrets are ephemeral and redact Debug output.
use super::*;
use crate::{wire_integration::*, wire_share::*};
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Call {
    Tokens,
    CreateToken(CreatePersonalToken),
    RevokeToken(String, RevokePersonalToken),
    Preview(String),
    SavePreview(String, ShareDraft),
    Consent(String, ShareConsent),
    Shares(String),
    CreateShare(String, CreateShare),
    RevokeShare(String, String),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Answer {
    Tokens(Result<Vec<PersonalTokenView>, ApiError>),
    TokenIssued(Result<PersonalTokenIssued, ApiError>),
    TokenRevoked(Result<RevokedView, ApiError>),
    Preview(Result<SharePreview, ApiError>),
    Consent(Result<RevokedView, ApiError>),
    Shares(Result<Vec<ShareLinkView>, ApiError>),
    ShareIssued(Result<ShareIssued, ApiError>),
    ShareRevoked(Result<RevokedView, ApiError>),
}
impl Call {
    pub fn request(&self) -> HttpRequest {
        let session =
            |id: &str, tail: &str| at(&format!("sessions/{}/{tail}", encode_component(id)));
        match self {
            Self::Tokens => get(at("developer-tokens")),
            Self::CreateToken(body) => post(at("developer-tokens"), body),
            Self::RevokeToken(id, body) => post(
                at(&format!("developer-tokens/{}/revoke", encode_component(id))),
                body,
            ),
            Self::Preview(id) => get(session(id, "share-preview")),
            Self::SavePreview(id, body) => post(session(id, "share-preview"), body),
            Self::Consent(id, body) => post(session(id, "share-consent"), body),
            Self::Shares(id) => get(session(id, "shares")),
            Self::CreateShare(id, body) => post(session(id, "shares"), body),
            Self::RevokeShare(id, key) => post_empty(session(
                id,
                &format!("shares/{}/revoke", encode_component(key)),
            )),
        }
    }
    pub fn decode(&self, status: u16, body: &str) -> Answer {
        let mutation = self.request().method != Method::Get;
        match self {
            Self::Tokens => Answer::Tokens(decode(status, body, mutation)),
            Self::CreateToken(_) => Answer::TokenIssued(decode(status, body, mutation)),
            Self::RevokeToken(..) => Answer::TokenRevoked(decode(status, body, mutation)),
            Self::Preview(_) | Self::SavePreview(..) => {
                Answer::Preview(decode(status, body, mutation))
            }
            Self::Consent(..) => Answer::Consent(decode(status, body, mutation)),
            Self::Shares(_) => Answer::Shares(decode(status, body, mutation)),
            Self::CreateShare(..) => Answer::ShareIssued(decode(status, body, mutation)),
            Self::RevokeShare(..) => Answer::ShareRevoked(decode(status, body, mutation)),
        }
    }
}
impl Answer {
    pub fn error(&self) -> Option<ApiError> {
        match self {
            Self::Tokens(r) => r.as_ref().err().copied(),
            Self::TokenIssued(r) => r.as_ref().err().copied(),
            Self::TokenRevoked(r) | Self::Consent(r) | Self::ShareRevoked(r) => {
                r.as_ref().err().copied()
            }
            Self::Preview(r) => r.as_ref().err().copied(),
            Self::Shares(r) => r.as_ref().err().copied(),
            Self::ShareIssued(r) => r.as_ref().err().copied(),
        }
    }
}
