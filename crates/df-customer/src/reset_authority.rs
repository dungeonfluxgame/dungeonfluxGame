//! Runs `df_auth::recovery::recover_credentials` (single use, expiry, bounded attempts,
//! generation bump; G04-D01) over the snapshot a store loaded inside its reset transaction.
//! The store writes the snapshot back in the same transaction.

use std::convert::Infallible;

use df_auth::{
    recovery::{
        AccountAuthority, AccountOperation, AccountOutcome, AccountReceipt, AccountRefusal,
        LinkRecord, LinkRequest, RecoveryRecord, RecoveryRequest, recover_credentials,
    },
    rotation::CredentialFence,
};
use df_types::{AccountId, OperationId};

use crate::{records::ResetSnapshot, tokens::TokenDigest};

/// One reset token's facts, borrowed for the duration of the policy call.
struct SnapshotAuthority<'a> {
    snapshot: &'a mut ResetSnapshot,
    challenge: TokenDigest,
}

impl AccountAuthority for SnapshotAuthority<'_> {
    type Scope = ();
    type Principal = AccountId;
    type Lookup = ();
    type Challenge = TokenDigest;
    type Proof = ();
    type Fingerprint = ();
    type Error = Infallible;

    fn request_challenge(&mut self, _lookup: &()) -> Result<(), Infallible> {
        Ok(())
    }

    fn with_recovery(
        &mut self,
        _request: &RecoveryRequest<'_, Self>,
        apply: impl FnOnce(
            RecoveryRecord<'_, Self>,
        ) -> Result<AccountReceipt<AccountId>, AccountRefusal>,
    ) -> AccountOutcome<AccountId, Infallible> {
        let mut credential = CredentialFence::new(self.snapshot.credential_generation);
        if !self.snapshot.is_account_active {
            credential.revoke();
        }
        let mut attempts = self.snapshot.attempts;
        let mut is_consumed = self.snapshot.is_consumed;
        let mut prior_bindings_active = true;
        let principal = self.snapshot.account;
        let result = apply(RecoveryRecord {
            scope: &(),
            principal: &principal,
            // An email-and-password account is its own linked account.
            linked_account: true,
            challenge: &self.challenge,
            challenge_principal: &principal,
            // The token was delivered to the account's address: holding it proves the channel.
            channel_verified: true,
            // The new password already passed the policy before the transaction began.
            proof_verified: true,
            recovery_purpose: self.snapshot.purpose_is_reset,
            expires_at: self.snapshot.expires_at_ms,
            attempts: &mut attempts,
            attempt_limit: self.snapshot.attempt_limit,
            consumed: &mut is_consumed,
            credential: &mut credential,
            prior_bindings_active: &mut prior_bindings_active,
        });
        self.snapshot.attempts = attempts;
        self.snapshot.is_consumed = is_consumed;
        self.snapshot.credential_generation = *credential.generation();
        match result {
            Ok(receipt) => AccountOutcome::Committed {
                receipt,
                replayed: false,
            },
            Err(refusal) => AccountOutcome::Refused(refusal),
        }
    }

    fn with_link(
        &mut self,
        _request: &LinkRequest<'_, Self>,
        _apply: impl FnOnce(LinkRecord<'_, Self>) -> Result<AccountReceipt<AccountId>, AccountRefusal>,
    ) -> AccountOutcome<AccountId, Infallible> {
        // Guest linking is not part of password reset.
        AccountOutcome::Refused(AccountRefusal::NotGuest)
    }

    fn lookup(
        &mut self,
        _operation: &AccountOperation<'_, Self>,
    ) -> AccountOutcome<AccountId, Infallible> {
        AccountOutcome::Pending
    }
}

/// Applies the recovery policy to `snapshot` at `now_ms`. On success the snapshot carries the
/// bumped generation and the consumed token; on refusal, any attempt it counted.
pub(crate) fn apply_reset_policy(
    snapshot: &mut ResetSnapshot,
    challenge: TokenDigest,
    operation: OperationId,
    now_ms: u64,
) -> Result<(), AccountRefusal> {
    let mut authority = SnapshotAuthority {
        snapshot,
        challenge,
    };
    let request = RecoveryRequest {
        operation: AccountOperation {
            scope: &(),
            key: operation,
            fingerprint: &(),
        },
        challenge: &challenge,
        proof: &(),
    };
    match recover_credentials(&mut authority, &request, now_ms) {
        AccountOutcome::Committed { .. } => Ok(()),
        AccountOutcome::Refused(refusal) => Err(refusal),
        // The snapshot authority answers every call synchronously and cannot fail.
        AccountOutcome::Unavailable(never) | AccountOutcome::Unknown(never) => match never {},
        AccountOutcome::Pending => Err(AccountRefusal::Conflict),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> ResetSnapshot {
        ResetSnapshot {
            account: AccountId::from_bytes(&[3; 16]).unwrap(),
            purpose_is_reset: true,
            expires_at_ms: 1_000,
            attempts: 0,
            attempt_limit: 5,
            is_consumed: false,
            credential_generation: 7,
            is_account_active: true,
        }
    }

    fn operation() -> OperationId {
        OperationId::from_bytes(&[9; 16]).unwrap()
    }

    const CHALLENGE: TokenDigest = TokenDigest::from_bytes([4; 32]);

    #[test]
    fn a_current_reset_consumes_the_token_and_bumps_the_generation() {
        let mut current = snapshot();
        assert_eq!(
            apply_reset_policy(&mut current, CHALLENGE, operation(), 999),
            Ok(())
        );
        assert!(current.is_consumed);
        assert_eq!(current.credential_generation, 8);
        assert_eq!(current.attempts, 1);
    }

    #[test]
    fn a_used_token_is_refused_and_changes_nothing() {
        let mut used = ResetSnapshot {
            is_consumed: true,
            ..snapshot()
        };
        assert_eq!(
            apply_reset_policy(&mut used, CHALLENGE, operation(), 10),
            Err(AccountRefusal::ChallengeUsed)
        );
        assert_eq!(used.credential_generation, 7);
    }

    #[test]
    fn an_expired_token_is_refused_at_its_expiry() {
        let mut current = snapshot();
        assert_eq!(
            apply_reset_policy(&mut current, CHALLENGE, operation(), 1_000),
            Err(AccountRefusal::Expired)
        );
    }

    #[test]
    fn a_token_past_its_attempt_limit_is_refused() {
        let mut exhausted = ResetSnapshot {
            attempts: 5,
            ..snapshot()
        };
        assert_eq!(
            apply_reset_policy(&mut exhausted, CHALLENGE, operation(), 10),
            Err(AccountRefusal::AttemptsExhausted)
        );
    }

    #[test]
    fn a_token_of_another_purpose_is_refused() {
        let mut verification = ResetSnapshot {
            purpose_is_reset: false,
            ..snapshot()
        };
        assert_eq!(
            apply_reset_policy(&mut verification, CHALLENGE, operation(), 10),
            Err(AccountRefusal::WrongPurpose)
        );
    }
}
