//! Pure staged transition classification. Durable acknowledgement belongs to the session.
use crate::checkpoint::{Checkpoint, PendingInput, ResolutionId, WindowId};
use df_types::OperationId;

/// A pending continuation is an accepted suspension, not completed mechanics or a receipt.
/// Rejection retains no candidate. The source-qualified engine supplies the candidate; this
/// representation verifies its operation and continuation binding, not rules legality.
#[derive(Clone, Eq, PartialEq)]
pub enum TransitionResult<R> {
    Accepted(Checkpoint),
    Pending {
        candidate: Checkpoint,
        resolution: ResolutionId,
        window: WindowId,
        next: PendingInput,
    },
    Rejected(R),
}

impl<R: std::fmt::Debug> std::fmt::Debug for TransitionResult<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Accepted(candidate) => {
                formatter.debug_tuple("Accepted").field(candidate).finish()
            }
            Self::Pending {
                candidate,
                resolution,
                window,
                ..
            } => formatter
                .debug_struct("Pending")
                .field("candidate", candidate)
                .field("resolution", resolution)
                .field("window", window)
                .finish_non_exhaustive(),
            Self::Rejected(error) => formatter.debug_tuple("Rejected").field(error).finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionError {
    Basis,
    Decision,
    AmbiguousPending,
    PendingEffects,
    Disposition,
}

fn pending_identity(
    source_checkpoint: &Checkpoint,
    staged_checkpoint: &Checkpoint,
    operation_id: OperationId,
) -> Result<Option<(ResolutionId, WindowId, PendingInput)>, TransitionError> {
    let previous = source_checkpoint.basis();
    let next = staged_checkpoint.basis();
    if previous.session != next.session
        || previous.run != next.run
        || previous.revision.next_sequence().ok() != Some(next.revision)
        || source_checkpoint.pins() != staged_checkpoint.pins()
    {
        return Err(TransitionError::Basis);
    }
    let earlier = &source_checkpoint.state().decisions;
    let decisions = &staged_checkpoint.state().decisions;
    let Some([decision]) = decisions.get(earlier.len()..) else {
        return Err(TransitionError::Decision);
    };
    if !decisions.starts_with(earlier)
        || decision.operation != operation_id
        || decision.revision != next.revision
    {
        return Err(TransitionError::Decision);
    }
    let mut matching = staged_checkpoint.state().pending.iter().filter(|pending| {
        if pending.basis != next {
            return false;
        }
        let new_resolution = !source_checkpoint
            .state()
            .pending
            .iter()
            .any(|previous| previous.id == pending.id)
            && source_checkpoint
                .state()
                .facts
                .iter()
                .any(|fact| fact.id == pending.window.causal_fact);
        if decision.facts.contains(&pending.window.causal_fact) || new_resolution {
            return true;
        }
        source_checkpoint.state().pending.iter().any(|previous| {
            previous.id == pending.id
                && (previous.continuation != pending.continuation
                    || previous.window != pending.window
                    || previous.next != pending.next
                    || previous.choices != pending.choices
                    || previous.draw_ordinals != pending.draw_ordinals
                    || previous.spent != pending.spent
                    || previous.rulings != pending.rulings)
        })
    });
    let identity = matching
        .next()
        .map(|pending| (pending.id, pending.window.id, pending.next.clone()));
    if matching.next().is_some() {
        return Err(TransitionError::AmbiguousPending);
    }
    if identity.is_some() && !decision.effects.is_empty() {
        return Err(TransitionError::PendingEffects);
    }
    Ok(identity)
}

impl<R> TransitionResult<R> {
    /// Classify one already source-validated staged checkpoint. A newly created resolution may
    /// refer to either a new or a retained causal fact; an advanced old resolution must change
    /// its legal continuation. Basis-only carriage of historical rows is not a new suspension.
    pub fn classify(
        current: &Checkpoint,
        candidate: Checkpoint,
        operation: OperationId,
    ) -> Result<Self, TransitionError> {
        match pending_identity(current, &candidate, operation)? {
            Some((resolution, window, next)) => Ok(Self::Pending {
                candidate,
                resolution,
                window,
                next,
            }),
            None => Ok(Self::Accepted(candidate)),
        }
    }

    /// The serialized owner rechecks even an engine-supplied variant before attempting commit.
    pub fn validate(
        &self,
        current: &Checkpoint,
        operation: OperationId,
    ) -> Result<(), TransitionError> {
        match self {
            Self::Accepted(candidate) => {
                if pending_identity(current, candidate, operation)?.is_some() {
                    Err(TransitionError::Disposition)
                } else {
                    Ok(())
                }
            }
            Self::Pending {
                candidate,
                resolution,
                window,
                next,
            } => {
                if pending_identity(current, candidate, operation)?
                    != Some((*resolution, *window, next.clone()))
                {
                    Err(TransitionError::Disposition)
                } else {
                    Ok(())
                }
            }
            Self::Rejected(_) => Ok(()),
        }
    }
}
