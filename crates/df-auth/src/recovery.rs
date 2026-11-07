//! Owner-bound account recovery and guest linking; no credential or channel issuer.
//!
//! The native owner supplies independently verified current facts and serializes
//! policy, affected grants and the operation receipt together. This crate neither
//! implements durable storage nor authenticates a caller-supplied email or flag.

use df_types::{MemberId, OperationId};

use crate::rotation::{CredentialFence, RotationRefusal};

/// Stable intent in the owner's account-command namespace, never identity proof.
pub struct AccountOperation<'a, A: AccountAuthority> {
    pub scope: &'a A::Scope,
    pub key: OperationId,
    pub fingerprint: &'a A::Fingerprint,
}

/// Opaque proof and challenge stay within the authentication owner.
pub struct RecoveryRequest<'a, A: AccountAuthority> {
    pub operation: AccountOperation<'a, A>,
    pub challenge: &'a A::Challenge,
    pub proof: &'a A::Proof,
}

/// Selection is explicit; possession of a member ID does not authorize transfer.
pub struct LinkRequest<'a, A: AccountAuthority> {
    pub operation: AccountOperation<'a, A>,
    pub guest: &'a A::Principal,
    pub account: &'a A::Principal,
    pub guest_proof: &'a A::Proof,
    pub account_proof: &'a A::Proof,
    pub members: &'a [MemberId],
    pub expected_revision: u64,
}

/// Borrowed current facts, produced only after the owner verifies the request.
/// Expiry and the caller's trusted time use the same owner-selected clock units.
pub struct RecoveryRecord<'a, A: AccountAuthority> {
    pub scope: &'a A::Scope,
    pub principal: &'a A::Principal,
    pub linked_account: bool,
    pub challenge: &'a A::Challenge,
    pub challenge_principal: &'a A::Principal,
    pub channel_verified: bool,
    pub proof_verified: bool,
    pub recovery_purpose: bool,
    pub expires_at: u64,
    pub attempts: &'a mut u32,
    pub attempt_limit: u32,
    pub consumed: &'a mut bool,
    pub credential: &'a mut CredentialFence<u64>,
    pub prior_bindings_active: &'a mut bool,
}

/// Borrow a membership's actual owner field; this is not another seat registry.
pub struct LinkMembership<'a, P> {
    pub member: MemberId,
    pub principal: &'a mut P,
}

/// Both proofs must bind current identity, credential generation and purpose.
/// The membership owner supplies one consistent revision and a finite selection
/// bound; the callback must not unlock, await or publish private data.
pub struct LinkRecord<'a, A: AccountAuthority> {
    pub scope: &'a A::Scope,
    pub guest: &'a A::Principal,
    pub account: &'a A::Principal,
    pub guest_is_guest: bool,
    pub account_verified: bool,
    pub guest_proof_verified: bool,
    pub account_proof_verified: bool,
    pub guest_credential: &'a mut CredentialFence<u64>,
    pub guest_bindings_active: &'a mut bool,
    pub linked_account: &'a mut Option<A::Principal>,
    pub revision: &'a mut u64,
    pub selection_limit: usize,
    pub memberships: &'a mut [LinkMembership<'a, A::Principal>],
}

/// Receipts deliberately have no default diagnostic representation.
#[derive(Clone)]
pub enum AccountReceipt<P> {
    Recovered {
        principal: P,
        credential_generation: u64,
    },
    Linked {
        account: P,
        membership_revision: u64,
        transferred: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountRefusal {
    WrongScope,
    WrongChallenge,
    WrongIdentity,
    UnlinkedGuest,
    NoRecoveryChannel,
    WrongPurpose,
    UnverifiedProof,
    Expired,
    ChallengeUsed,
    AttemptsExhausted,
    CredentialRevoked,
    GenerationExhausted,
    NotGuest,
    UnverifiedAccount,
    StaleRevision,
    RevisionExhausted,
    SelectionTooLarge,
    InvalidSelection,
    MembershipDenied,
    AlreadyLinked,
    Conflict,
    Retired,
}

/// Unknown never acknowledges a credential or transfer. Lookup must preserve
/// the original scoped key and fingerprint, not create a fresh command.
pub enum AccountOutcome<P, E> {
    Committed {
        receipt: AccountReceipt<P>,
        replayed: bool,
    },
    Refused(AccountRefusal),
    Unavailable(E),
    Unknown(E),
    Pending,
}

/// Safe classification excludes account existence, identities and error payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountObservation {
    Committed,
    Replayed,
    Refused,
    Unavailable,
    Unknown,
    Pending,
}

impl<P, E> AccountOutcome<P, E> {
    pub fn observation(&self) -> AccountObservation {
        match self {
            Self::Committed { replayed: true, .. } => AccountObservation::Replayed,
            Self::Committed { .. } => AccountObservation::Committed,
            Self::Refused(_) => AccountObservation::Refused,
            Self::Unavailable(_) => AccountObservation::Unavailable,
            Self::Unknown(_) => AccountObservation::Unknown,
            Self::Pending => AccountObservation::Pending,
        }
    }
}

/// Identical public start response: no promise of existence or channel delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryRequested;

/// Serialized account-command boundary, consistent with GuestBootstrapStore.
///
/// Identities, lookup, proof, scope and fingerprint are the actual owner's types.
/// The owner independently verifies issuer/channel and proof bound to the exact
/// challenge, account-command scope, purpose, active credential and current
/// generation. Both linking proofs must be current and unrevoked. Request fields
/// or a trace cannot authenticate these records. The owner resolves an
/// exact scoped-key replay BEFORE invoking a callback on a consumed challenge.
/// Canonical fingerprints bind the complete command, proofs, selection and basis.
/// Changed fingerprint/command namespace returns Conflict; retired keys remain
/// fenced. A new callback is invoked once, without unlock or reentrancy.
///
/// The owner commits challenge accounting, credential/binding fencing, selected
/// membership changes and the stable result atomically. A refused proof retains
/// its bounded attempt accounting, while no credential/transfer is acknowledged.
/// Failed or uncertain commits return Unavailable/Unknown, never Committed.
/// The owner supplies bounded synchronous calls and rollback/commit semantics;
/// this port does not supply a cryptographic verifier, channel or durable store.
pub trait AccountAuthority: Sized {
    type Scope: Eq;
    type Principal: Clone + Eq;
    type Lookup;
    type Challenge: Eq;
    type Proof;
    type Fingerprint;
    type Error;

    /// Same response semantics for absent, present and failed channel requests.
    /// Independently retain safe operational failure facts; never log lookup or
    /// proof contents. Neither success nor failure here promises delivery.
    fn request_challenge(&mut self, lookup: &Self::Lookup) -> Result<(), Self::Error>;

    fn with_recovery(
        &mut self,
        request: &RecoveryRequest<'_, Self>,
        apply: impl FnOnce(
            RecoveryRecord<'_, Self>,
        ) -> Result<AccountReceipt<Self::Principal>, AccountRefusal>,
    ) -> AccountOutcome<Self::Principal, Self::Error>;

    fn with_link(
        &mut self,
        request: &LinkRequest<'_, Self>,
        apply: impl FnOnce(
            LinkRecord<'_, Self>,
        ) -> Result<AccountReceipt<Self::Principal>, AccountRefusal>,
    ) -> AccountOutcome<Self::Principal, Self::Error>;

    /// An absent local result stays Pending; it does not prove a failed commit.
    fn lookup(
        &mut self,
        operation: &AccountOperation<'_, Self>,
    ) -> AccountOutcome<Self::Principal, Self::Error>;
}

/// Suppress account enumeration on every start outcome, including source errors.
/// Operational failures remain the owner's separate safe diagnostic concern.
pub fn request_recovery<A: AccountAuthority>(
    authority: &mut A,
    lookup: &A::Lookup,
) -> RecoveryRequested {
    // The result is ignored on purpose: the answer is the same whether or not the account exists
    // or the source failed, so a caller cannot enumerate accounts.
    let _ = authority.request_challenge(lookup);
    RecoveryRequested
}

/// Apply one verified recovery inside the owner's atomic operation boundary.
/// No retry, clock read, secret issuance or fallback guest allocation occurs.
pub fn recover_credentials<A: AccountAuthority>(
    authority: &mut A,
    request: &RecoveryRequest<'_, A>,
    now: u64,
) -> AccountOutcome<A::Principal, A::Error> {
    authority.with_recovery(request, |record| {
        if record.scope != request.operation.scope {
            return Err(AccountRefusal::WrongScope);
        }
        if !record.linked_account {
            return Err(AccountRefusal::UnlinkedGuest);
        }
        if !record.channel_verified {
            return Err(AccountRefusal::NoRecoveryChannel);
        }
        if record.challenge != request.challenge {
            return Err(AccountRefusal::WrongChallenge);
        }
        if record.challenge_principal != record.principal {
            return Err(AccountRefusal::WrongIdentity);
        }
        if !record.recovery_purpose {
            return Err(AccountRefusal::WrongPurpose);
        }
        if *record.consumed {
            return Err(AccountRefusal::ChallengeUsed);
        }
        if now >= record.expires_at {
            return Err(AccountRefusal::Expired);
        }
        if record.attempt_limit == 0 || *record.attempts >= record.attempt_limit {
            return Err(AccountRefusal::AttemptsExhausted);
        }
        if !record.credential.is_active() {
            return Err(AccountRefusal::CredentialRevoked);
        }
        let next = record
            .credential
            .generation()
            .checked_add(1)
            .ok_or(AccountRefusal::GenerationExhausted)?;
        // Constructor-independent bound: attempts < nonzero u32 limit.
        *record.attempts += 1;
        if !record.proof_verified {
            return Err(AccountRefusal::UnverifiedProof);
        }
        record
            .credential
            .rotate(next)
            .map_err(|refusal| match refusal {
                RotationRefusal::Revoked => AccountRefusal::CredentialRevoked,
                RotationRefusal::NonIncreasingGeneration => AccountRefusal::GenerationExhausted,
            })?;
        *record.prior_bindings_active = false;
        *record.consumed = true;
        Ok(AccountReceipt::Recovered {
            principal: record.principal.clone(),
            credential_generation: next,
        })
    })
}

/// Link only the explicitly selected current memberships after proof of both
/// identities. No destination generation, payer grant or campaign is allocated.
pub fn link_guest<A: AccountAuthority>(
    authority: &mut A,
    request: &LinkRequest<'_, A>,
) -> AccountOutcome<A::Principal, A::Error> {
    authority.with_link(request, |record| {
        if record.scope != request.operation.scope {
            return Err(AccountRefusal::WrongScope);
        }
        if record.guest != request.guest || record.account != request.account {
            return Err(AccountRefusal::WrongIdentity);
        }
        if record.guest == record.account {
            return Err(AccountRefusal::WrongIdentity);
        }
        if !record.guest_is_guest {
            return Err(AccountRefusal::NotGuest);
        }
        if !record.account_verified {
            return Err(AccountRefusal::UnverifiedAccount);
        }
        if !record.guest_credential.is_active() {
            return Err(AccountRefusal::CredentialRevoked);
        }
        if !record.guest_proof_verified || !record.account_proof_verified {
            return Err(AccountRefusal::UnverifiedProof);
        }
        if record.linked_account.is_some() {
            return Err(AccountRefusal::AlreadyLinked);
        }
        if *record.revision != request.expected_revision {
            return Err(AccountRefusal::StaleRevision);
        }
        let next = record
            .revision
            .checked_add(1)
            .ok_or(AccountRefusal::RevisionExhausted)?;
        if request.members.len() > record.selection_limit {
            return Err(AccountRefusal::SelectionTooLarge);
        }
        for (index, member) in request.members.iter().enumerate() {
            if request.members[..index].contains(member) {
                return Err(AccountRefusal::InvalidSelection);
            }
            let mut matches = record
                .memberships
                .iter()
                .filter(|row| row.member == *member);
            let row = matches.next().ok_or(AccountRefusal::MembershipDenied)?;
            if matches.next().is_some() || &*row.principal != request.guest {
                return Err(AccountRefusal::MembershipDenied);
            }
        }
        // Validate the entire selection before touching any actual owner field.
        for row in record.memberships.iter_mut() {
            if request.members.contains(&row.member) {
                *row.principal = record.account.clone();
            }
        }
        *record.linked_account = Some(record.account.clone());
        *record.revision = next;
        record.guest_credential.revoke();
        *record.guest_bindings_active = false;
        Ok(AccountReceipt::Linked {
            account: record.account.clone(),
            membership_revision: next,
            transferred: request.members.len(),
        })
    })
}
