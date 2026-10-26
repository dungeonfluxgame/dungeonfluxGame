//! Customer PAT authority (integration-api-and-embeds-research sections 2 and 4).
//! Operator credentials intentionally have no representation in this customer-only contract.
mod token;
pub use token::{PepperRefusal, Peppers, PersonalToken, TokenId, TokenParseRefusal};

use crate::{
    AccountRecord, AccountStatus, AccountStore, Authenticated, CustomerService, EmailSender,
    Reauthentication, ServiceError, StoreResult,
    service::{read, write},
};
use df_types::AccountId;
use std::future::Future;

pub const MAXIMUM_ACTIVE_TOKENS: usize = 10;
pub const DAY_MS: u64 = 86_400_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    Account,
    Email,
    AuthSessions,
    PlaySessions,
    Assets,
    Funding,
    Statements,
    Stats,
}

impl Scope {
    pub const ALL: [Self; 8] = [
        Self::Account,
        Self::Email,
        Self::AuthSessions,
        Self::PlaySessions,
        Self::Assets,
        Self::Funding,
        Self::Statements,
        Self::Stats,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Account => "account:read",
            Self::Email => "account:email:read",
            Self::AuthSessions => "auth_sessions:read",
            Self::PlaySessions => "play_sessions:read",
            Self::Assets => "assets:read",
            Self::Funding => "funding:read",
            Self::Statements => "statements:read",
            Self::Stats => "stats:read",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == text)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenMetadata {
    pub id: TokenId,
    pub account: AccountId,
    pub label: String,
    pub scopes: Vec<Scope>,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub revoked_at_ms: Option<u64>,
    pub last_used_at_ms: Option<u64>,
}

/// Digest-bearing storage record; deliberately not Debug or serializable.
pub struct StoredToken {
    pub metadata: TokenMetadata,
    pub pepper_id: u32,
    pub digest: [u8; 32],
}

pub struct SignInSession {
    pub id: TokenId,
    pub device_label: &'static str,
    pub created_at_ms: u64,
    pub last_seen_at_ms: u64,
    pub expires_at_ms: u64,
    pub revoked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateTokenOutcome {
    Created,
    LimitReached,
    InactiveAccount,
}

/// Implementations lock the account before counting/inserting. Closure takes that same lock.
pub trait PersonalTokenStore: Send + Sync + 'static {
    /// Opaque metadata only, sorted by stable id, at most 101 records for keyset pagination.
    fn integration_sessions(
        &self,
        account: AccountId,
        after: Option<TokenId>,
        limit: u32,
    ) -> impl Future<Output = StoreResult<Vec<SignInSession>>> + Send;
    fn create_personal_token(
        &self,
        token: StoredToken,
    ) -> impl Future<Output = StoreResult<CreateTokenOutcome>> + Send;
    /// At most the 100 most recent records. Never returns any digest or pepper.
    fn personal_tokens(
        &self,
        account: AccountId,
    ) -> impl Future<Output = StoreResult<Vec<TokenMetadata>>> + Send;
    fn personal_token(
        &self,
        id: TokenId,
    ) -> impl Future<Output = StoreResult<Option<StoredToken>>> + Send;
    fn revoke_personal_token(
        &self,
        account: AccountId,
        id: TokenId,
        now_ms: u64,
    ) -> impl Future<Output = StoreResult<bool>> + Send;
    /// Compare-and-swap the old digest; revocation is never cleared by a concurrent use.
    fn used_personal_token(
        &self,
        id: TokenId,
        old_digest: [u8; 32],
        pepper_id: u32,
        digest: [u8; 32],
        now_ms: u64,
    ) -> impl Future<Output = StoreResult<bool>> + Send;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenRefusal {
    InvalidLabel,
    InvalidScopes,
    InvalidExpiry,
    ReauthenticationRequired,
    LimitReached,
    InactiveAccount,
}

pub struct IssuedToken {
    pub metadata: TokenMetadata,
    pub secret: PersonalToken,
}
pub enum IssueToken {
    Issued(IssuedToken),
    Refused(TokenRefusal),
}
pub struct IntegrationPrincipal {
    pub account: AccountRecord,
    pub token: TokenMetadata,
}

fn validate(label: &str, scopes: &[Scope], days: u16) -> Result<(), TokenRefusal> {
    if label.trim().is_empty() || label.chars().count() > 60 || label.chars().any(char::is_control)
    {
        return Err(TokenRefusal::InvalidLabel);
    }
    if scopes.is_empty()
        || scopes.len() > Scope::ALL.len()
        || scopes.windows(2).any(|s| s[0] >= s[1])
        || (scopes.contains(&Scope::Email) && !scopes.contains(&Scope::Account))
    {
        return Err(TokenRefusal::InvalidScopes);
    }
    if ![7, 30, 90, 180, 365].contains(&days) {
        return Err(TokenRefusal::InvalidExpiry);
    }
    Ok(())
}

impl<S: AccountStore + PersonalTokenStore, M: EmailSender> CustomerService<S, M> {
    pub async fn integration_sessions(
        &self,
        account: AccountId,
        after: Option<TokenId>,
        limit: u32,
    ) -> Result<Vec<SignInSession>, ServiceError> {
        let mut sessions = read(
            self.shared
                .store
                .integration_sessions(account, after, limit.min(101)),
        )
        .await?;
        let idle_ms = u64::try_from(self.shared.tunables.sessions.idle.as_millis())
            .map_err(|_| ServiceError::InvalidRecord)?;
        for session in &mut sessions {
            session.expires_at_ms = session.expires_at_ms.min(session.last_seen_at_ms.saturating_add(idle_ms));
        }
        Ok(sessions)
    }
    pub async fn issue_personal_token(
        &self,
        caller: &Authenticated,
        proof: Reauthentication<'_>,
        peppers: &Peppers,
        label: &str,
        mut scopes: Vec<Scope>,
        days: u16,
    ) -> Result<IssueToken, ServiceError> {
        scopes.sort_unstable();
        if let Err(refusal) = validate(label, &scopes, days) {
            return Ok(IssueToken::Refused(refusal));
        }
        if caller.signed_in_at_ms > self.now_ms()
            || !self
                .is_reauthenticated(caller, proof, crate::CLOSURE_REAUTHENTICATION_MS)
                .await?
        {
            return Ok(IssueToken::Refused(TokenRefusal::ReauthenticationRequired));
        }
        let secret = PersonalToken::generate()?;
        let now_ms = self.now_ms();
        let metadata = TokenMetadata {
            id: secret.id,
            account: caller.account.id,
            label: label.trim().to_owned(),
            scopes,
            created_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(u64::from(days) * DAY_MS),
            revoked_at_ms: None,
            last_used_at_ms: None,
        };
        let stored = StoredToken {
            metadata: metadata.clone(),
            pepper_id: peppers.active_id(),
            digest: peppers.digest(&secret),
        };
        match write(self.shared.store.create_personal_token(stored)).await? {
            CreateTokenOutcome::Created => {
                audit("created", metadata.id);
                Ok(IssueToken::Issued(IssuedToken { metadata, secret }))
            }
            CreateTokenOutcome::LimitReached => Ok(IssueToken::Refused(TokenRefusal::LimitReached)),
            CreateTokenOutcome::InactiveAccount => {
                Ok(IssueToken::Refused(TokenRefusal::InactiveAccount))
            }
        }
    }

    pub async fn personal_tokens(
        &self,
        caller: &Authenticated,
    ) -> Result<Vec<TokenMetadata>, ServiceError> {
        read(self.shared.store.personal_tokens(caller.account.id)).await
    }

    pub async fn revoke_personal_token(
        &self,
        caller: &Authenticated,
        proof: Reauthentication<'_>,
        id: TokenId,
    ) -> Result<Result<bool, TokenRefusal>, ServiceError> {
        if caller.signed_in_at_ms > self.now_ms()
            || !self
                .is_reauthenticated(caller, proof, crate::CLOSURE_REAUTHENTICATION_MS)
                .await?
        {
            return Ok(Err(TokenRefusal::ReauthenticationRequired));
        }
        let revoked = write(self.shared.store.revoke_personal_token(
            caller.account.id,
            id,
            self.now_ms(),
        ))
        .await?;
        if revoked {
            audit("revoked", id);
        }
        Ok(Ok(revoked))
    }

    pub async fn integration_principal(
        &self,
        bearer: &str,
        peppers: &Peppers,
    ) -> Result<Option<IntegrationPrincipal>, ServiceError> {
        let Ok(token) = PersonalToken::parse(bearer) else {
            return Ok(None);
        };
        let Some(stored) = read(self.shared.store.personal_token(token.id)).await? else {
            return Ok(None);
        };
        let now_ms = self.now_ms();
        if stored.metadata.revoked_at_ms.is_some()
            || stored.metadata.expires_at_ms <= now_ms
            || !peppers.accepts(&token, stored.pepper_id, &stored.digest, now_ms)
        {
            return Ok(None);
        }
        let Some(account) = read(self.shared.store.account(stored.metadata.account)).await? else {
            return Ok(None);
        };
        if account.status != AccountStatus::Active || !account.is_verified {
            return Ok(None);
        }
        if !write(self.shared.store.used_personal_token(
            token.id,
            stored.digest,
            peppers.active_id(),
            peppers.digest(&token),
            now_ms,
        ))
        .await?
        {
            return Ok(None);
        }
        Ok(Some(IntegrationPrincipal {
            account,
            token: stored.metadata,
        }))
    }
}

fn audit(event: &'static str, id: TokenId) {
    df_observe::record_audit_line(
        "account.personal_token",
        format!("ACCOUNT personal_token status={event} token={}", id.text()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pat_creation_rejects_non_catalog_scopes_expiries_and_empty_labels() {
        assert_eq!(Scope::parse("admin:accounts:read"), None);
        assert_eq!(Scope::parse("*"), None);
        assert_eq!(
            validate("x", &[Scope::Email], 90),
            Err(TokenRefusal::InvalidScopes)
        );
        assert_eq!(
            validate("x", &[Scope::Account, Scope::Account], 90),
            Err(TokenRefusal::InvalidScopes)
        );
        assert_eq!(
            validate("x", &[Scope::Account], 0),
            Err(TokenRefusal::InvalidExpiry)
        );
        assert_eq!(
            validate(" ", &[Scope::Account], 90),
            Err(TokenRefusal::InvalidLabel)
        );
        for days in [7, 30, 90, 180, 365] {
            assert!(validate("tool", &[Scope::Account, Scope::Email], days).is_ok());
        }
    }
}
