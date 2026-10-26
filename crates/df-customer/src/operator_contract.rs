//! Durable operator authority contracts. Values alone confer no caller authorization.
//! MFA cryptography and HTTP admission belong to their respective adapters.

use df_types::{AccountId, OperationId, TenantId, WebSessionId};
use sha2::{Digest, Sha256};

/// Maximum lifetime of a newly issued operator grant, in milliseconds.
pub const MAXIMUM_OPERATOR_GRANT_MS: u64 = 30 * 24 * 60 * 60 * 1_000;
/// Maximum number of rows returned by an operator projection.
pub const MAXIMUM_OPERATOR_PAGE: u16 = 100;

/// Explicit independent powers; no capability implies another capability.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(i16)]
pub enum OperatorCapability {
    SupportInspect = 1,
    EmailReveal,
    AccountRestriction,
    SessionInspect,
    SessionTerminate,
    Refund,
    GoodwillCredit,
    SupportTriage,
    SupportContent,
    ContentReview,
    AuditInspect,
}

impl OperatorCapability {
    pub fn from_code(code: i16) -> Option<Self> {
        match code {
            1 => Some(Self::SupportInspect),
            2 => Some(Self::EmailReveal),
            3 => Some(Self::AccountRestriction),
            4 => Some(Self::SessionInspect),
            5 => Some(Self::SessionTerminate),
            6 => Some(Self::Refund),
            7 => Some(Self::GoodwillCredit),
            8 => Some(Self::SupportTriage),
            9 => Some(Self::SupportContent),
            10 => Some(Self::ContentReview),
            11 => Some(Self::AuditInspect),
            _ => None,
        }
    }
}

/// A nonempty canonical set, bounded by the closed capability catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorCapabilities(Vec<OperatorCapability>);

impl OperatorCapabilities {
    pub fn new(mut capabilities: Vec<OperatorCapability>) -> Result<Self, OperatorError> {
        if capabilities.is_empty() || capabilities.len() > 11 {
            return Err(OperatorError::InvalidCapabilities);
        }
        capabilities.sort_unstable();
        capabilities.dedup();
        Ok(Self(capabilities))
    }

    pub fn as_slice(&self) -> &[OperatorCapability] {
        &self.0
    }

    pub fn contains(&self, capability: OperatorCapability) -> bool {
        self.0.binary_search(&capability).is_ok()
    }
}

/// The resource boundary at which a capability may be exercised.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatorScope {
    Global,
    Account(AccountId),
    Tenant(TenantId),
}

impl OperatorScope {
    pub fn allows(self, requested: Self) -> bool {
        self == Self::Global || self == requested
    }

    pub fn parts(self) -> (i16, Option<[u8; 16]>) {
        match self {
            Self::Global => (0, None),
            Self::Account(id) => (1, Some(*id.as_bytes())),
            Self::Tenant(id) => (2, Some(*id.as_bytes())),
        }
    }
}

/// A grant identity shares its creating operation's stable identity, without a new registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperatorGrantId(pub OperationId);

/// A normalized reason or support reference, containing 1..500 non-control characters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorReason(String);

impl OperatorReason {
    pub fn parse(reason: &str) -> Result<Self, OperatorError> {
        let reason = reason.trim();
        if reason.is_empty() || reason.chars().count() > 500 || reason.chars().any(char::is_control)
        {
            return Err(OperatorError::InvalidReason);
        }
        Ok(Self(reason.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperatorFingerprint(pub [u8; 32]);

/// Session facts must be loaded from durable server authority, never browser claims.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperatorSessionBasis {
    pub account: AccountId,
    pub session: WebSessionId,
    pub credential_generation: u64,
    pub enrolment_generation: u64,
    pub proof: OperationId,
    pub verified_at_ms: u64,
    pub grant: OperatorGrantId,
    pub grant_version: u64,
}

/// Host-only bootstrap has no web session; protected web actions retain the exact proof basis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatorAuditBasis {
    Host,
    Session(OperatorSessionBasis),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i16)]
pub enum OperatorAuditAction {
    Grant = 1,
    Revoke,
    PrepareEnrolment,
    ConfirmEnrolment,
    VerifyMfa,
    RevokeProof,
    InspectAccount,
    RevealEmail,
    RestrictAccount,
    InspectSession,
    TerminateSession,
    Refund,
    GoodwillCredit,
    TriageSupport,
    ReadSupport,
    ReviewContent,
    InspectAudit,
}

impl OperatorAuditAction {
    pub fn from_code(code: i16) -> Option<Self> {
        match code {
            1 => Some(Self::Grant),
            2 => Some(Self::Revoke),
            3 => Some(Self::PrepareEnrolment),
            4 => Some(Self::ConfirmEnrolment),
            5 => Some(Self::VerifyMfa),
            6 => Some(Self::RevokeProof),
            7 => Some(Self::InspectAccount),
            8 => Some(Self::RevealEmail),
            9 => Some(Self::RestrictAccount),
            10 => Some(Self::InspectSession),
            11 => Some(Self::TerminateSession),
            12 => Some(Self::Refund),
            13 => Some(Self::GoodwillCredit),
            14 => Some(Self::TriageSupport),
            15 => Some(Self::ReadSupport),
            16 => Some(Self::ReviewContent),
            17 => Some(Self::InspectAudit),
            _ => None,
        }
    }
}

/// External owner ids remain opaque bounded references, never private content or URLs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorTargetReference(String);

impl OperatorTargetReference {
    pub fn parse(value: &str) -> Result<Self, OperatorError> {
        if value.is_empty()
            || value.len() > 128
            || !value.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(OperatorError::InvalidTarget);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperatorAuditTarget {
    Account(AccountId),
    Grant(OperatorGrantId),
    WebSession(WebSessionId),
    RuntimeSession(df_types::SessionId),
    Payment(OperationId),
    SupportTicket(OperatorTargetReference),
    Content(OperatorTargetReference),
    Audit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i16)]
pub enum OperatorAuditOutcome {
    Applied = 1,
    Refused,
    Unavailable,
    Unknown,
}

/// Redacted durable audit metadata; no credentials, secret bytes or private bodies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewOperatorAudit {
    pub basis: OperatorAuditBasis,
    pub action: OperatorAuditAction,
    pub target: OperatorAuditTarget,
    pub reason: OperatorReason,
    pub operation: OperationId,
    pub fingerprint: OperatorFingerprint,
    pub outcome: OperatorAuditOutcome,
    pub correlation: OperationId,
    pub at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OperatorAuditId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorAuditRecord {
    pub id: OperatorAuditId,
    pub event: NewOperatorAudit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorGrant {
    pub id: OperatorGrantId,
    pub account: AccountId,
    pub capabilities: OperatorCapabilities,
    pub scope: OperatorScope,
    pub version: u64,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub revoked_at_ms: Option<u64>,
    pub enrolment_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantOperator {
    pub account: AccountId,
    pub capabilities: OperatorCapabilities,
    pub scope: OperatorScope,
    pub expires_at_ms: u64,
    pub reason: OperatorReason,
    pub operation: OperationId,
    pub correlation: OperationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevokeOperator {
    pub grant: OperatorGrantId,
    pub account: AccountId,
    pub expected_version: u64,
    pub reason: OperatorReason,
    pub operation: OperationId,
    pub correlation: OperationId,
}

/// Host-only commands; adapters must not expose this authority through HTTP.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperatorCommand {
    Grant(GrantOperator),
    Revoke(RevokeOperator),
}

impl OperatorCommand {
    pub fn account(&self) -> AccountId {
        match self {
            Self::Grant(value) => value.account,
            Self::Revoke(value) => value.account,
        }
    }

    pub fn operation(&self) -> OperationId {
        match self {
            Self::Grant(value) => value.operation,
            Self::Revoke(value) => value.operation,
        }
    }

    /// Canonical server-computed binding excludes retry time and diagnostic correlation.
    pub fn fingerprint(&self) -> OperatorFingerprint {
        let mut hash = Sha256::new();
        hash.update(b"dungeonflux.operator-command/1");
        hash.update(self.account().as_bytes());
        match self {
            Self::Grant(value) => {
                hash.update([1]);
                let (kind, id) = value.scope.parts();
                hash.update(kind.to_be_bytes());
                if let Some(id) = id {
                    hash.update(id);
                }
                hash.update(value.expires_at_ms.to_be_bytes());
                for capability in value.capabilities.as_slice() {
                    hash.update((*capability as i16).to_be_bytes());
                }
                hash.update([0]);
                hash.update(value.reason.as_str().as_bytes());
            }
            Self::Revoke(value) => {
                hash.update([2]);
                hash.update(value.grant.0.as_bytes());
                hash.update(value.expected_version.to_be_bytes());
                hash.update(value.reason.as_str().as_bytes());
            }
        }
        OperatorFingerprint(hash.finalize().into())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperatorReceipt {
    pub operation: OperationId,
    pub grant: OperatorGrantId,
    pub version: u64,
    pub audit: OperatorAuditId,
}

/// Definite refusals are separate from storage loss and uncertain commit acknowledgement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatorError {
    InvalidReason,
    InvalidCapabilities,
    InvalidExpiry,
    InvalidTarget,
    InvalidPage,
    AccountNotVerified,
    ConfirmedMfaRequired,
    NotFound,
    StaleVersion,
    PayloadConflict,
    Unavailable,
    Unknown,
    InvalidRecord,
}

/// A validated keyset page size; zero and unbounded requests are refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperatorPageSize(u16);

impl OperatorPageSize {
    pub fn new(limit: u16) -> Result<Self, OperatorError> {
        if limit == 0 || limit > MAXIMUM_OPERATOR_PAGE {
            return Err(OperatorError::InvalidPage);
        }
        Ok(Self(limit))
    }
    pub fn get(self) -> u16 {
        self.0
    }
}

/// Sealed material has deliberately no Debug implementation; AF02 owns sealing and activation.
#[derive(Clone)]
pub struct SealedOperatorSecret {
    pub key_id: String,
    pub ciphertext: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatorMfaState {
    Pending,
    Active,
    Revoked,
}

#[derive(Clone)]
pub struct OperatorMfaRecord {
    pub account: AccountId,
    pub generation: u64,
    pub credential_generation: u64,
    pub state: OperatorMfaState,
    pub sealed: SealedOperatorSecret,
    pub created_at_ms: u64,
    pub confirmed_at_ms: Option<u64>,
    pub last_used_step: Option<u64>,
}

#[derive(Clone)]
pub struct OperatorEnrolmentTicket {
    pub digest: crate::TokenDigest,
    pub account: AccountId,
    pub enrolment_generation: u64,
    pub credential_generation: u64,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub consumed_at_ms: Option<u64>,
}

/// A sensitive-action proof is bound to one exact command and consumed atomically by its owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorActionProof {
    pub id: OperationId,
    pub basis: OperatorSessionBasis,
    pub operation: OperationId,
    pub action: OperatorAuditAction,
    pub target: OperatorAuditTarget,
    pub fingerprint: OperatorFingerprint,
    pub expires_at_ms: u64,
    pub consumed_at_ms: Option<u64>,
}
