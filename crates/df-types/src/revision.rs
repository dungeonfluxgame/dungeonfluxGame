use std::num::NonZeroU64;

/// Invalid epoch or exhausted in-epoch sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionError {
    ZeroEpoch,
    SequenceOverflow,
}

/// Supplied nonzero recovery epoch; construction does not issue or verify an epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecoveryEpoch(NonZeroU64);

impl RecoveryEpoch {
    /// Accepts any nonzero supplied epoch; zero is `ZeroEpoch`.
    pub fn new(value: u64) -> Result<Self, RevisionError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(RevisionError::ZeroEpoch)
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// Composite recovery revision ordered lexicographically by epoch, then sequence.
/// Sequence zero is valid initial state. This value grants no recovery authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionRevision {
    epoch: RecoveryEpoch,
    sequence: u64,
}

impl SessionRevision {
    /// Pairs a supplied epoch with an in-epoch sequence; zero is the initial sequence.
    pub fn new(epoch: RecoveryEpoch, sequence: u64) -> Self {
        Self { epoch, sequence }
    }

    pub fn epoch(self) -> RecoveryEpoch {
        self.epoch
    }

    pub fn sequence(self) -> u64 {
        self.sequence
    }

    /// Advances only within this epoch, refusing overflow rather than issuing an epoch.
    pub fn next_sequence(self) -> Result<Self, RevisionError> {
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(RevisionError::SequenceOverflow)?;
        Ok(Self::new(self.epoch, sequence))
    }
}
