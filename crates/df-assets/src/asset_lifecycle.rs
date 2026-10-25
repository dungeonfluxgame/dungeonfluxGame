//! The asset lifecycle (plan 13.3; planning/asset-storage-research.md 6.1): the closed transition
//! table every catalog adapter enforces, the terminal reasons reported to F's settlement, and the
//! orphan-sweep deletion rule (6.2). Pure: adapters apply these under their own row locks.

use crate::media_ids::{AttemptId, ClaimToken};

/// Stored lifecycle state of one asset row.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AssetState {
    Uploading,
    Quarantined,
    Validated,
    Ready,
    Archived,
    Deleted,
    Purging,
    Purged,
    Rejected,
}

/// One requested change of state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transition {
    /// The first complete staging write was recorded.
    Stage,
    Validate,
    /// Master and variants are written; the row becomes deliverable.
    Finalize,
    Reject,
    Archive,
    Restore,
    Tombstone,
    BeginPurge,
    CompletePurge,
}

/// The pair is not in the 6.1 table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IllegalTransition {
    pub from: AssetState,
    pub transition: Transition,
}

const STATES: [AssetState; 9] = [
    AssetState::Uploading,
    AssetState::Quarantined,
    AssetState::Validated,
    AssetState::Ready,
    AssetState::Archived,
    AssetState::Deleted,
    AssetState::Purging,
    AssetState::Purged,
    AssetState::Rejected,
];

impl AssetState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Uploading => "uploading",
            Self::Quarantined => "quarantined",
            Self::Validated => "validated",
            Self::Ready => "ready",
            Self::Archived => "archived",
            Self::Deleted => "deleted",
            Self::Purging => "purging",
            Self::Purged => "purged",
            Self::Rejected => "rejected",
        }
    }

    /// The state a stored label names; `None` for any other text.
    pub fn from_label(label: &str) -> Option<Self> {
        STATES.into_iter().find(|state| state.label() == label)
    }

    /// The row has been deliverable at least once (`ready_at_ms` is set).
    pub fn has_reached_ready(self) -> bool {
        matches!(
            self,
            Self::Ready | Self::Archived | Self::Deleted | Self::Purging | Self::Purged
        )
    }

    /// A live row: at most one per request key (the partial unique index).
    pub fn is_live(self) -> bool {
        matches!(
            self,
            Self::Uploading | Self::Quarantined | Self::Validated | Self::Ready
        )
    }

    /// The table of 6.1. A `Validated` row may still be rejected: its recorded staged object can
    /// be lost before publication (`staged_lost`).
    pub fn after(self, transition: Transition) -> Result<Self, IllegalTransition> {
        use AssetState as S;
        use Transition as T;
        let next = match (self, transition) {
            (S::Uploading, T::Stage) => S::Quarantined,
            (S::Uploading | S::Quarantined | S::Validated, T::Reject) => S::Rejected,
            (S::Quarantined, T::Validate) => S::Validated,
            (S::Validated, T::Finalize) => S::Ready,
            (S::Ready, T::Archive) => S::Archived,
            (S::Archived, T::Restore) => S::Ready,
            (S::Ready | S::Archived, T::Tombstone) => S::Deleted,
            (S::Deleted, T::BeginPurge) => S::Purging,
            (S::Purging, T::CompletePurge) => S::Purged,
            _ => {
                return Err(IllegalTransition {
                    from: self,
                    transition,
                });
            }
        };
        Ok(next)
    }
}

/// Why an attempt ended without a deliverable asset. Closed, so audit lines carry no free text.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RejectReason {
    /// Decoding, bounds or format validation failed.
    ValidationFailed,
    ScreenRejected,
    /// The upload ticket expired without a finalize request.
    Abandoned,
    /// F cancelled the unclaimed effect: nothing was sent.
    NotDispatched,
    /// The supplier reported a failed result.
    SupplierFailed,
    /// F's reconciliation limit passed with the outcome still unknown (D25).
    UnknownOutcomeFailed,
    /// The dispatcher answered `OutputGone`.
    OutputLost,
    /// The staging stream exceeded the kind's maximum bytes.
    OverMaximumBytes,
    ReviewExpired,
    ScreenUnavailable,
    StagedExpired,
    /// The recorded staged object was missing or failed its digest check.
    StagedLost,
    /// An upload's actual bytes did not fit the quota at commit.
    QuotaExceeded,
}

const REASONS: [RejectReason; 13] = [
    RejectReason::ValidationFailed,
    RejectReason::ScreenRejected,
    RejectReason::Abandoned,
    RejectReason::NotDispatched,
    RejectReason::SupplierFailed,
    RejectReason::UnknownOutcomeFailed,
    RejectReason::OutputLost,
    RejectReason::OverMaximumBytes,
    RejectReason::ReviewExpired,
    RejectReason::ScreenUnavailable,
    RejectReason::StagedExpired,
    RejectReason::StagedLost,
    RejectReason::QuotaExceeded,
];

impl RejectReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::ValidationFailed => "validation_failed",
            Self::ScreenRejected => "screen_rejected",
            Self::Abandoned => "abandoned",
            Self::NotDispatched => "not_dispatched",
            Self::SupplierFailed => "supplier_failed",
            Self::UnknownOutcomeFailed => "unknown_outcome_failed",
            Self::OutputLost => "output_lost",
            Self::OverMaximumBytes => "over_maximum_bytes",
            Self::ReviewExpired => "review_expired",
            Self::ScreenUnavailable => "screen_unavailable",
            Self::StagedExpired => "staged_expired",
            Self::StagedLost => "staged_lost",
            Self::QuotaExceeded => "quota_exceeded",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        REASONS.into_iter().find(|reason| reason.label() == label)
    }

    /// Output that arrived but is not usable, reported as `Invalid` (plan 6.3: not charged).
    fn is_invalid_output(self) -> bool {
        matches!(self, Self::OverMaximumBytes | Self::ValidationFailed)
    }
}

/// The one terminal report per attempt, written to the settlement outbox F consumes (D22, D27).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinalizeOutcome {
    Ready,
    Superseded,
    Rejected(RejectReason),
    Invalid(RejectReason),
}

impl FinalizeOutcome {
    pub fn of_rejection(reason: RejectReason) -> Self {
        if reason.is_invalid_output() {
            Self::Invalid(reason)
        } else {
            Self::Rejected(reason)
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Superseded => "superseded",
            Self::Rejected(_) => "rejected",
            Self::Invalid(_) => "invalid",
        }
    }

    pub fn reason(self) -> Option<RejectReason> {
        match self {
            Self::Rejected(reason) | Self::Invalid(reason) => Some(reason),
            Self::Ready | Self::Superseded => None,
        }
    }
}

/// What the orphan sweep knows about an asset row, read under its `FOR SHARE` lock.
#[derive(Clone, Copy, Debug)]
pub struct SweepRow {
    pub state: AssetState,
    pub current_attempt: AttemptId,
    pub current_owner: Option<ClaimToken>,
}

/// What the orphan sweep knows about one variant write.
#[derive(Clone, Copy, Debug)]
pub struct SweepWrite {
    pub attempt: AttemptId,
    pub owner: ClaimToken,
    pub is_winner: bool,
}

/// The 6.2 rule. A winning write of a row that reached `Ready` is never deletable here (only the GC
/// purge removes it); a write of the current owner on a row that never reached `Ready` is never
/// deletable, however old.
pub fn is_write_deletable(row: SweepRow, write: SweepWrite) -> bool {
    if row.state.has_reached_ready() && write.is_winner {
        return false;
    }
    if write.attempt != row.current_attempt || row.state == AssetState::Rejected {
        return true;
    }
    if row.state.has_reached_ready() {
        return true;
    }
    row.current_owner != Some(write.owner)
}

/// A recorded staged object is needed only until its attempt publishes or ends.
pub fn is_staged_deletable(row: SweepRow, staged_attempt: AttemptId) -> bool {
    staged_attempt != row.current_attempt
        || row.state == AssetState::Rejected
        || row.state.has_reached_ready()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(byte: u8) -> AttemptId {
        let mut bytes = [0_u8; 16];
        bytes[15] = byte;
        AttemptId::from_bytes(&bytes).unwrap()
    }

    fn claim(byte: u8) -> ClaimToken {
        let mut bytes = [0_u8; 16];
        bytes[15] = byte;
        ClaimToken::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn exactly_the_table_transitions_are_legal() {
        use AssetState as S;
        use Transition as T;
        let legal = [
            (S::Uploading, T::Stage, S::Quarantined),
            (S::Uploading, T::Reject, S::Rejected),
            (S::Quarantined, T::Reject, S::Rejected),
            (S::Validated, T::Reject, S::Rejected),
            (S::Quarantined, T::Validate, S::Validated),
            (S::Validated, T::Finalize, S::Ready),
            (S::Ready, T::Archive, S::Archived),
            (S::Archived, T::Restore, S::Ready),
            (S::Ready, T::Tombstone, S::Deleted),
            (S::Archived, T::Tombstone, S::Deleted),
            (S::Deleted, T::BeginPurge, S::Purging),
            (S::Purging, T::CompletePurge, S::Purged),
        ];
        let transitions = [
            T::Stage,
            T::Validate,
            T::Finalize,
            T::Reject,
            T::Archive,
            T::Restore,
            T::Tombstone,
            T::BeginPurge,
            T::CompletePurge,
        ];
        for from in STATES {
            for transition in transitions {
                let expected = legal
                    .iter()
                    .find(|(state, step, _)| *state == from && *step == transition)
                    .map(|(_, _, to)| *to);
                assert_eq!(
                    from.after(transition).ok(),
                    expected,
                    "{from:?} {transition:?}"
                );
            }
        }
    }

    #[test]
    fn every_state_and_reason_round_trips_through_its_label() {
        for state in STATES {
            assert_eq!(AssetState::from_label(state.label()), Some(state));
        }
        for reason in REASONS {
            assert_eq!(RejectReason::from_label(reason.label()), Some(reason));
        }
        assert_eq!(AssetState::from_label("READY"), None);
    }

    #[test]
    fn unusable_output_is_reported_invalid_and_everything_else_rejected() {
        assert_eq!(
            FinalizeOutcome::of_rejection(RejectReason::OverMaximumBytes),
            FinalizeOutcome::Invalid(RejectReason::OverMaximumBytes)
        );
        assert_eq!(
            FinalizeOutcome::of_rejection(RejectReason::OutputLost),
            FinalizeOutcome::Rejected(RejectReason::OutputLost)
        );
    }

    #[test]
    fn a_winner_of_a_row_that_reached_ready_is_never_swept_whatever_the_lease() {
        for state in [
            AssetState::Ready,
            AssetState::Archived,
            AssetState::Deleted,
            AssetState::Purging,
            AssetState::Purged,
        ] {
            let row = SweepRow {
                state,
                current_attempt: attempt(1),
                current_owner: Some(claim(9)),
            };
            let winner = SweepWrite {
                attempt: attempt(1),
                owner: claim(2),
                is_winner: true,
            };
            assert!(!is_write_deletable(row, winner), "{state:?}");
            assert!(is_write_deletable(
                row,
                SweepWrite {
                    is_winner: false,
                    ..winner
                }
            ));
        }
    }

    #[test]
    fn a_winner_of_a_row_that_reached_ready_survives_even_under_a_different_attempt() {
        let row = SweepRow {
            state: AssetState::Ready,
            current_attempt: attempt(5),
            current_owner: None,
        };
        let winner = SweepWrite {
            attempt: attempt(1),
            owner: claim(2),
            is_winner: true,
        };
        assert!(!is_write_deletable(row, winner));
        assert!(is_write_deletable(
            row,
            SweepWrite {
                is_winner: false,
                ..winner
            }
        ));
    }

    #[test]
    fn the_current_owners_writes_survive_and_older_claims_are_swept_before_ready() {
        let row = SweepRow {
            state: AssetState::Validated,
            current_attempt: attempt(1),
            current_owner: Some(claim(2)),
        };
        let mine = SweepWrite {
            attempt: attempt(1),
            owner: claim(2),
            is_winner: false,
        };
        assert!(!is_write_deletable(row, mine));
        assert!(is_write_deletable(
            row,
            SweepWrite {
                owner: claim(3),
                ..mine
            }
        ));
        assert!(is_write_deletable(
            row,
            SweepWrite {
                attempt: attempt(4),
                ..mine
            }
        ));
        let rejected = SweepRow {
            state: AssetState::Rejected,
            ..row
        };
        assert!(is_write_deletable(rejected, mine));
    }

    #[test]
    fn a_staged_object_is_kept_until_its_attempt_publishes_or_ends() {
        let row = SweepRow {
            state: AssetState::Quarantined,
            current_attempt: attempt(1),
            current_owner: None,
        };
        assert!(!is_staged_deletable(row, attempt(1)));
        assert!(is_staged_deletable(row, attempt(2)));
        assert!(is_staged_deletable(
            SweepRow {
                state: AssetState::Ready,
                ..row
            },
            attempt(1)
        ));
    }
}
