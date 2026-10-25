use std::collections::BTreeMap;
use std::sync::Arc;

use df_types::{OperationId, RunId, SessionId, SessionRevision};

/// Caller-owned scene and cue identities anchored to a canonical committed revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneBinding<SceneId, CueId> {
    pub session: SessionId,
    pub run: RunId,
    pub scene: SceneId,
    pub cue: CueId,
    pub revision: SessionRevision,
}

/// Refusals leave admission and current-scene state unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleError {
    ZeroCapacity,
    Capacity,
    AlreadyActive,
    ObsoleteScene,
    RevisionNotAdvanced,
    GenerationExhausted,
}

/// Reasons an owned completion was prevented from updating the current scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionRefusal {
    ForeignOwner,
    NotActive,
    ReplacedJob,
    ObsoleteScene,
}

/// Rejected provider output is returned to its caller; no publication callback ran.
#[derive(Debug)]
pub struct CompletionRejection<Output> {
    pub reason: CompletionRefusal,
    pub output: Output,
}

/// Move-only permission to offer one completion to the owner that admitted it.
/// This permission does not establish provider success or verified asset publication.
#[derive(Debug)]
pub struct CompletionTicket<JobId, SceneId, CueId> {
    owner: Arc<()>,
    job: JobId,
    operation: OperationId,
    generation: u64,
    binding: SceneBinding<SceneId, CueId>,
}

#[derive(Debug)]
struct ActiveJob {
    operation: OperationId,
    generation: u64,
}

/// Bounded admission and publication fence owned by the server scene owner.
/// Provider work is caller-owned. Explicit cancellation invalidates its result,
/// but does not claim cancellation of paid work or reversal of committed actions.
/// One Arc identity prevents cross-owner ABA even when an owner is recreated.
#[derive(Debug)]
pub struct MediaLifecycle<JobId, SceneId, CueId> {
    owner: Arc<()>,
    current: SceneBinding<SceneId, CueId>,
    active: BTreeMap<JobId, ActiveJob>,
    capacity: usize,
    next_generation: u64,
}

impl<JobId: Ord + Clone, SceneId: Clone + Eq, CueId: Clone + Eq>
    MediaLifecycle<JobId, SceneId, CueId>
{
    pub fn new(
        current: SceneBinding<SceneId, CueId>,
        capacity: usize,
    ) -> Result<Self, LifecycleError> {
        if capacity == 0 {
            return Err(LifecycleError::ZeroCapacity);
        }
        Ok(Self {
            owner: Arc::new(()),
            current,
            active: BTreeMap::new(),
            capacity,
            next_generation: 0,
        })
    }

    pub fn current(&self) -> &SceneBinding<SceneId, CueId> {
        &self.current
    }

    pub fn active_jobs(&self) -> usize {
        self.active.len()
    }

    /// Accepts only the current binding and preserves the originating operation.
    pub fn admit(
        &mut self,
        job: JobId,
        operation: OperationId,
        binding: SceneBinding<SceneId, CueId>,
    ) -> Result<CompletionTicket<JobId, SceneId, CueId>, LifecycleError> {
        if binding != self.current {
            return Err(LifecycleError::ObsoleteScene);
        }
        if self.active.contains_key(&job) {
            return Err(LifecycleError::AlreadyActive);
        }
        if self.active.len() >= self.capacity {
            return Err(LifecycleError::Capacity);
        }
        let generation = self
            .next_generation
            .checked_add(1)
            .ok_or(LifecycleError::GenerationExhausted)?;
        self.active.insert(
            job.clone(),
            ActiveJob {
                operation,
                generation,
            },
        );
        self.next_generation = generation;
        Ok(CompletionTicket {
            owner: Arc::clone(&self.owner),
            job,
            operation,
            generation,
            binding,
        })
    }

    /// Explicit owner cancellation. False means there is no active job to cancel.
    /// A late completion cannot remove a newly admitted job with the same identity.
    pub fn cancel(&mut self, ticket: &CompletionTicket<JobId, SceneId, CueId>) -> bool {
        if !Arc::ptr_eq(&self.owner, &ticket.owner) {
            return false;
        }
        let is_active_job = self.active.get(&ticket.job).is_some_and(|active| {
            active.generation == ticket.generation && active.operation == ticket.operation
        });
        if is_active_job {
            self.active.remove(&ticket.job);
        }
        is_active_job
    }

    /// Moves to a strictly newer canonical revision and invalidates all old work.
    /// Session/run and identity authority remain the caller's responsibility.
    pub fn replace_scene(
        &mut self,
        current: SceneBinding<SceneId, CueId>,
    ) -> Result<usize, LifecycleError> {
        if current.revision <= self.current.revision {
            return Err(LifecycleError::RevisionNotAdvanced);
        }
        let invalidated = self.active.len();
        self.active.clear();
        self.current = current;
        Ok(invalidated)
    }

    /// Consumes one completion. The owner callback runs synchronously only after
    /// all fences pass; cancelled, obsolete and replaced output returns untouched.
    /// The callback consumes an already validated result; it cannot claim durable
    /// bytes or metadata publication merely from passing this eligibility check.
    pub fn complete<Output>(
        &mut self,
        ticket: CompletionTicket<JobId, SceneId, CueId>,
        output: Output,
        publish: impl FnOnce(OperationId, Output),
    ) -> Result<(), CompletionRejection<Output>> {
        let refusal = if !Arc::ptr_eq(&self.owner, &ticket.owner) {
            Some(CompletionRefusal::ForeignOwner)
        } else if ticket.binding != self.current {
            Some(CompletionRefusal::ObsoleteScene)
        } else {
            match self.active.get(&ticket.job) {
                None => Some(CompletionRefusal::NotActive),
                Some(active)
                    if active.generation != ticket.generation
                        || active.operation != ticket.operation =>
                {
                    Some(CompletionRefusal::ReplacedJob)
                }
                Some(_) => None,
            }
        };
        if let Some(reason) = refusal {
            return Err(CompletionRejection { reason, output });
        }
        self.active.remove(&ticket.job);
        publish(ticket.operation, output);
        Ok(())
    }
}
