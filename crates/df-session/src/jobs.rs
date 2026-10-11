//! Native ownership before execution. Registration is not commit, auth, spend or readiness.
//! The session owner must supply acknowledged durable intents and current run ownership.
//! F23-I03/W01 own the actor/supervisor connection. Drop aborts children; explicit retirement
//! followed by draining terminal results is required to observe their joined outcomes.
use std::future::Future;
use std::mem::{size_of, size_of_val};
use std::pin::Pin;

use df_model::checkpoint::{Basis, DurableIntent, DurableStatus, EffectId, EffectKind};
use df_observe::{OperationContext, begin_dispatched};
use df_types::{RunId, SessionId};
use tokio::task::{AbortHandle, Id, JoinSet};
use tokio::time::{Instant, timeout_at};

use crate::effects::EffectCompletion;
use crate::inbox::ActorInput;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobOwner {
    pub session: SessionId,
    pub run: RunId,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobLimits {
    pub maximum_items: usize,
    /// Owned registration/context/future capacities, excluding opaque Tokio allocator overhead.
    pub maximum_retained_bytes: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobIdentity {
    pub effect: EffectId,
    pub basis: Basis,
    pub completion: EffectCompletion,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobFailure {
    Unavailable,
    InvalidResult,
    Capacity,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobRefusal {
    Closed,
    InvalidOwner,
    InvalidIntent,
    MissingExecutor,
    Duplicate,
    ItemCapacity,
    ByteCapacity,
    AllocationUnavailable,
    Deadline,
    RuntimeUnavailable,
    PreparationPanicked,
    LifecycleMismatch,
    Preparation(JobFailure),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobOutcome {
    Completed,
    Failed(JobFailure),
    Deadline,
    Cancelled,
    Panicked,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobTerminal {
    pub identity: JobIdentity,
    pub outcome: JobOutcome,
}

/// Future implementations count all retained heap capacities and perform only bounded,
/// nonblocking work. No task may spawn a detached child or retain an RPC cancellation scope.
pub trait OwnedJob: Future<Output = Result<(), JobFailure>> + ActorInput {}
impl<T: Future<Output = Result<(), JobFailure>> + ActorInput> OwnedJob for T {}
pub type PreparedJob = Pin<Box<dyn OwnedJob>>;

/// Private construction proves local registration occurred before this dispatch context
/// existed. The identity is provenance only, never a durable database acknowledgement.
pub struct JobDispatch {
    identity: JobIdentity,
    context: OperationContext,
}
impl JobDispatch {
    pub fn identity(&self) -> JobIdentity {
        self.identity
    }
    pub fn context(&self) -> &OperationContext {
        &self.context
    }
}
/// A compiled factory is preparation only: it must neither poll work nor perform I/O.
/// Its returned future performs execution after registration and task ownership exist.
/// This binding supplies no production readiness or external egress authority.
#[derive(Clone, Copy)]
pub struct JobExecutor {
    pub kind: EffectKind,
    pub prepare: fn(JobDispatch) -> Result<PreparedJob, JobFailure>,
}

struct Registration {
    identity: JobIdentity,
    context: OperationContext,
    retained_bytes: usize,
    handle: Option<AbortHandle>,
    deadline: Instant,
    cancellation: Option<JobOutcome>,
}
pub struct JobRegistry {
    owner: JobOwner,
    limits: JobLimits,
    accepting: bool,
    registrations: Vec<Registration>,
    retained_bytes: usize,
    tasks: JoinSet<JobOutcome>,
}
impl JobRegistry {
    pub fn new(owner: JobOwner, limits: JobLimits) -> Result<Self, JobRefusal> {
        if owner.generation == 0 {
            return Err(JobRefusal::InvalidOwner);
        }
        if limits.maximum_items == 0 || limits.maximum_retained_bytes == 0 {
            return Err(JobRefusal::ItemCapacity);
        }
        Ok(Self {
            owner,
            limits,
            accepting: true,
            registrations: Vec::new(),
            retained_bytes: 0,
            tasks: JoinSet::new(),
        })
    }
    pub fn registered_items(&self) -> usize {
        self.registrations.len()
    }
    pub fn retained_bytes(&self) -> Option<usize> {
        self.registrations
            .capacity()
            .checked_mul(size_of::<Registration>())
            .and_then(|bytes| bytes.checked_add(self.retained_bytes))
    }
    pub fn contains(&self, identity: JobIdentity) -> bool {
        self.registrations
            .iter()
            .any(|entry| entry.identity == identity)
    }

    /// The caller resolves committed state and its owner fence before this local step.
    /// A Pending intent is data, not proof of commit. Factories are actual compiled
    /// bindings; missing kinds refuse rather than becoming fabricated readiness.
    pub fn spawn(
        &mut self,
        intent: &DurableIntent,
        context: &OperationContext,
        deadline: Instant,
        executors: &[JobExecutor],
    ) -> Result<JobIdentity, JobRefusal> {
        if !self.accepting {
            return Err(JobRefusal::Closed);
        }
        if intent.basis.session != self.owner.session
            || intent.basis.run != self.owner.run
            || intent.generation != self.owner.generation
            || intent.generation == 0
        {
            return Err(JobRefusal::InvalidOwner);
        }
        if intent.status != DurableStatus::Pending
            || intent.timer.is_some()
            || !matches!(
                intent.kind,
                EffectKind::RunAi | EffectKind::RunMedia | EffectKind::LoadMemoryCandidates
            )
        {
            return Err(JobRefusal::InvalidIntent);
        }
        let job = intent.job.ok_or(JobRefusal::InvalidIntent)?;
        let identity = JobIdentity {
            effect: intent.id,
            basis: intent.basis,
            completion: EffectCompletion {
                run: intent.basis.run,
                operation: intent.operation,
                job,
                generation: intent.generation,
            },
        };
        if self.registrations.iter().any(|entry| {
            entry.identity.effect == identity.effect || entry.identity.completion.job == job
        }) {
            return Err(JobRefusal::Duplicate);
        }
        // There are exactly three canonical job-producing kinds; duplicate or
        // oversized binding lists provide no executor authority and stay bounded.
        if executors.len() > 3 {
            return Err(JobRefusal::MissingExecutor);
        }
        let mut matches = executors
            .iter()
            .filter(|executor| executor.kind == intent.kind);
        let executor = matches.next().ok_or(JobRefusal::MissingExecutor)?;
        if matches.next().is_some() {
            return Err(JobRefusal::MissingExecutor);
        }
        if deadline <= Instant::now() {
            return Err(JobRefusal::Deadline);
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(JobRefusal::RuntimeUnavailable);
        }
        if self.registrations.len() >= self.limits.maximum_items {
            return Err(JobRefusal::ItemCapacity);
        }
        // Preflight the retained vector allocation and two owned context snapshots.
        let required = self
            .registrations
            .len()
            .checked_add(1)
            .map(|items| items.max(self.registrations.capacity()))
            .and_then(|items| items.checked_mul(size_of::<Registration>()))
            .and_then(|bytes| bytes.checked_add(self.retained_bytes))
            .and_then(|bytes| bytes.checked_add(context.build.len().checked_mul(2)?))
            .and_then(|bytes| bytes.checked_add(context.trace_parent.len().checked_mul(2)?))
            .ok_or(JobRefusal::ByteCapacity)?;
        if required > self.limits.maximum_retained_bytes {
            return Err(JobRefusal::ByteCapacity);
        }
        self.registrations
            .try_reserve_exact(1)
            .map_err(|_| JobRefusal::AllocationUnavailable)?;
        let captured = capture_context(context)?;
        let dispatch_context = capture_context(context)?;
        let base_bytes = captured
            .build
            .capacity()
            .checked_add(captured.trace_parent.capacity())
            .and_then(|bytes| bytes.checked_add(dispatch_context.build.capacity()))
            .and_then(|bytes| bytes.checked_add(dispatch_context.trace_parent.capacity()))
            .ok_or(JobRefusal::ByteCapacity)?;
        self.registrations.push(Registration {
            identity,
            context: captured,
            retained_bytes: base_bytes,
            handle: None,
            deadline,
            cancellation: None,
        });
        // No factory or future can be polled before the exact registration is owned.
        let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (executor.prepare)(JobDispatch {
                identity,
                context: dispatch_context,
            })
        }));
        let work = match prepared {
            Ok(Ok(work)) => work,
            Ok(Err(error)) => {
                self.registrations.pop();
                return Err(JobRefusal::Preparation(error));
            }
            Err(_) => {
                self.registrations.pop();
                return Err(JobRefusal::PreparationPanicked);
            }
        };
        let work_bytes = work
            .as_ref()
            .get_ref()
            .retained_heap_bytes()
            .and_then(|heap| size_of_val(work.as_ref().get_ref()).checked_add(heap));
        let bytes = work_bytes.and_then(|bytes| bytes.checked_add(base_bytes));
        let total = bytes
            .and_then(|bytes| self.retained_bytes.checked_add(bytes))
            .and_then(|bytes| {
                bytes.checked_add(
                    self.registrations
                        .capacity()
                        .checked_mul(size_of::<Registration>())?,
                )
            });
        if total.is_none_or(|bytes| bytes > self.limits.maximum_retained_bytes) {
            self.registrations.pop();
            return Err(JobRefusal::ByteCapacity);
        }
        let Some(bytes) = bytes else {
            self.registrations.pop();
            return Err(JobRefusal::ByteCapacity);
        };
        // The retained context supplies terminal observation; the future owns dispatch context.
        let handle = self.tasks.spawn(async move {
            if Instant::now() >= deadline {
                return JobOutcome::Deadline;
            }
            match timeout_at(deadline, work).await {
                Ok(Ok(())) => JobOutcome::Completed,
                Ok(Err(error)) => JobOutcome::Failed(error),
                Err(_) => JobOutcome::Deadline,
            }
        });
        let Some(entry) = self.registrations.last_mut() else {
            handle.abort();
            return Err(JobRefusal::InvalidIntent);
        };
        entry.retained_bytes = bytes;
        entry.handle = Some(handle);
        self.retained_bytes += bytes;
        Ok(identity)
    }

    /// Exact matching identity only; cancellation retains the registration until join.
    pub fn cancel(&mut self, identity: JobIdentity) -> bool {
        if let Some(entry) = self
            .registrations
            .iter_mut()
            .find(|entry| entry.identity == identity)
            && let Some(handle) = &entry.handle
        {
            entry.cancellation = Some(JobOutcome::Cancelled);
            handle.abort();
            return true;
        }
        false
    }
    /// A native owner wake may supply controlled monotonic scheduling time. This never
    /// advances canonical logical time. Due registrations remain until actual join.
    pub fn expire_due(&mut self, now: Instant) -> usize {
        let mut expired = 0;
        for entry in &mut self.registrations {
            if now >= entry.deadline
                && entry.cancellation.is_none()
                && let Some(handle) = &entry.handle
            {
                entry.cancellation = Some(JobOutcome::Deadline);
                handle.abort();
                expired += 1;
            }
        }
        expired
    }
    /// Close admission and request cancellation of every owned task. Drain next_terminal
    /// until None to observe outcomes; cancellation cannot undo already completed work.
    pub fn retire(&mut self) {
        self.accepting = false;
        for entry in &mut self.registrations {
            if entry.cancellation.is_none() {
                entry.cancellation = Some(JobOutcome::Cancelled);
            }
        }
        self.tasks.abort_all();
    }
    pub async fn next_terminal(&mut self) -> Result<Option<JobTerminal>, JobRefusal> {
        let Some(result) = self.tasks.join_next_with_id().await else {
            return if self.registrations.is_empty() {
                Ok(None)
            } else {
                Err(JobRefusal::LifecycleMismatch)
            };
        };
        let (id, outcome) = match result {
            Ok((id, outcome)) => (id, outcome),
            Err(error) => {
                let outcome = if error.is_cancelled() {
                    JobOutcome::Cancelled
                } else {
                    JobOutcome::Panicked
                };
                (error.id(), outcome)
            }
        };
        self.take_terminal(id, outcome)
    }
    fn take_terminal(
        &mut self,
        id: Id,
        outcome: JobOutcome,
    ) -> Result<Option<JobTerminal>, JobRefusal> {
        let index = self
            .registrations
            .iter()
            .position(|entry| {
                entry
                    .handle
                    .as_ref()
                    .is_some_and(|handle| handle.id() == id)
            })
            .ok_or(JobRefusal::LifecycleMismatch)?;
        let entry = self.registrations.remove(index);
        let outcome = if outcome == JobOutcome::Cancelled {
            entry.cancellation.unwrap_or(outcome)
        } else {
            outcome
        };
        self.retained_bytes -= entry.retained_bytes;
        if self.registrations.is_empty() {
            self.registrations = Vec::new();
        }
        let mut span = begin_dispatched(&entry.context, "session.job.terminal");
        span.finish_unmeasured(match outcome {
            JobOutcome::Completed => "completed",
            JobOutcome::Failed(_) => "failed",
            JobOutcome::Deadline => "deadline",
            JobOutcome::Cancelled => "cancelled",
            JobOutcome::Panicked => "panicked",
        });
        Ok(Some(JobTerminal {
            identity: entry.identity,
            outcome,
        }))
    }
}
fn capture_context(context: &OperationContext) -> Result<OperationContext, JobRefusal> {
    fn copy(value: &str) -> Result<String, JobRefusal> {
        let mut captured = String::new();
        captured
            .try_reserve_exact(value.len())
            .map_err(|_| JobRefusal::AllocationUnavailable)?;
        captured.push_str(value);
        Ok(captured)
    }
    Ok(OperationContext {
        trace_parent: copy(&context.trace_parent)?,
        build: copy(&context.build)?,
    })
}
