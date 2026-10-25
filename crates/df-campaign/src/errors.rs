//! Typed failures. A refusal is a decision the person can act on; a store error is trouble they
//! can only wait out. An uncertain commit is `Unknown`, never shown as failure or success.

use crate::{
    display_pairing::{PairingRefusal, TicketRefusal},
    join_invitation::JoinFailure,
};

/// The store could not do what was asked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    /// Nothing changed.
    Unavailable,
    /// The commit may or may not have happened (a lost acknowledgement, a session terminated
    /// during COMMIT). Look the operation up before repeating it.
    Unknown,
    /// A stored row failed validation on read.
    InvalidRecord,
    /// A deadlock or lock timeout; nothing changed and the call may be repeated.
    Retryable,
    /// A halt of the service is in progress (`DF002`); retry after it.
    ServiceHalting,
    /// The caller's owner fence is not current.
    FenceStale,
    /// A method called inside a caller's transaction found no usable `transaction_timeout`.
    TimeoutNotSet,
    /// The random source failed; nothing secret was issued.
    RandomUnavailable,
}

pub type StoreResult<T> = Result<T, StoreError>;

/// A decision, in a closed set the wire layer maps to a `Reason`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// A missing id, a foreign id and an id of another campaign all look like this.
    NotFound,
    /// The caller's role, delegations or rank do not allow it.
    NotAllowed,
    OwnerOnly,
    ConfirmationRequired,
    Join(JoinFailure),
    InvitationUnknown,
    CodeUnknown,
    AllowBackFirst,
    RequestsFull,
    StaleRevision,
    CampaignUnavailable,
    AccountClosing,
    SessionInProgress,
    SessionEnded,
    HostAccountNotActive,
    HostMustTransferFirst,
    OwnerMustTransferFirst,
    CannotRemoveHost,
    CannotRemoveOwner,
    TargetOutranks,
    DeletionScheduled,
    AlreadyDeleting,
    DisplayLimit,
    /// Every pairing failure a display may not tell apart.
    DisplayNeutral,
    Pairing(PairingRefusal),
    Ticket(TicketRefusal),
    OperationConflict,
    InvalidInput,
    SeatsFull,
    ContributorsFull,
    NotSeated,
    ContributorCannotBeCoHost,
    TransferTargetNotAllowed,
    Throttled,
    OperationExpired,
    ProposalExpired,
    OwnerAccountNotActive,
    TooManyCampaigns,
    /// The creator's email is not verified (decision 12); drafts cannot be started.
    EmailUnverified,
    /// The account already holds `drafts_per_account` unfinished drafts.
    DraftsFull,
    /// Finish was asked before every step was saved.
    DraftIncomplete,
    /// Ready was asked before a character was chosen.
    CharacterNotChosen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failure {
    Refused(Refusal),
    Store(StoreError),
}

impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<Refusal> for Failure {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

pub type Outcome<T> = Result<T, Failure>;
