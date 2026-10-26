//! Provider-neutral OAuth/OIDC port; Google is the sole configured provider.
//! Credentials are deliberately absent from Debug and sessions remain first-party.

use std::{fmt, future::Future, pin::Pin};

use crate::EmailAddress;
use sha2::{Digest, Sha256};

/// RFC 7636 S256, encoded as base64url by the authorization adapter.
pub fn pkce_challenge(verifier: &OAuthSecret) -> [u8; 32] {
    Sha256::digest(verifier.expose().as_bytes()).into()
}

/// Temporary OAuth material; callers must never put it in diagnostics.
pub struct OAuthSecret(String);

impl OAuthSecret {
    pub fn new(text: String) -> Self {
        Self(text)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OAuthSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OAuthSecret(<redacted>)")
    }
}

pub struct OAuthAuthorize<'a> {
    pub state: &'a OAuthSecret,
    pub nonce: &'a OAuthSecret,
    pub challenge: [u8; 32],
    /// Hints alone never count as fresh authentication proof.
    pub require_fresh: bool,
}

pub struct OAuthExchange<'a> {
    pub code: &'a OAuthSecret,
    pub verifier: &'a OAuthSecret,
    pub nonce: &'a OAuthSecret,
    pub now_s: u64,
    pub require_fresh: bool,
}

/// Only validated identity claims cross the provider boundary; tokens are discarded.
pub struct OAuthIdentity {
    pub subject: String,
    pub email: EmailAddress,
    pub authenticated_at_s: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OAuthFailure {
    Unavailable,
    Timeout,
    Oversize,
    Malformed,
    Rejected,
    EmailNotVerified,
    FreshAuthenticationRequired,
}

pub type OAuthCall<'a> =
    Pin<Box<dyn Future<Output = Result<OAuthIdentity, OAuthFailure>> + Send + 'a>>;

/// Small object-safe port used by composition and the local fake. Every exchange has a
/// service deadline as well as the transport's deadline. No provider tokens cross this seam.
pub trait OAuthProvider: Send + Sync + 'static {
    fn redirect_uri(&self) -> &str;
    fn authorize(&self, request: OAuthAuthorize<'_>) -> Result<OAuthSecret, OAuthFailure>;
    fn exchange<'a>(&'a self, request: OAuthExchange<'a>) -> OAuthCall<'a>;
}
