//! Consumer-owned ports (`subsystem-architecture.md`: async ports belong to their consumer).
//! `df-persistence` implements `AccountStore`; email adapters implement `EmailSender`.
//! Composition builds the adapters and hands them to the service.
//!
//! Every store call runs under the service's `STORE_DEADLINE`. Each mutating method is one
//! transaction: it either commits all of its rows or none.

use std::future::Future;

use df_types::{AccountId, TenantId, WebSessionId};

use crate::{
    closure::{CloseOutcome, CompletionOutcome},
    email::Message,
    provider::IdentityProviderKind,
    records::{
        AccountEvent, AccountRecord, AccountsSummary, ConfirmChangeOutcome, CreateOutcome,
        DisplayName, EmailChangeDecision, EmailChangeOutcome, EmailChangeRequest, NewAccount,
        NewIdentityAccount, NewSession, NewToken, OperationClaim, OperationKey, PasswordChange,
        PasswordChangeOutcome, PurgeCounts, ReceiptOutcome, ResetOutcome, ResetSnapshot,
        RetentionCutoffs, RevokeReason, RevokeScope, SessionRecord, SessionSummary,
        VerificationTarget, VerifyOutcome,
    },
    tokens::{PasswordHash, TokenDigest},
};

/// A store failure. `Unavailable` made no change; `Unknown` may have committed (a lost commit
/// acknowledgement or a deadline during commit) and must never be reported as failure or success.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    Unavailable,
    Unknown,
    /// A stored row failed validation on read (a corrupt or foreign value).
    InvalidRecord,
}

pub type StoreResult<T> = Result<T, StoreError>;

/// The reset policy the store runs inside its transaction, over the loaded snapshot.
pub type ResetPolicy =
    Box<dyn FnOnce(&mut ResetSnapshot) -> Result<(), df_auth::recovery::AccountRefusal> + Send>;

pub trait AccountStore: Send + Sync + 'static {
    /// Inserts the account, its password credential, its verification token and a `SignedUp`
    /// event; `EmailTaken` when the normalized address exists (nothing is written).
    fn create_account(
        &self,
        account: NewAccount,
    ) -> impl Future<Output = StoreResult<CreateOutcome>> + Send;

    /// Inserts an account whose credentials live at an identity provider (no password hash),
    /// with its identity link, its verification token when one is given and a `SignedUp` event,
    /// in one transaction; `EmailTaken` when the address exists (nothing is written).
    fn create_identity_account(
        &self,
        account: NewIdentityAccount,
    ) -> impl Future<Output = StoreResult<CreateOutcome>> + Send;

    /// The account linked to a provider subject.
    fn account_for_identity(
        &self,
        provider: IdentityProviderKind,
        subject: &str,
    ) -> impl Future<Output = StoreResult<Option<AccountRecord>>> + Send;

    /// Links a provider subject to an existing account (idempotent for the same pair).
    fn link_identity(
        &self,
        account: AccountId,
        provider: IdentityProviderKind,
        subject: &str,
        now_ms: u64,
    ) -> impl Future<Output = StoreResult<()>> + Send;

    /// The provider subject linked to an account, if any.
    fn identity_for_account(
        &self,
        account: AccountId,
        provider: IdentityProviderKind,
    ) -> impl Future<Output = StoreResult<Option<String>>> + Send;

    /// Every provider identity linked to an account.
    fn identities_of(
        &self,
        account: AccountId,
    ) -> impl Future<Output = StoreResult<Vec<(IdentityProviderKind, String)>>> + Send;

    /// Drops a provider link after the provider's user was deleted.
    fn unlink_identity(
        &self,
        account: AccountId,
        provider: IdentityProviderKind,
        subject: &str,
    ) -> impl Future<Output = StoreResult<()>> + Send;

    fn account_by_email(
        &self,
        normalized_email: &str,
    ) -> impl Future<Output = StoreResult<Option<AccountRecord>>> + Send;

    fn account(
        &self,
        id: AccountId,
    ) -> impl Future<Output = StoreResult<Option<AccountRecord>>> + Send;

    /// The verified active account's personal tenant, only while its standing is active.
    /// The binding survives account closure and tenant restrictions, but cannot be used then.
    fn tenant_for(
        &self,
        account: AccountId,
    ) -> impl Future<Output = StoreResult<Option<TenantId>>> + Send;

    /// Stores a token and consumes the account's earlier unconsumed tokens of the same purpose.
    fn issue_token(&self, token: NewToken) -> impl Future<Output = StoreResult<()>> + Send;

    /// The open verification link's account, without consuming it: `None` for an unknown,
    /// spent, expired or wrong-purpose token.
    fn verification_target(
        &self,
        digest: TokenDigest,
        now_ms: u64,
    ) -> impl Future<Output = StoreResult<Option<VerificationTarget>>> + Send;

    /// Consumes a verification token and marks the address verified, with a `Verified` event.
    /// Verifying bumps the credential generation and revokes every session of the account (all
    /// of them were created before the proof, so anyone who signed up with someone else's address
    /// cannot keep the account). `replacement` is the password chosen at this step, stored in the
    /// same transaction (local passwords; a provider holds its own).
    fn verify_email(
        &self,
        digest: TokenDigest,
        now_ms: u64,
        replacement: Option<PasswordHash>,
    ) -> impl Future<Output = StoreResult<VerifyOutcome>> + Send;

    /// Inserts a session and, when given, records the event in the same transaction.
    fn create_session(
        &self,
        session: NewSession,
        event: Option<AccountEvent>,
    ) -> impl Future<Output = StoreResult<()>> + Send;

    fn session_by_digest(
        &self,
        digest: TokenDigest,
    ) -> impl Future<Output = StoreResult<Option<(SessionRecord, AccountRecord)>>> + Send;

    fn touch_session(
        &self,
        id: WebSessionId,
        now_ms: u64,
    ) -> impl Future<Output = StoreResult<()>> + Send;

    /// Revokes the account's own sessions only; another account's session id matches nothing.
    /// Returns how many were revoked and records `event` when given.
    fn revoke_sessions(
        &self,
        account: AccountId,
        scope: RevokeScope,
        reason: RevokeReason,
        now_ms: u64,
        event: Option<AccountEvent>,
    ) -> impl Future<Output = StoreResult<u64>> + Send;

    fn sessions(
        &self,
        account: AccountId,
        now_ms: u64,
        idle_before_ms: u64,
    ) -> impl Future<Output = StoreResult<Vec<SessionSummary>>> + Send;

    /// Replaces the hash, bumps the generation, revokes every session of the account and inserts
    /// the replacement session, if the generation is still `expected_generation`.
    fn change_password(
        &self,
        change: PasswordChange,
    ) -> impl Future<Output = StoreResult<PasswordChangeOutcome>> + Send;

    /// Stores a fresh hash of the same password (new cost parameters) if the generation is
    /// still `expected_generation`; sessions and the generation are untouched.
    fn rehash_password(
        &self,
        account: AccountId,
        expected_generation: u64,
        new_hash: PasswordHash,
    ) -> impl Future<Output = StoreResult<()>> + Send;

    /// Loads the reset token's snapshot under lock, runs `policy`, writes the snapshot back and,
    /// when it succeeds, stores `new_hash`, revokes every session and records the event.
    fn reset_password(
        &self,
        digest: TokenDigest,
        new_hash: PasswordHash,
        now_ms: u64,
        policy: ResetPolicy,
    ) -> impl Future<Output = StoreResult<ResetOutcome>> + Send;

    fn request_email_change(
        &self,
        request: EmailChangeRequest,
        now_ms: u64,
    ) -> impl Future<Output = StoreResult<EmailChangeOutcome>> + Send;

    /// Applies one address's decision. When both have confirmed and the address is still free,
    /// the address changes, the generation bumps and every session is revoked.
    fn confirm_email_change(
        &self,
        digest: TokenDigest,
        decision: EmailChangeDecision,
        now_ms: u64,
    ) -> impl Future<Output = StoreResult<ConfirmChangeOutcome>> + Send;

    fn update_profile(
        &self,
        account: AccountId,
        display_name: Option<DisplayName>,
    ) -> impl Future<Output = StoreResult<()>> + Send;

    /// Spends one attempt on a reset token the provider issued (stored by digest when the mail
    /// was queued). Loads the token's snapshot under lock and runs `policy`; when it succeeds the
    /// token is consumed, the credential generation bumps, every session is revoked and
    /// `PasswordReset` is recorded, in one transaction. The password itself stays at the provider.
    fn consume_reset_token(
        &self,
        digest: TokenDigest,
        now_ms: u64,
        policy: ResetPolicy,
    ) -> impl Future<Output = StoreResult<ResetOutcome>> + Send;

    /// The close transaction (J14): sets the account `closing` until `delete_after_ms` and
    /// revokes every session, unless campaigns block (nothing changes) or it was already closing.
    fn close_account(
        &self,
        account: AccountId,
        now_ms: u64,
        delete_after_ms: u64,
    ) -> impl Future<Output = StoreResult<CloseOutcome>> + Send;

    /// Reopens a closing account whose window has not passed (its owner logged in) and cancels
    /// the deletions its closing scheduled, in one transaction. `true` only when exactly one
    /// account row changed; `false` leaves everything as it was and no session may be issued.
    fn reopen_account(
        &self,
        account: AccountId,
        now_ms: u64,
    ) -> impl Future<Output = StoreResult<bool>> + Send;

    /// Closing accounts whose window has passed, oldest first, at most `limit`.
    fn due_closures(
        &self,
        now_ms: u64,
        limit: u32,
    ) -> impl Future<Output = StoreResult<Vec<AccountId>>> + Send;

    /// The completion check and the campaign side of closure for one account that is due: under
    /// the account row lock, re-checks the blockers, then leaves the campaigns in chunks (each its
    /// own transaction). Only `Completed` permits what follows.
    fn complete_closure(
        &self,
        account: AccountId,
        now_ms: u64,
    ) -> impl Future<Output = StoreResult<CompletionOutcome>> + Send;

    /// Deletes the personal data of one completed account (one transaction), if it is still
    /// closing, due and has no provider link left. `true` when it was purged.
    fn purge_closed(
        &self,
        account: AccountId,
        now_ms: u64,
    ) -> impl Future<Output = StoreResult<bool>> + Send;

    fn record_event(
        &self,
        account: AccountId,
        event: AccountEvent,
    ) -> impl Future<Output = StoreResult<()>> + Send;

    /// The most recent events, newest first, at most `limit`.
    fn events(
        &self,
        account: AccountId,
        limit: u32,
    ) -> impl Future<Output = StoreResult<Vec<AccountEvent>>> + Send;

    fn claim_operation(
        &self,
        key: OperationKey,
        fingerprint: [u8; 32],
        now_ms: u64,
    ) -> impl Future<Output = StoreResult<OperationClaim>> + Send;

    fn finish_operation(
        &self,
        key: OperationKey,
        outcome: ReceiptOutcome,
    ) -> impl Future<Output = StoreResult<()>> + Send;

    /// Forgets a claim whose operation was refused, so the corrected form can be sent again.
    fn release_operation(&self, key: OperationKey) -> impl Future<Output = StoreResult<()>> + Send;

    /// Applies the retention cutoffs to sessions, tokens, email changes and receipts. Accounts
    /// are closed out one at a time by `complete_closure` and `purge_closed`.
    fn purge(
        &self,
        cutoffs: RetentionCutoffs,
    ) -> impl Future<Output = StoreResult<PurgeCounts>> + Send;

    fn summary(&self, recent: u32) -> impl Future<Output = StoreResult<AccountsSummary>> + Send;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmailError {
    /// Not delivered and known not delivered.
    Failed,
    /// The deadline passed; delivery is unknown.
    Unknown,
}

pub trait EmailSender: Send + Sync + 'static {
    fn send(&self, message: Message) -> impl Future<Output = Result<(), EmailError>> + Send;
}
