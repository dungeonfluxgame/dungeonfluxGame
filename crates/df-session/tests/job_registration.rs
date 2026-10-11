#![cfg(not(target_arch = "wasm32"))]
//! B-F23-D02 Source contract: actual native task ownership, not production readiness.
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use df_model::checkpoint::{
    Basis, ContentReference, DurableIntent, DurableStatus, EffectId, EffectKind, JobId,
};
use df_observe::OperationContext;
use df_session::inbox::ActorInput;
use df_session::jobs::*;
use df_types::{OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision};
use tokio::runtime::Builder;
use tokio::time::Instant;

const CONTRACT_LITERAL: &str = r##"{
  "alternatives": [
    {
      "decision": "rejected",
      "option": "Spawn first and register its handle afterwards",
      "reason": "Immediate completion or panic can escape ownership."
    },
    {
      "decision": "rejected",
      "option": "Use RPC cancellation for already accepted effects",
      "reason": "Durable session/run work survives cancelled or lost receipt waits."
    },
    {
      "decision": "rejected",
      "option": "Release registration when cancellation is requested",
      "reason": "A queued or completed child must still be joined, and cancellation cannot undo completed work."
    },
    {
      "decision": "rejected",
      "option": "Advertise all effect kinds as ready using no-op executors",
      "reason": "An actual compiled binding and native ownership are required; missing executors remain explicit."
    }
  ],
  "attempt_id": "B-F23-D02-a1",
  "decision": {
    "authority": "Supplied intent/status/context are data only. Actual durable acknowledgement, current owner fence, auth, spend/journal and paid-egress readiness remain the native caller's responsibilities. Missing or ambiguous compiled factories refuse.",
    "bounds": "Caller-selected nonzero item and retained-capacity bounds; no unbounded executor scan. Retained capacities cover registration/context/future payload, excluding opaque Tokio allocator overhead. Owned future implementations preserve ActorInput capacity accounting and bounded nonblocking behavior.",
    "clock": "Explicit monotonic owner wakes and independently scheduled deadlines never advance canonical game logical time.",
    "lifecycle": "Only actual joined completion, failure, panic, cancellation or deadline releases registration. Cancelled RPC receipt waits do not appear in the job lifetime. Full identity mismatches are inert.",
    "owner": "The native JobRegistry owns the exact effect/basis/run/operation/job/generation registration before the compiled preparation function is invoked and before Tokio JoinSet may poll work."
  },
  "objective": "Specify job registration lifecycle; expected: effects owned before spawn.",
  "original_acceptance": [
    "effects owned before spawn",
    "The named outcome has actual source/build-bound evidence; unsupported, pending, failed and unperformed checks remain explicit."
  ],
  "task_id": "B-F23-D02",
  "unresolved": [
    "F23-I03/W01 own production DurableOwner/actor/supervisor wiring and whole-game observation.",
    "Current df-server source has no Cargo package and its MissingEffectExecutor catalogue is unchanged.",
    "No PostgreSQL commit, paid egress, restart/takeover, physical-device, browser/audio or full-game readiness qualification is delivered by this Source contract.",
    "Compiler/native/Git/integration qualifications remain UNPERFORMED until Root-issued release and independent review."
  ]
}"##;

static POLLS: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];
struct Probe(JobDispatch);
impl ActorInput for Probe {
    fn retained_heap_bytes(&self) -> Option<usize> {
        if self.0.context().build == "overflow" {
            return None;
        }
        self.0
            .context()
            .build
            .capacity()
            .checked_add(self.0.context().trace_parent.capacity())
    }
}
impl Future for Probe {
    type Output = Result<(), JobFailure>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let identity = self.0.identity();
        POLLS[usize::from(identity.completion.operation.as_bytes()[0])]
            .fetch_add(1, Ordering::SeqCst);
        assert_eq!(identity.basis.run, identity.completion.run);
        assert_eq!(identity.completion.generation, 7);
        assert_eq!(
            self.0.context().trace_parent,
            "00-11111111111111111111111111111111-2222222222222222-01"
        );
        match self.0.context().build.as_str() {
            "pending" => Poll::Pending,
            "panic" => panic!("controlled registered child panic"),
            "failed" => Poll::Ready(Err(JobFailure::Unavailable)),
            "complete" => Poll::Ready(Ok(())),
            other => panic!("unexpected retained context {other}"),
        }
    }
}
fn prepare(dispatch: JobDispatch) -> Result<PreparedJob, JobFailure> {
    assert_eq!(dispatch.identity().completion.generation, 7);
    Ok(Box::pin(Probe(dispatch)))
}
fn refuse_preparation(_: JobDispatch) -> Result<PreparedJob, JobFailure> {
    Err(JobFailure::Capacity)
}
fn panic_preparation(_: JobDispatch) -> Result<PreparedJob, JobFailure> {
    panic!("controlled preparation panic")
}
fn context(build: &str) -> OperationContext {
    OperationContext {
        trace_parent: "00-11111111111111111111111111111111-2222222222222222-01".to_owned(),
        build: build.to_owned(),
    }
}
fn intent(index: u8) -> DurableIntent {
    DurableIntent {
        id: EffectId::from_bytes(&[index; 16]).unwrap(),
        basis: Basis {
            session: SessionId::from_bytes(&[1; 16]).unwrap(),
            run: RunId::from_bytes(&[2; 16]).unwrap(),
            revision: SessionRevision::new(RecoveryEpoch::new(3).unwrap(), 8),
        },
        operation: OperationId::from_bytes(&[index; 16]).unwrap(),
        slot: 0,
        kind: EffectKind::RunAi,
        job: Some(JobId::from_bytes(&[index; 16]).unwrap()),
        timer: None,
        generation: 7,
        status: DurableStatus::Pending,
        definition: ContentReference {
            package: RevisionLabel::new(Some("fixture-package")).unwrap(),
            entry: RevisionLabel::new(Some("fixture-executor")).unwrap(),
        },
    }
}
fn registry(items: usize, bytes: usize) -> JobRegistry {
    JobRegistry::new(
        JobOwner {
            session: SessionId::from_bytes(&[1; 16]).unwrap(),
            run: RunId::from_bytes(&[2; 16]).unwrap(),
            generation: 7,
        },
        JobLimits {
            maximum_items: items,
            maximum_retained_bytes: bytes,
        },
    )
    .unwrap()
}
fn bindings() -> [JobExecutor; 1] {
    [JobExecutor {
        kind: EffectKind::RunAi,
        prepare,
    }]
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
fn run(body: impl Future<Output = ()>) {
    Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(body);
}

#[test]
fn registration_exists_before_first_poll_and_context_outlives_receipt_wait() {
    run(async {
        assert!(CONTRACT_LITERAL.contains("effects owned before spawn"));
        assert!(CONTRACT_LITERAL.contains("F23-I03/W01"));
        let mut jobs = registry(2, 8192);
        let mut admitted = context("complete");
        let identity = jobs
            .spawn(&intent(1), &admitted, deadline(), &bindings())
            .unwrap();
        assert!(jobs.contains(identity));
        assert_eq!(jobs.registered_items(), 1);
        assert!(jobs.retained_bytes().is_some_and(|bytes| bytes <= 8192));
        assert_eq!(POLLS[1].load(Ordering::SeqCst), 0);
        admitted.build.clear();
        admitted.trace_parent.clear();
        drop(admitted);
        let terminal = jobs.next_terminal().await.unwrap().unwrap();
        assert_eq!(terminal.identity, identity);
        assert_eq!(terminal.outcome, JobOutcome::Completed);
        assert_eq!(POLLS[1].load(Ordering::SeqCst), 1);
        assert_eq!(jobs.registered_items(), 0);
        assert_eq!(jobs.retained_bytes(), Some(0));
        assert!(jobs.next_terminal().await.unwrap().is_none());
    });
}
#[test]
fn missing_or_ambiguous_executor_and_invalid_intents_never_spawn() {
    run(async {
        let mut jobs = registry(2, 8192);
        let original = intent(2);
        assert_eq!(
            jobs.spawn(&original, &context("complete"), deadline(), &[]),
            Err(JobRefusal::MissingExecutor)
        );
        let duplicate_binding = [bindings()[0], bindings()[0]];
        assert_eq!(
            jobs.spawn(
                &original,
                &context("complete"),
                deadline(),
                &duplicate_binding
            ),
            Err(JobRefusal::MissingExecutor)
        );
        let oversized = [bindings()[0]; 4];
        assert_eq!(
            jobs.spawn(&original, &context("complete"), deadline(), &oversized),
            Err(JobRefusal::MissingExecutor)
        );
        for variant in 0..6 {
            let mut changed = original.clone();
            match variant {
                0 => changed.generation = 0,
                1 => changed.basis.run = RunId::from_bytes(&[9; 16]).unwrap(),
                2 => changed.basis.session = SessionId::from_bytes(&[9; 16]).unwrap(),
                3 => changed.status = DurableStatus::Completed,
                4 => changed.job = None,
                _ => changed.kind = EffectKind::ArmTimer,
            }
            assert!(
                jobs.spawn(&changed, &context("complete"), deadline(), &bindings())
                    .is_err()
            );
            assert_eq!(jobs.registered_items(), 0);
        }
        assert_eq!(POLLS[2].load(Ordering::SeqCst), 0);
    });
}
#[test]
fn capacity_overflow_and_expired_deadline_refuse_without_a_task() {
    run(async {
        let mut small = registry(2, 1);
        assert_eq!(
            small.spawn(&intent(3), &context("complete"), deadline(), &bindings()),
            Err(JobRefusal::ByteCapacity)
        );
        let mut jobs = registry(2, 8192);
        assert_eq!(
            jobs.spawn(
                &intent(3),
                &context("complete"),
                Instant::now(),
                &bindings()
            ),
            Err(JobRefusal::Deadline)
        );
        assert_eq!(
            jobs.spawn(&intent(3), &context("overflow"), deadline(), &bindings()),
            Err(JobRefusal::ByteCapacity)
        );
        assert_eq!(jobs.registered_items(), 0);
        assert_eq!(POLLS[3].load(Ordering::SeqCst), 0);
        let good = jobs
            .spawn(&intent(3), &context("complete"), deadline(), &bindings())
            .unwrap();
        assert_eq!(jobs.next_terminal().await.unwrap().unwrap().identity, good);
    });
}
#[test]
fn duplicate_and_item_capacity_cannot_release_an_existing_job() {
    run(async {
        let mut jobs = registry(1, 8192);
        let first = jobs
            .spawn(&intent(4), &context("pending"), deadline(), &bindings())
            .unwrap();
        assert_eq!(
            jobs.spawn(&intent(4), &context("complete"), deadline(), &bindings()),
            Err(JobRefusal::Duplicate)
        );
        assert_eq!(
            jobs.spawn(&intent(5), &context("complete"), deadline(), &bindings()),
            Err(JobRefusal::ItemCapacity)
        );
        assert!(jobs.contains(first));
        jobs.retire();
        assert_eq!(
            jobs.next_terminal().await.unwrap().unwrap(),
            JobTerminal {
                identity: first,
                outcome: JobOutcome::Cancelled
            }
        );
        assert_eq!(jobs.registered_items(), 0);
    });
}
#[test]
fn failure_and_child_panic_are_joined_typed_terminal_outcomes() {
    run(async {
        let mut jobs = registry(2, 8192);
        let failed = jobs
            .spawn(&intent(6), &context("failed"), deadline(), &bindings())
            .unwrap();
        assert_eq!(
            jobs.next_terminal().await.unwrap().unwrap(),
            JobTerminal {
                identity: failed,
                outcome: JobOutcome::Failed(JobFailure::Unavailable)
            }
        );
        let panicked = jobs
            .spawn(&intent(7), &context("panic"), deadline(), &bindings())
            .unwrap();
        assert_eq!(
            jobs.next_terminal().await.unwrap().unwrap(),
            JobTerminal {
                identity: panicked,
                outcome: JobOutcome::Panicked
            }
        );
        assert_eq!(jobs.registered_items(), 0);
    });
}
#[test]
fn wrong_identity_cancellation_is_inert_and_exact_cancellation_waits_for_join() {
    run(async {
        let mut jobs = registry(2, 8192);
        let identity = jobs
            .spawn(&intent(8), &context("pending"), deadline(), &bindings())
            .unwrap();
        for variant in 0..8 {
            let mut stale = identity;
            match variant {
                0 => stale.completion.generation += 1,
                1 => stale.effect = EffectId::from_bytes(&[15; 16]).unwrap(),
                2 => stale.basis.session = SessionId::from_bytes(&[15; 16]).unwrap(),
                3 => stale.basis.run = RunId::from_bytes(&[15; 16]).unwrap(),
                4 => stale.basis.revision = SessionRevision::new(RecoveryEpoch::new(3).unwrap(), 9),
                5 => stale.completion.run = RunId::from_bytes(&[15; 16]).unwrap(),
                6 => stale.completion.operation = OperationId::from_bytes(&[15; 16]).unwrap(),
                _ => stale.completion.job = JobId::from_bytes(&[15; 16]).unwrap(),
            }
            assert!(!jobs.cancel(stale));
        }
        assert!(jobs.contains(identity));
        assert!(jobs.cancel(identity));
        assert!(jobs.contains(identity));
        assert_eq!(
            jobs.next_terminal().await.unwrap().unwrap(),
            JobTerminal {
                identity,
                outcome: JobOutcome::Cancelled
            }
        );
        assert_eq!(jobs.registered_items(), 0);
    });
}
#[test]
fn controlled_owner_wake_expires_only_due_registration_and_joins_before_release() {
    run(async {
        let mut jobs = registry(2, 8192);
        let due = deadline();
        let early = jobs
            .spawn(&intent(9), &context("pending"), due, &bindings())
            .unwrap();
        let later = jobs
            .spawn(
                &intent(10),
                &context("pending"),
                due + Duration::from_secs(1),
                &bindings(),
            )
            .unwrap();
        assert_eq!(jobs.expire_due(due), 1);
        assert_eq!(jobs.expire_due(due), 0);
        assert!(jobs.contains(early));
        assert!(jobs.contains(later));
        assert_eq!(
            jobs.next_terminal().await.unwrap().unwrap(),
            JobTerminal {
                identity: early,
                outcome: JobOutcome::Deadline
            }
        );
        assert!(jobs.contains(later));
        jobs.retire();
        assert_eq!(jobs.next_terminal().await.unwrap().unwrap().identity, later);
        assert_eq!(jobs.registered_items(), 0);
        assert_eq!(jobs.retained_bytes(), Some(0));
        assert!(jobs.next_terminal().await.unwrap().is_none());
    });
}
#[test]
fn preparation_failure_and_panic_remove_only_unspawned_registration() {
    run(async {
        let mut jobs = registry(2, 8192);
        let failing = [JobExecutor {
            kind: EffectKind::RunAi,
            prepare: refuse_preparation,
        }];
        assert_eq!(
            jobs.spawn(&intent(11), &context("complete"), deadline(), &failing),
            Err(JobRefusal::Preparation(JobFailure::Capacity))
        );
        assert_eq!(jobs.registered_items(), 0);
        let panicking = [JobExecutor {
            kind: EffectKind::RunAi,
            prepare: panic_preparation,
        }];
        assert_eq!(
            jobs.spawn(&intent(11), &context("complete"), deadline(), &panicking),
            Err(JobRefusal::PreparationPanicked)
        );
        assert_eq!(jobs.registered_items(), 0);
        assert_eq!(POLLS[11].load(Ordering::SeqCst), 0);
    });
}
#[test]
fn retired_owner_rejects_new_work_and_invalid_owner_is_not_allocated() {
    run(async {
        let mut jobs = registry(1, 8192);
        jobs.retire();
        assert_eq!(
            jobs.spawn(&intent(12), &context("complete"), deadline(), &bindings()),
            Err(JobRefusal::Closed)
        );
        assert!(jobs.next_terminal().await.unwrap().is_none());
        let invalid = JobRegistry::new(
            JobOwner {
                session: intent(12).basis.session,
                run: intent(12).basis.run,
                generation: 0,
            },
            JobLimits {
                maximum_items: 1,
                maximum_retained_bytes: 8192,
            },
        );
        assert!(matches!(invalid, Err(JobRefusal::InvalidOwner)));
    });
}
#[test]
fn absence_of_native_runtime_is_a_refusal_before_preparation() {
    let mut jobs = registry(1, 8192);
    assert_eq!(
        jobs.spawn(&intent(13), &context("complete"), deadline(), &bindings()),
        Err(JobRefusal::RuntimeUnavailable)
    );
    assert_eq!(jobs.registered_items(), 0);
    assert_eq!(POLLS[13].load(Ordering::SeqCst), 0);
}
