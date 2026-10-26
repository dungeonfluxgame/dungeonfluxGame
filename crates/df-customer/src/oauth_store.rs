//! Durable OAuth attempts and Google subject links. Each mutation is one bounded transaction.

use crate::{AccountRecord, EmailAddress, OAuthSecret, StoreResult, TokenDigest};
use df_types::{AccountId, WebSessionId};
use std::{future::Future, pin::Pin};

pub type GoogleStoreCall<'a, T> = Pin<Box<dyn Future<Output = StoreResult<T>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OAuthIntent {
    SignUp,
    SignIn,
    Link,
    Reauthenticate,
}

impl OAuthIntent {
    pub const fn label(self) -> &'static str {
        match self {
            Self::SignUp => "sign_up",
            Self::SignIn => "sign_in",
            Self::Link => "link",
            Self::Reauthenticate => "reauthenticate",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        [Self::SignUp, Self::SignIn, Self::Link, Self::Reauthenticate]
            .into_iter()
            .find(|intent| intent.label() == text)
    }
    pub fn requires_fresh(self) -> bool {
        matches!(self, Self::Link | Self::Reauthenticate)
    }
}

pub struct OAuthAttempt {
    pub redirect_uri: String,
    pub state_digest: TokenDigest,
    pub cookie_digest: TokenDigest,
    pub client_digest: [u8; 32],
    pub nonce: OAuthSecret,
    pub verifier: OAuthSecret,
    pub intent: OAuthIntent,
    pub account: Option<AccountId>,
    pub session: Option<WebSessionId>,
    pub credential_generation: Option<u64>,
    pub terms_version: Option<String>,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

/// Validated identity and server-bound resolution data; never supplied by the browser.
pub struct GoogleResolution {
    pub subject: String,
    pub email: EmailAddress,
    pub intent: OAuthIntent,
    pub account: Option<AccountId>,
    pub credential_generation: Option<u64>,
    pub originating_session: Option<WebSessionId>,
    pub idle_before_ms: u64,
    pub new_account: AccountId,
    pub terms_version: Option<String>,
    pub accepted_at_ms: u64,
    pub now_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OAuthRefusal {
    Disabled,
    ConsentRequired,
    Expired,
    Canceled,
    EmailNotVerified,
    PasswordSignInRequired,
    NoAccount,
    AccountUnavailable,
    Conflict,
    ReauthenticationRequired,
    LastSignInMethod,
    Unavailable,
    Throttled,
}

impl OAuthRefusal {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::ConsentRequired => "consent_required",
            Self::Expired => "expired",
            Self::Canceled => "canceled",
            Self::EmailNotVerified => "email_not_verified",
            Self::PasswordSignInRequired => "password_sign_in_required",
            Self::NoAccount => "no_account",
            Self::AccountUnavailable => "account_unavailable",
            Self::Conflict => "conflict",
            Self::ReauthenticationRequired => "reauthentication_required",
            Self::LastSignInMethod => "last_sign_in_method",
            Self::Unavailable => "unavailable",
            Self::Throttled => "throttled",
        }
    }
}

pub enum GoogleResolved {
    Account(AccountRecord),
    Created(AccountRecord),
    Refused(OAuthRefusal),
}

pub struct GoogleSession {
    pub session: crate::NewSession,
    pub subject: String,
    pub reauthentication_expires_at_ms: Option<u64>,
    pub originating_session: Option<WebSessionId>,
    pub idle_before_ms: u64,
    pub event: crate::AccountEvent,
}

/// Implementations cap active attempts globally and per client and purge expired secrets.
pub trait GoogleStore: Send + Sync + 'static {
    fn save_attempt(&self, attempt: OAuthAttempt) -> GoogleStoreCall<'_, bool>;
    /// Atomically deletes and returns one unexpired attempt; all retries return `None`.
    fn claim_attempt(
        &self,
        state: TokenDigest,
        cookie: TokenDigest,
        now_ms: u64,
    ) -> GoogleStoreCall<'_, Option<OAuthAttempt>>;
    fn resolve(&self, request: GoogleResolution) -> GoogleStoreCall<'_, GoogleResolved>;
    fn is_linked(&self, account: AccountId) -> GoogleStoreCall<'_, bool>;
    /// Refuses removal unless an independent password sign-in method remains.
    fn unlink(
        &self,
        account: AccountId,
        generation: u64,
    ) -> GoogleStoreCall<'_, Result<(), OAuthRefusal>>;
    fn create_google_session(&self, session: GoogleSession) -> GoogleStoreCall<'_, bool>;
    /// Outer `None`: password session. Inner `None`: ordinary Google session; `Some` is our proof expiry.
    fn session_proof(&self, session: WebSessionId) -> GoogleStoreCall<'_, Option<Option<u64>>>;
}
