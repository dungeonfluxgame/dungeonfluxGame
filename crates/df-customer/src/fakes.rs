//! Faithful in-memory fakes for tests: an `AccountStore` with the same uniqueness, revision,
//! single-use and scoping rules as the PostgreSQL adapter (both pass `contract`), and a mail
//! sender that records messages. Not for production.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use df_types::{AccountId, TenantId, WebSessionId};

use std::future::Future;

use df_types::CampaignId;

use crate::{
    closure::{
        BlockReason, BlockingCampaign, CloseOutcome, ClosureCheck, CompletionOutcome,
        OwnedArchivedCampaign, ScheduledDeletion,
    },
    email::{MailPurpose, Message},
    ports::{AccountStore, EmailError, EmailSender, ResetPolicy, StoreResult},
    provider::{
        Authentication, CallerContext, CreateUser, IdentityFailure, IdentityProvider,
        IdentityProviderKind, ProviderUser, ResetFinish, ResetStart, SetPassword,
    },
    records::{
        AccountEvent, AccountRecord, AccountStatus, AccountsSummary, ConfirmChangeOutcome,
        CreateOutcome, DeviceLabel, DisplayName, EmailAddress, EmailChangeDecision,
        EmailChangeOutcome, EmailChangeRequest, EventKind, NewAccount, NewIdentityAccount,
        NewSession, NewToken, OperationClaim, OperationKey, PasswordChange, PasswordChangeOutcome,
        PurgeCounts, ReceiptOutcome, RecentAccount, ResetOutcome, ResetSnapshot, RetentionCutoffs,
        RevokeReason, RevokeScope, SessionRecord, SessionSummary, TokenPurpose, VerificationTarget,
        VerifyOutcome,
    },
    tokens::{PasswordHash, TokenDigest},
};

mod oauth;

struct TokenRow {
    token: NewToken,
    is_consumed: bool,
    attempts: u32,
}

struct SessionRow {
    record: SessionRecord,
    digest: TokenDigest,
    absolute_expires_at_ms: u64,
    revoked_at_ms: Option<u64>,
}

struct ChangeRow {
    account: AccountId,
    new_email: EmailAddress,
    is_old_confirmed: bool,
    is_new_confirmed: bool,
    is_open: bool,
    expires_at_ms: u64,
}

struct ReceiptRow {
    fingerprint: [u8; 32],
    outcome: Option<ReceiptOutcome>,
    created_at_ms: u64,
}

#[derive(Default)]
struct State {
    oauth_attempts: HashMap<TokenDigest, crate::OAuthAttempt>,
    google_links: HashMap<String, AccountId>,
    google_proofs: HashMap<WebSessionId, Option<u64>>,
    google_consents: HashMap<AccountId, (u64, String, u64)>,
    identities: HashMap<(IdentityProviderKind, String), AccountId>,
    accounts: HashMap<AccountId, AccountRecord>,
    tenants: HashMap<AccountId, TenantId>,
    by_email: HashMap<String, AccountId>,
    tokens: HashMap<TokenDigest, TokenRow>,
    sessions: HashMap<WebSessionId, SessionRow>,
    changes: HashMap<[u8; 16], ChangeRow>,
    events: HashMap<AccountId, Vec<AccountEvent>>,
    receipts: HashMap<(&'static str, [u8; 32], [u8; 16]), ReceiptRow>,
    campaigns: Vec<SeededCampaign>,
    proposals: Vec<SeededProposal>,
}

/// A campaign the memory store's closure model knows (what the campaign owner's participant
/// would answer from its own tables).
#[derive(Clone, Debug)]
pub struct CampaignSeed {
    pub campaign: CampaignId,
    pub name: String,
    pub owner: AccountId,
    pub host: AccountId,
    pub archived: bool,
    pub members: Vec<AccountId>,
}

struct SeededCampaign {
    seed: CampaignSeed,
    scheduled_delete_at_ms: Option<u64>,
}

struct SeededProposal {
    from: AccountId,
    is_open: bool,
}

/// What the closure scenarios need from a store's campaign model, so the memory store and the
/// PostgreSQL store (with a test participant) are driven by the same scenarios.
pub trait CampaignSeeding {
    fn seed_campaign(&self, seed: CampaignSeed) -> impl Future<Output = ()> + Send;
    fn seed_outgoing_proposal(&self, from: AccountId) -> impl Future<Output = ()> + Send;
    fn archive_campaign(&self, campaign: CampaignId) -> impl Future<Output = ()> + Send;
    /// The campaign's scheduled deletion time, if any.
    fn scheduled_deletion(&self, campaign: CampaignId) -> impl Future<Output = Option<u64>> + Send;
    fn open_proposals_from(&self, from: AccountId) -> impl Future<Output = usize> + Send;
    /// Whether the campaign still lists `account` as a member.
    fn is_member(
        &self,
        campaign: CampaignId,
        account: AccountId,
    ) -> impl Future<Output = bool> + Send;
}

impl State {
    fn issue_tenant(&mut self, account: AccountId, tenant: TenantId) {
        self.tenants.entry(account).or_insert(tenant);
    }

    fn closure_check(&self, account: AccountId) -> ClosureCheck {
        let mut check = ClosureCheck::default();
        for entry in &self.campaigns {
            let seed = &entry.seed;
            if seed.archived {
                if seed.owner == account {
                    check.owned_archived.push(OwnedArchivedCampaign {
                        campaign: seed.campaign,
                        name: seed.name.clone(),
                    });
                }
                continue;
            }
            let reason = match (seed.owner == account, seed.host == account) {
                (true, true) => BlockReason::OwnsAndHosts,
                (true, false) => BlockReason::Owns,
                (false, true) => BlockReason::Hosts,
                (false, false) => continue,
            };
            check.blocking.push(BlockingCampaign {
                campaign: seed.campaign,
                name: seed.name.clone(),
                reason,
            });
        }
        check
    }
}

impl CampaignSeeding for MemoryAccountStore {
    async fn seed_campaign(&self, seed: CampaignSeed) {
        self.state().campaigns.push(SeededCampaign {
            seed,
            scheduled_delete_at_ms: None,
        });
    }

    async fn seed_outgoing_proposal(&self, from: AccountId) {
        self.state().proposals.push(SeededProposal {
            from,
            is_open: true,
        });
    }

    async fn archive_campaign(&self, campaign: CampaignId) {
        for entry in &mut self.state().campaigns {
            if entry.seed.campaign == campaign {
                entry.seed.archived = true;
            }
        }
    }

    async fn scheduled_deletion(&self, campaign: CampaignId) -> Option<u64> {
        self.state()
            .campaigns
            .iter()
            .find(|entry| entry.seed.campaign == campaign)
            .and_then(|entry| entry.scheduled_delete_at_ms)
    }

    async fn open_proposals_from(&self, from: AccountId) -> usize {
        self.state()
            .proposals
            .iter()
            .filter(|proposal| proposal.from == from && proposal.is_open)
            .count()
    }

    async fn is_member(&self, campaign: CampaignId, account: AccountId) -> bool {
        self.state()
            .campaigns
            .iter()
            .find(|entry| entry.seed.campaign == campaign)
            .is_some_and(|entry| entry.seed.members.contains(&account))
    }
}

impl State {
    fn event(&mut self, account: AccountId, kind: EventKind, at_ms: u64, device: DeviceLabel) {
        self.events.entry(account).or_default().push(AccountEvent {
            kind,
            at_ms,
            device,
        });
    }

    /// The memory store keeps no revocation reason: no port method reads it back.
    fn revoke(&mut self, account: AccountId, scope: RevokeScope, now_ms: u64) -> u64 {
        let mut revoked = 0;
        for row in self.sessions.values_mut() {
            let in_scope = match scope {
                RevokeScope::One(id) => row.record.id == id,
                RevokeScope::AllExcept(id) => row.record.id != id,
                RevokeScope::All => true,
            };
            if row.record.account == account && in_scope && row.revoked_at_ms.is_none() {
                row.revoked_at_ms = Some(now_ms);
                row.record.is_revoked = true;
                revoked += 1;
            }
        }
        revoked
    }

    fn insert_session(&mut self, session: NewSession) {
        self.sessions.insert(
            session.id,
            SessionRow {
                record: SessionRecord {
                    id: session.id,
                    account: session.account,
                    credential_generation: session.credential_generation,
                    created_at_ms: session.created_at_ms,
                    last_seen_at_ms: session.created_at_ms,
                    is_revoked: false,
                    csrf_secret: session.csrf_secret,
                    device: session.device,
                },
                digest: session.digest,
                absolute_expires_at_ms: session.absolute_expires_at_ms,
                revoked_at_ms: None,
            },
        );
    }

    fn open_token(&self, digest: &TokenDigest, now_ms: u64) -> Option<&TokenRow> {
        self.tokens
            .get(digest)
            .filter(|row| !row.is_consumed && now_ms < row.token.expires_at_ms)
    }

    /// The shared reset transaction: policy over the token's snapshot, then (on success) the
    /// replacement hash if any, the generation bump, every session revoked and the event.
    fn apply_reset(
        &mut self,
        digest: TokenDigest,
        now_ms: u64,
        policy: ResetPolicy,
        new_hash: Option<PasswordHash>,
        tenant: TenantId,
    ) -> ResetOutcome {
        let Some(row) = self.tokens.get(&digest) else {
            return ResetOutcome::Invalid;
        };
        let account_id = row.token.account;
        let Some(account) = self.accounts.get(&account_id) else {
            return ResetOutcome::Invalid;
        };
        let mut snapshot = ResetSnapshot {
            account: account_id,
            purpose_is_reset: row.token.purpose == TokenPurpose::PasswordReset,
            expires_at_ms: row.token.expires_at_ms,
            attempts: row.attempts,
            attempt_limit: row.token.attempt_limit,
            is_consumed: row.is_consumed,
            credential_generation: account.credential_generation,
            is_account_active: account.status == AccountStatus::Active,
        };
        let decision = policy(&mut snapshot);
        if let Some(row) = self.tokens.get_mut(&digest) {
            row.attempts = snapshot.attempts;
            row.is_consumed = snapshot.is_consumed;
        }
        if let Err(refusal) = decision {
            return ResetOutcome::Refused(refusal);
        }
        if let Some(account) = self.accounts.get_mut(&account_id) {
            account.credential_generation = snapshot.credential_generation;
            if new_hash.is_some() {
                account.password_hash = new_hash;
            }
            // Holding the emailed link proves control of the address.
            account.is_verified = true;
        }
        self.issue_tenant(account_id, tenant);
        self.revoke(account_id, RevokeScope::All, now_ms);
        self.event(
            account_id,
            EventKind::PasswordReset,
            now_ms,
            DeviceLabel::Unknown,
        );
        ResetOutcome::Reset(account_id)
    }

    fn consume_change_tokens(&mut self, change: [u8; 16]) {
        for row in self.tokens.values_mut() {
            if row.token.email_change == Some(change) {
                row.is_consumed = true;
            }
        }
    }
}

fn random_tenant() -> StoreResult<TenantId> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| crate::StoreError::Unavailable)?;
    TenantId::from_bytes(&bytes).map_err(|_| crate::StoreError::InvalidRecord)
}

fn receipt_key(key: &OperationKey) -> (&'static str, [u8; 32], [u8; 16]) {
    (
        key.namespace.label(),
        key.principal,
        *key.operation.as_bytes(),
    )
}

/// The in-memory `AccountStore`. Clones share one state.
#[derive(Clone, Default)]
pub struct MemoryAccountStore {
    state: Arc<Mutex<State>>,
    event_write_delay_ms: Arc<AtomicU64>,
}

impl MemoryAccountStore {
    /// Delays account-event writes so timing behavior can be tested against a slow store.
    pub fn set_event_write_delay(&self, delay: Duration) {
        let millis = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        self.event_write_delay_ms.store(millis, Ordering::Relaxed);
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every stored session token digest, so tests can show no raw token is kept.
    pub fn stored_session_digests(&self) -> Vec<TokenDigest> {
        self.state()
            .sessions
            .values()
            .map(|row| row.digest)
            .collect()
    }
}

impl AccountStore for MemoryAccountStore {
    async fn create_account(&self, account: NewAccount) -> StoreResult<CreateOutcome> {
        let mut state = self.state();
        if state.by_email.contains_key(account.email.normalized()) {
            return Ok(CreateOutcome::EmailTaken);
        }
        state
            .by_email
            .insert(account.email.normalized().to_owned(), account.id);
        state.accounts.insert(
            account.id,
            AccountRecord {
                id: account.id,
                email: Some(account.email),
                is_verified: false,
                display_name: None,
                status: AccountStatus::Active,
                credential_generation: 1,
                password_hash: Some(account.password_hash),
                created_at_ms: account.created_at_ms,
                delete_after_ms: None,
            },
        );
        state.tokens.insert(
            account.verification.digest,
            TokenRow {
                token: account.verification,
                is_consumed: false,
                attempts: 0,
            },
        );
        state.event(
            account.id,
            EventKind::SignedUp,
            account.created_at_ms,
            DeviceLabel::Unknown,
        );
        Ok(CreateOutcome::Created)
    }

    async fn create_identity_account(
        &self,
        account: NewIdentityAccount,
    ) -> StoreResult<CreateOutcome> {
        let tenant = random_tenant()?;
        let mut state = self.state();
        if state.by_email.contains_key(account.email.normalized()) {
            return Ok(CreateOutcome::EmailTaken);
        }
        state
            .by_email
            .insert(account.email.normalized().to_owned(), account.id);
        state
            .identities
            .insert((account.provider, account.subject), account.id);
        if account.is_verified {
            state.issue_tenant(account.id, tenant);
        }
        if let Some(token) = account.verification {
            state.tokens.insert(
                token.digest,
                TokenRow {
                    token,
                    is_consumed: false,
                    attempts: 0,
                },
            );
        }
        state.accounts.insert(
            account.id,
            AccountRecord {
                id: account.id,
                email: Some(account.email),
                is_verified: account.is_verified,
                display_name: None,
                status: AccountStatus::Active,
                credential_generation: 1,
                password_hash: None,
                created_at_ms: account.created_at_ms,
                delete_after_ms: None,
            },
        );
        state.event(
            account.id,
            EventKind::SignedUp,
            account.created_at_ms,
            DeviceLabel::Unknown,
        );
        Ok(CreateOutcome::Created)
    }

    async fn account_for_identity(
        &self,
        provider: IdentityProviderKind,
        subject: &str,
    ) -> StoreResult<Option<AccountRecord>> {
        let state = self.state();
        Ok(state
            .identities
            .get(&(provider, subject.to_owned()))
            .and_then(|id| state.accounts.get(id))
            .cloned())
    }

    async fn identity_for_account(
        &self,
        account: AccountId,
        provider: IdentityProviderKind,
    ) -> StoreResult<Option<String>> {
        Ok(self
            .state()
            .identities
            .iter()
            .find(|((kind, _), linked)| *kind == provider && **linked == account)
            .map(|((_, subject), _)| subject.clone()))
    }

    async fn identities_of(
        &self,
        account: AccountId,
    ) -> StoreResult<Vec<(IdentityProviderKind, String)>> {
        let state = self.state();
        let mut found: Vec<(IdentityProviderKind, String)> = state
            .identities
            .iter()
            .filter(|(_, linked)| **linked == account)
            .map(|((provider, subject), _)| (*provider, subject.clone()))
            .collect();
        found.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(found)
    }

    async fn unlink_identity(
        &self,
        account: AccountId,
        provider: IdentityProviderKind,
        subject: &str,
    ) -> StoreResult<()> {
        let mut state = self.state();
        let key = (provider, subject.to_owned());
        if state.identities.get(&key) == Some(&account) {
            state.identities.remove(&key);
        }
        Ok(())
    }

    async fn link_identity(
        &self,
        account: AccountId,
        provider: IdentityProviderKind,
        subject: &str,
        _now_ms: u64,
    ) -> StoreResult<()> {
        self.state()
            .identities
            .insert((provider, subject.to_owned()), account);
        Ok(())
    }

    async fn account_by_email(&self, normalized_email: &str) -> StoreResult<Option<AccountRecord>> {
        let state = self.state();
        Ok(state
            .by_email
            .get(normalized_email)
            .and_then(|id| state.accounts.get(id))
            .cloned())
    }

    async fn account(&self, id: AccountId) -> StoreResult<Option<AccountRecord>> {
        Ok(self.state().accounts.get(&id).cloned())
    }

    async fn tenant_for(&self, account: AccountId) -> StoreResult<Option<TenantId>> {
        let state = self.state();
        Ok(state
            .accounts
            .get(&account)
            .filter(|record| record.is_verified && record.status == AccountStatus::Active)
            .and_then(|_| state.tenants.get(&account).copied()))
    }

    async fn issue_token(&self, token: NewToken) -> StoreResult<()> {
        let mut state = self.state();
        for row in state.tokens.values_mut() {
            if row.token.account == token.account && row.token.purpose == token.purpose {
                row.is_consumed = true;
            }
        }
        state.tokens.insert(
            token.digest,
            TokenRow {
                token,
                is_consumed: false,
                attempts: 0,
            },
        );
        Ok(())
    }

    async fn verification_target(
        &self,
        digest: TokenDigest,
        now_ms: u64,
    ) -> StoreResult<Option<VerificationTarget>> {
        let state = self.state();
        let Some(row) = state.tokens.get(&digest) else {
            return Ok(None);
        };
        if row.token.purpose != TokenPurpose::VerifyEmail {
            return Ok(None);
        }
        let account = row.token.account;
        let already_verified = state
            .accounts
            .get(&account)
            .is_some_and(|account| account.is_verified);
        let is_open = !row.is_consumed && now_ms < row.token.expires_at_ms;
        Ok((already_verified || is_open).then_some(VerificationTarget {
            account,
            already_verified,
        }))
    }

    async fn verify_email(
        &self,
        digest: TokenDigest,
        now_ms: u64,
        replacement: Option<PasswordHash>,
    ) -> StoreResult<VerifyOutcome> {
        let tenant = random_tenant()?;
        let mut state = self.state();
        let Some(row) = state.tokens.get(&digest) else {
            return Ok(VerifyOutcome::Invalid);
        };
        if row.token.purpose != TokenPurpose::VerifyEmail {
            return Ok(VerifyOutcome::Invalid);
        }
        let account = row.token.account;
        let is_open = !row.is_consumed && now_ms < row.token.expires_at_ms;
        let is_verified = state
            .accounts
            .get(&account)
            .is_some_and(|account| account.is_verified);
        if is_verified {
            return Ok(VerifyOutcome::AlreadyVerified(account));
        }
        if !is_open {
            return Ok(VerifyOutcome::Invalid);
        }
        if let Some(row) = state.tokens.get_mut(&digest) {
            row.is_consumed = true;
        }
        let Some(record) = state.accounts.get_mut(&account) else {
            return Ok(VerifyOutcome::Invalid);
        };
        record.is_verified = true;
        // Proof of the mailbox ends every session made before it, and a password chosen at
        // this step replaces whatever the sign-up set.
        record.credential_generation = record.credential_generation.saturating_add(1);
        if let Some(hash) = replacement {
            record.password_hash = Some(hash);
        }
        state.issue_tenant(account, tenant);
        state.revoke(account, RevokeScope::All, now_ms);
        state.event(account, EventKind::Verified, now_ms, DeviceLabel::Unknown);
        Ok(VerifyOutcome::Verified(account))
    }

    async fn create_session(
        &self,
        session: NewSession,
        event: Option<AccountEvent>,
    ) -> StoreResult<()> {
        let mut state = self.state();
        let account = session.account;
        state.insert_session(session);
        if let Some(event) = event {
            state.events.entry(account).or_default().push(event);
        }
        Ok(())
    }

    async fn session_by_digest(
        &self,
        digest: TokenDigest,
    ) -> StoreResult<Option<(SessionRecord, AccountRecord)>> {
        let state = self.state();
        Ok(state
            .sessions
            .values()
            .find(|row| row.digest == digest)
            .and_then(|row| {
                let account = state.accounts.get(&row.record.account)?;
                Some((row.record.clone(), account.clone()))
            }))
    }

    async fn touch_session(&self, id: WebSessionId, now_ms: u64) -> StoreResult<()> {
        if let Some(row) = self.state().sessions.get_mut(&id) {
            row.record.last_seen_at_ms = row.record.last_seen_at_ms.max(now_ms);
        }
        Ok(())
    }

    async fn revoke_sessions(
        &self,
        account: AccountId,
        scope: RevokeScope,
        _reason: RevokeReason,
        now_ms: u64,
        event: Option<AccountEvent>,
    ) -> StoreResult<u64> {
        let mut state = self.state();
        let revoked = state.revoke(account, scope, now_ms);
        if let Some(event) = event {
            state.events.entry(account).or_default().push(event);
        }
        Ok(revoked)
    }

    async fn sessions(
        &self,
        account: AccountId,
        now_ms: u64,
        idle_before_ms: u64,
    ) -> StoreResult<Vec<SessionSummary>> {
        let mut sessions: Vec<SessionSummary> = self
            .state()
            .sessions
            .values()
            .filter(|row| {
                row.record.account == account
                    && row.revoked_at_ms.is_none()
                    && now_ms < row.absolute_expires_at_ms
                    && row.record.last_seen_at_ms >= idle_before_ms
            })
            .map(|row| SessionSummary {
                id: row.record.id,
                device: row.record.device,
                created_at_ms: row.record.created_at_ms,
                last_seen_at_ms: row.record.last_seen_at_ms,
            })
            .collect();
        sessions.sort_by(|a, b| {
            b.last_seen_at_ms
                .cmp(&a.last_seen_at_ms)
                .then(a.id.cmp(&b.id))
        });
        Ok(sessions)
    }

    async fn change_password(&self, change: PasswordChange) -> StoreResult<PasswordChangeOutcome> {
        let mut state = self.state();
        let Some(account) = state.accounts.get_mut(&change.account) else {
            return Ok(PasswordChangeOutcome::StaleGeneration);
        };
        if account.credential_generation != change.expected_generation {
            return Ok(PasswordChangeOutcome::StaleGeneration);
        }
        account.credential_generation += 1;
        account.password_hash = Some(change.new_hash);
        state.revoke(change.account, RevokeScope::All, change.at_ms);
        let device = change.replacement.device;
        state.insert_session(change.replacement);
        state.event(
            change.account,
            EventKind::PasswordChanged,
            change.at_ms,
            device,
        );
        Ok(PasswordChangeOutcome::Changed)
    }

    async fn rehash_password(
        &self,
        account: AccountId,
        expected_generation: u64,
        new_hash: PasswordHash,
    ) -> StoreResult<()> {
        if let Some(account) = self.state().accounts.get_mut(&account)
            && account.credential_generation == expected_generation
        {
            account.password_hash = Some(new_hash);
        }
        Ok(())
    }

    async fn reset_password(
        &self,
        digest: TokenDigest,
        new_hash: PasswordHash,
        now_ms: u64,
        policy: ResetPolicy,
    ) -> StoreResult<ResetOutcome> {
        Ok(self
            .state()
            .apply_reset(digest, now_ms, policy, Some(new_hash), random_tenant()?))
    }

    async fn consume_reset_token(
        &self,
        digest: TokenDigest,
        now_ms: u64,
        policy: ResetPolicy,
    ) -> StoreResult<ResetOutcome> {
        Ok(self
            .state()
            .apply_reset(digest, now_ms, policy, None, random_tenant()?))
    }

    async fn request_email_change(
        &self,
        request: EmailChangeRequest,
        now_ms: u64,
    ) -> StoreResult<EmailChangeOutcome> {
        let mut state = self.state();
        if state.by_email.contains_key(request.new_email.normalized()) {
            return Ok(EmailChangeOutcome::AddressTaken);
        }
        let earlier: Vec<[u8; 16]> = state
            .changes
            .iter()
            .filter(|(_, change)| change.account == request.account && change.is_open)
            .map(|(id, _)| *id)
            .collect();
        for id in earlier {
            if let Some(change) = state.changes.get_mut(&id) {
                change.is_open = false;
            }
            state.consume_change_tokens(id);
        }
        state.changes.insert(
            request.id,
            ChangeRow {
                account: request.account,
                new_email: request.new_email,
                is_old_confirmed: false,
                is_new_confirmed: false,
                is_open: true,
                expires_at_ms: request.expires_at_ms,
            },
        );
        for token in [request.old_address_token, request.new_address_token] {
            state.tokens.insert(
                token.digest,
                TokenRow {
                    token,
                    is_consumed: false,
                    attempts: 0,
                },
            );
        }
        state.event(
            request.account,
            EventKind::EmailChangeRequested,
            now_ms,
            DeviceLabel::Unknown,
        );
        Ok(EmailChangeOutcome::Requested)
    }

    async fn confirm_email_change(
        &self,
        digest: TokenDigest,
        decision: EmailChangeDecision,
        now_ms: u64,
    ) -> StoreResult<ConfirmChangeOutcome> {
        let mut state = self.state();
        let Some(row) = state.open_token(&digest, now_ms) else {
            return Ok(ConfirmChangeOutcome::Invalid);
        };
        let purpose = row.token.purpose;
        let (Some(change_id), true) = (
            row.token.email_change,
            matches!(
                purpose,
                TokenPurpose::EmailChangeOld | TokenPurpose::EmailChangeNew
            ),
        ) else {
            return Ok(ConfirmChangeOutcome::Invalid);
        };
        let Some(change) = state
            .changes
            .get_mut(&change_id)
            .filter(|change| change.is_open && now_ms < change.expires_at_ms)
        else {
            return Ok(ConfirmChangeOutcome::Invalid);
        };
        let account_id = change.account;
        if decision == EmailChangeDecision::Cancel {
            change.is_open = false;
            state.consume_change_tokens(change_id);
            return Ok(ConfirmChangeOutcome::Cancelled {
                account: account_id,
            });
        }
        if purpose == TokenPurpose::EmailChangeOld {
            change.is_old_confirmed = true;
        } else {
            change.is_new_confirmed = true;
        }
        let is_complete = change.is_old_confirmed && change.is_new_confirmed;
        let new_email = change.new_email.clone();
        if let Some(row) = state.tokens.get_mut(&digest) {
            row.is_consumed = true;
        }
        if !is_complete {
            return Ok(ConfirmChangeOutcome::AwaitingOtherAddress);
        }
        if let Some(change) = state.changes.get_mut(&change_id) {
            change.is_open = false;
        }
        if state.by_email.contains_key(new_email.normalized()) {
            return Ok(ConfirmChangeOutcome::AddressTaken);
        }
        let Some(account) = state.accounts.get_mut(&account_id) else {
            return Ok(ConfirmChangeOutcome::Invalid);
        };
        let Some(old_email) = account.email.replace(new_email.clone()) else {
            return Ok(ConfirmChangeOutcome::Invalid);
        };
        account.credential_generation += 1;
        account.is_verified = true;
        state.by_email.remove(old_email.normalized());
        state
            .by_email
            .insert(new_email.normalized().to_owned(), account_id);
        state.revoke(account_id, RevokeScope::All, now_ms);
        state.event(
            account_id,
            EventKind::EmailChanged,
            now_ms,
            DeviceLabel::Unknown,
        );
        Ok(ConfirmChangeOutcome::Completed {
            account: account_id,
            old_email,
            new_email,
        })
    }

    async fn update_profile(
        &self,
        account: AccountId,
        display_name: Option<DisplayName>,
    ) -> StoreResult<()> {
        if let Some(account) = self.state().accounts.get_mut(&account) {
            account.display_name = display_name;
        }
        Ok(())
    }

    async fn close_account(
        &self,
        account: AccountId,
        now_ms: u64,
        delete_after_ms: u64,
    ) -> StoreResult<CloseOutcome> {
        let mut guard = self.state();
        let state = &mut *guard;
        let Some(record) = state.accounts.get(&account) else {
            return Ok(CloseOutcome::Blocked(ClosureCheck::default()));
        };
        if record.status == AccountStatus::Closing {
            return Ok(CloseOutcome::AlreadyClosing {
                delete_after_ms: record.delete_after_ms.unwrap_or(delete_after_ms),
            });
        }
        // The decision is made before anything is written, which is what a rolled-back
        // transaction leaves behind.
        let check = state.closure_check(account);
        if !check.blocking.is_empty() {
            return Ok(CloseOutcome::Blocked(check));
        }
        if let Some(record) = state.accounts.get_mut(&account) {
            record.status = AccountStatus::Closing;
            record.delete_after_ms = Some(delete_after_ms);
        }
        let mut scheduled = Vec::new();
        for entry in &mut state.campaigns {
            if entry.seed.archived
                && entry.seed.owner == account
                && entry.scheduled_delete_at_ms.is_none()
            {
                entry.scheduled_delete_at_ms = Some(delete_after_ms);
                scheduled.push(ScheduledDeletion {
                    campaign: entry.seed.campaign,
                    name: entry.seed.name.clone(),
                    members: entry.seed.members.clone(),
                });
            }
        }
        for proposal in &mut state.proposals {
            if proposal.from == account {
                proposal.is_open = false;
            }
        }
        state.revoke(account, RevokeScope::All, now_ms);
        state.event(account, EventKind::Closed, now_ms, DeviceLabel::Unknown);
        Ok(CloseOutcome::Closed {
            delete_after_ms,
            scheduled,
        })
    }

    async fn reopen_account(&self, account: AccountId, now_ms: u64) -> StoreResult<bool> {
        let mut state = self.state();
        let Some(record) = state.accounts.get_mut(&account).filter(|record| {
            record.status == AccountStatus::Closing
                && record.delete_after_ms.is_some_and(|at| at > now_ms)
        }) else {
            return Ok(false);
        };
        record.status = AccountStatus::Active;
        record.delete_after_ms = None;
        for entry in &mut state.campaigns {
            if entry.seed.owner == account {
                entry.scheduled_delete_at_ms = None;
            }
        }
        state.event(account, EventKind::Reopened, now_ms, DeviceLabel::Unknown);
        Ok(true)
    }

    async fn due_closures(&self, now_ms: u64, limit: u32) -> StoreResult<Vec<AccountId>> {
        let state = self.state();
        let mut due: Vec<(u64, AccountId)> = state
            .accounts
            .values()
            .filter(|account| account.status == AccountStatus::Closing)
            .filter_map(|account| {
                account
                    .delete_after_ms
                    .filter(|at| *at <= now_ms)
                    .map(|at| (at, account.id))
            })
            .collect();
        due.sort();
        due.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok(due.into_iter().map(|(_, id)| id).collect())
    }

    async fn complete_closure(
        &self,
        account: AccountId,
        now_ms: u64,
    ) -> StoreResult<CompletionOutcome> {
        let mut guard = self.state();
        let state = &mut *guard;
        let is_due = state.accounts.get(&account).is_some_and(|record| {
            record.status == AccountStatus::Closing
                && record.delete_after_ms.is_some_and(|at| at <= now_ms)
        });
        if !is_due {
            return Ok(CompletionOutcome::NotDue);
        }
        let check = state.closure_check(account);
        if !check.blocking.is_empty() {
            return Ok(CompletionOutcome::Blocked(check));
        }
        for entry in &mut state.campaigns {
            entry.seed.members.retain(|member| *member != account);
        }
        Ok(CompletionOutcome::Completed)
    }

    async fn purge_closed(&self, account: AccountId, now_ms: u64) -> StoreResult<bool> {
        let mut guard = self.state();
        let state = &mut *guard;
        let purgeable = state.accounts.get(&account).is_some_and(|record| {
            record.status == AccountStatus::Closing
                && record.delete_after_ms.is_some_and(|at| at <= now_ms)
        }) && !state.identities.values().any(|linked| *linked == account);
        if !purgeable {
            return Ok(false);
        }
        if let Some(record) = state.accounts.get_mut(&account) {
            if let Some(email) = record.email.take() {
                state.by_email.remove(email.normalized());
            }
            record.password_hash = None;
            record.display_name = None;
            record.status = AccountStatus::Closed;
            record.delete_after_ms = None;
        }
        state
            .sessions
            .retain(|_, row| row.record.account != account);
        state.tokens.retain(|_, row| row.token.account != account);
        state.changes.retain(|_, change| change.account != account);
        state.events.remove(&account);
        state.google_links.retain(|_, linked| *linked != account);
        state
            .oauth_attempts
            .retain(|_, row| row.account != Some(account));
        state
            .google_proofs
            .retain(|session, _| state.sessions.contains_key(session));
        Ok(true)
    }

    async fn record_event(&self, account: AccountId, event: AccountEvent) -> StoreResult<()> {
        let delay_ms = self.event_write_delay_ms.load(Ordering::Relaxed);
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        self.state().events.entry(account).or_default().push(event);
        Ok(())
    }

    async fn events(&self, account: AccountId, limit: u32) -> StoreResult<Vec<AccountEvent>> {
        let state = self.state();
        Ok(state
            .events
            .get(&account)
            .map(|events| {
                events
                    .iter()
                    .rev()
                    .take(usize::try_from(limit).unwrap_or(usize::MAX))
                    .copied()
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn claim_operation(
        &self,
        key: OperationKey,
        fingerprint: [u8; 32],
        now_ms: u64,
    ) -> StoreResult<OperationClaim> {
        let mut state = self.state();
        let entry = receipt_key(&key);
        if let Some(receipt) = state.receipts.get(&entry) {
            if receipt.fingerprint != fingerprint {
                return Ok(OperationClaim::Conflict);
            }
            return Ok(receipt
                .outcome
                .map_or(OperationClaim::InProgress, OperationClaim::Replayed));
        }
        state.receipts.insert(
            entry,
            ReceiptRow {
                fingerprint,
                outcome: None,
                created_at_ms: now_ms,
            },
        );
        Ok(OperationClaim::New)
    }

    async fn finish_operation(
        &self,
        key: OperationKey,
        outcome: ReceiptOutcome,
    ) -> StoreResult<()> {
        if let Some(receipt) = self.state().receipts.get_mut(&receipt_key(&key)) {
            receipt.outcome = Some(outcome);
        }
        Ok(())
    }

    async fn release_operation(&self, key: OperationKey) -> StoreResult<()> {
        let mut state = self.state();
        let entry = receipt_key(&key);
        if state
            .receipts
            .get(&entry)
            .is_some_and(|receipt| receipt.outcome.is_none())
        {
            state.receipts.remove(&entry);
        }
        Ok(())
    }

    async fn purge(&self, cutoffs: RetentionCutoffs) -> StoreResult<PurgeCounts> {
        let mut guard = self.state();
        // A plain reference lets the loop borrow two fields at once.
        let state = &mut *guard;
        let mut counts = PurgeCounts::default();
        state
            .oauth_attempts
            .retain(|_, attempt| attempt.expires_at_ms > cutoffs.now_ms);
        let before = state.sessions.len();
        state.sessions.retain(|_, row| {
            let ended = row
                .revoked_at_ms
                .is_some_and(|at| at < cutoffs.sessions_ended_before_ms)
                || row.absolute_expires_at_ms < cutoffs.sessions_ended_before_ms
                || row.record.last_seen_at_ms < cutoffs.sessions_last_seen_before_ms;
            !ended
        });
        counts.sessions = u64::try_from(before - state.sessions.len()).unwrap_or(u64::MAX);
        state
            .google_proofs
            .retain(|session, _| state.sessions.contains_key(session));
        let before = state.tokens.len();
        state
            .tokens
            .retain(|_, row| row.token.expires_at_ms >= cutoffs.tokens_before_ms);
        counts.tokens = u64::try_from(before - state.tokens.len()).unwrap_or(u64::MAX);
        let before = state.receipts.len();
        state
            .receipts
            .retain(|_, receipt| receipt.created_at_ms >= cutoffs.receipts_before_ms);
        counts.receipts = u64::try_from(before - state.receipts.len()).unwrap_or(u64::MAX);
        Ok(counts)
    }

    async fn summary(&self, recent: u32) -> StoreResult<AccountsSummary> {
        let state = self.state();
        let mut summary = AccountsSummary::default();
        for account in state.accounts.values() {
            match account.status {
                AccountStatus::Active => summary.active += 1,
                AccountStatus::Closing => summary.closing += 1,
                AccountStatus::Closed => summary.closed += 1,
            }
            if account.is_verified {
                summary.verified += 1;
            }
        }
        let mut accounts: Vec<&AccountRecord> = state.accounts.values().collect();
        accounts.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms).then(a.id.cmp(&b.id)));
        summary.recent = accounts
            .into_iter()
            .take(usize::try_from(recent).unwrap_or(usize::MAX))
            .map(|account| RecentAccount {
                id: account.id,
                email: account.email.clone(),
                status: account.status,
                is_verified: account.is_verified,
                active_sessions: state
                    .sessions
                    .values()
                    .filter(|row| row.record.account == account.id && row.revoked_at_ms.is_none())
                    .count() as u64,
                last_event: state
                    .events
                    .get(&account.id)
                    .and_then(|events| events.last().copied()),
            })
            .collect();
        Ok(summary)
    }
}

/// A mail sender that records every message. Clones share one list.
#[derive(Clone, Default)]
pub struct RecordingMail {
    messages: Arc<Mutex<Vec<Message>>>,
    sent: Arc<tokio::sync::Notify>,
}

impl RecordingMail {
    /// Waits until at least `count` messages were sent (the worker delivers asynchronously).
    /// Panics after five seconds: a test waiting that long has found a defect.
    pub async fn wait_for(&self, count: usize) {
        let wait = async {
            loop {
                let notified = self.sent.notified();
                if self.messages().len() >= count {
                    return;
                }
                notified.await;
            }
        };
        if tokio::time::timeout(std::time::Duration::from_secs(5), wait)
            .await
            .is_err()
        {
            panic!("expected {count} messages, have {}", self.messages().len());
        }
    }

    pub fn messages(&self) -> Vec<Message> {
        self.messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The token text of the newest message of `purpose` sent to `to`.
    pub fn last_token(&self, to: &str, purpose: MailPurpose) -> Option<String> {
        self.messages()
            .iter()
            .rev()
            .find(|message| message.to == to && message.purpose == purpose)
            .and_then(|message| {
                let (_, rest) = message
                    .body
                    .split_once("?token=")
                    .or_else(|| message.body.split_once("#token="))?;
                Some(
                    rest.chars()
                        .take_while(|character| {
                            character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                        })
                        .collect(),
                )
            })
    }
}

impl EmailSender for RecordingMail {
    async fn send(&self, message: Message) -> Result<(), EmailError> {
        self.messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message);
        self.sent.notify_waiters();
        Ok(())
    }
}

struct FakeUser {
    subject: String,
    email: EmailAddress,
    password: String,
    verified: bool,
}

struct FakeProviderState {
    users: Vec<FakeUser>,
    next: u64,
    reset_tokens: HashMap<String, String>,
    sessions: Vec<(String, String)>,
    require_verified: bool,
    rejected_passwords: Vec<String>,
    fail_next: Option<IdentityFailure>,
    reset_delay: Option<std::time::Duration>,
    calls: Vec<&'static str>,
}

/// A faithful fake identity provider: addresses are trimmed and lowercased and unique, a wrong
/// password and an unknown address answer alike, an unverified address is refused when the
/// provider requires verification (the WorkOS dashboard setting), reset tokens are single-use
/// and end the user's sessions, and every authentication opens a provider session that stays
/// open until ended. `fail_next` makes the next call fail as the provider would when it is down.
#[derive(Clone)]
pub struct FakeIdentityProvider {
    state: Arc<Mutex<FakeProviderState>>,
}

impl Default for FakeIdentityProvider {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeProviderState {
                users: Vec::new(),
                next: 0,
                reset_tokens: HashMap::new(),
                sessions: Vec::new(),
                require_verified: false,
                rejected_passwords: Vec::new(),
                fail_next: None,
                reset_delay: None,
                calls: Vec::new(),
            })),
        }
    }
}

impl FakeIdentityProvider {
    fn state(&self) -> MutexGuard<'_, FakeProviderState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the provider refuses to authenticate an address it has not verified.
    pub fn require_verified(&self, required: bool) {
        self.state().require_verified = required;
    }

    /// Passwords the provider's own policy refuses.
    pub fn reject_password(&self, password: &str) {
        self.state().rejected_passwords.push(password.to_owned());
    }

    /// The next call fails with `failure` (and changes nothing).
    pub fn fail_next(&self, failure: IdentityFailure) {
        self.state().fail_next = Some(failure);
    }

    /// The next `begin_password_reset` takes this long (a slow provider).
    pub fn delay_next_reset(&self, delay: std::time::Duration) {
        self.state().reset_delay = Some(delay);
    }

    /// A user an operator made at the provider, outside any sign-up of ours.
    pub fn add_user(&self, email: &str, password: &str, verified: bool) -> String {
        let mut state = self.state();
        state.next += 1;
        let subject = format!("fake-user-{}", state.next);
        if let Ok(email) = EmailAddress::parse(email) {
            state.users.push(FakeUser {
                subject: subject.clone(),
                email,
                password: password.to_owned(),
                verified,
            });
        }
        subject
    }

    pub fn has_user(&self, email: &str) -> bool {
        EmailAddress::parse(email).is_ok_and(|email| {
            self.state()
                .users
                .iter()
                .any(|user| user.email.normalized() == email.normalized())
        })
    }

    pub fn is_verified(&self, email: &str) -> bool {
        EmailAddress::parse(email).is_ok_and(|email| {
            self.state()
                .users
                .iter()
                .any(|user| user.email.normalized() == email.normalized() && user.verified)
        })
    }

    /// Provider sessions opened and not yet ended.
    pub fn open_sessions(&self) -> usize {
        self.state().sessions.len()
    }

    /// The names of the calls made so far, oldest first.
    pub fn calls(&self) -> Vec<&'static str> {
        self.state().calls.clone()
    }

    fn begin(state: &mut FakeProviderState, call: &'static str) -> Result<(), IdentityFailure> {
        state.calls.push(call);
        match state.fail_next.take() {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }

    fn user_view(user: &FakeUser) -> ProviderUser {
        ProviderUser {
            subject: user.subject.clone(),
            email: user.email.clone(),
            email_verified: user.verified,
        }
    }
}

impl IdentityProvider for FakeIdentityProvider {
    fn kind(&self) -> IdentityProviderKind {
        IdentityProviderKind::Fake
    }

    async fn create_user(
        &self,
        email: &EmailAddress,
        password: &str,
        _caller: CallerContext,
    ) -> Result<CreateUser, IdentityFailure> {
        let mut state = self.state();
        Self::begin(&mut state, "create_user")?;
        if state
            .users
            .iter()
            .any(|user| user.email.normalized() == email.normalized())
        {
            return Ok(CreateUser::EmailNotAvailable);
        }
        if state
            .rejected_passwords
            .iter()
            .any(|rejected| rejected == password)
        {
            return Ok(CreateUser::PasswordRejected);
        }
        state.next += 1;
        let user = FakeUser {
            subject: format!("fake-user-{}", state.next),
            email: email.clone(),
            password: password.to_owned(),
            verified: false,
        };
        let view = Self::user_view(&user);
        state.users.push(user);
        Ok(CreateUser::Created(view))
    }

    async fn authenticate(
        &self,
        email: &EmailAddress,
        password: &str,
        _caller: CallerContext,
    ) -> Result<Authentication, IdentityFailure> {
        let mut state = self.state();
        Self::begin(&mut state, "authenticate")?;
        let Some(user) = state.users.iter().find(|user| {
            user.email.normalized() == email.normalized() && user.password == password
        }) else {
            return Ok(Authentication::InvalidCredentials);
        };
        if !user.verified && state.require_verified {
            return Ok(Authentication::EmailVerificationRequired);
        }
        let view = Self::user_view(user);
        let session = format!("fake-session-{}-{}", view.subject, state.sessions.len() + 1);
        state.sessions.push((view.subject.clone(), session.clone()));
        Ok(Authentication::Authenticated {
            user: view,
            provider_session: Some(session),
        })
    }

    async fn find_user(
        &self,
        email: &EmailAddress,
    ) -> Result<Option<ProviderUser>, IdentityFailure> {
        let mut state = self.state();
        Self::begin(&mut state, "find_user")?;
        Ok(state
            .users
            .iter()
            .find(|user| user.email.normalized() == email.normalized())
            .map(Self::user_view))
    }

    async fn mark_email_verified(&self, subject: &str) -> Result<(), IdentityFailure> {
        let mut state = self.state();
        Self::begin(&mut state, "mark_email_verified")?;
        if let Some(user) = state.users.iter_mut().find(|user| user.subject == subject) {
            user.verified = true;
        }
        Ok(())
    }

    async fn set_password(
        &self,
        subject: &str,
        password: &str,
    ) -> Result<SetPassword, IdentityFailure> {
        let mut state = self.state();
        Self::begin(&mut state, "set_password")?;
        if state
            .rejected_passwords
            .iter()
            .any(|rejected| rejected == password)
        {
            return Ok(SetPassword::Rejected);
        }
        if let Some(user) = state.users.iter_mut().find(|user| user.subject == subject) {
            user.password = password.to_owned();
        }
        state.sessions.retain(|(owner, _)| owner != subject);
        Ok(SetPassword::Set)
    }

    async fn begin_password_reset(
        &self,
        email: &EmailAddress,
    ) -> Result<ResetStart, IdentityFailure> {
        let delay = self.state().reset_delay.take();
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        let mut state = self.state();
        Self::begin(&mut state, "begin_password_reset")?;
        let Some(subject) = state
            .users
            .iter()
            .find(|user| user.email.normalized() == email.normalized())
            .map(|user| user.subject.clone())
        else {
            return Ok(ResetStart::NoSuchUser);
        };
        state.next += 1;
        let token = format!("fake-reset-token-{:06}", state.next);
        state.reset_tokens.insert(token.clone(), subject);
        Ok(ResetStart::Started { token })
    }

    async fn finish_password_reset(
        &self,
        token: &str,
        new_password: &str,
    ) -> Result<ResetFinish, IdentityFailure> {
        let mut state = self.state();
        Self::begin(&mut state, "finish_password_reset")?;
        if state
            .rejected_passwords
            .iter()
            .any(|rejected| rejected == new_password)
        {
            return Ok(ResetFinish::PasswordRejected);
        }
        let Some(subject) = state.reset_tokens.remove(token) else {
            return Ok(ResetFinish::InvalidToken);
        };
        state.sessions.retain(|(owner, _)| *owner != subject);
        let Some(user) = state.users.iter_mut().find(|user| user.subject == subject) else {
            return Ok(ResetFinish::InvalidToken);
        };
        user.password = new_password.to_owned();
        // A completed reset proves control of the mailbox.
        user.verified = true;
        Ok(ResetFinish::Reset(Self::user_view(user)))
    }

    async fn end_session(&self, provider_session: &str) -> Result<(), IdentityFailure> {
        let mut state = self.state();
        Self::begin(&mut state, "end_session")?;
        state.sessions.retain(|(_, id)| id != provider_session);
        Ok(())
    }

    async fn delete_user(&self, subject: &str) -> Result<(), IdentityFailure> {
        let mut state = self.state();
        Self::begin(&mut state, "delete_user")?;
        state.users.retain(|user| user.subject != subject);
        state.sessions.retain(|(owner, _)| owner != subject);
        state.reset_tokens.retain(|_, owner| owner != subject);
        Ok(())
    }
}
