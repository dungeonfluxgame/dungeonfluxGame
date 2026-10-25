use super::*;
use df_model::checkpoint::{
    Basis, CommandInput, EntityId, GameCommand, HostCommand, HostInput, JobCompletion, JobId,
    JobOutcome, LogicalTime, NativeFailure, PresentationObservation, PresentationReport, RecordId,
    TimerExpiry, TimerId,
};
use df_types::{
    ClientBindingId, MemberId, OperationId, RecoveryEpoch, RunId, SessionId, SessionRevision,
};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;

fn basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        run: RunId::from_bytes(&[2; 16]).unwrap(),
        revision: SessionRevision::new(RecoveryEpoch::new(3).unwrap(), 4),
    }
}

fn input(value: u8, heap_capacity: usize) -> GameInput {
    GameInput::Game(CommandInput {
        basis: basis(),
        observed_revision: basis().revision,
        operation: OperationId::from_bytes(&[value; 16]).unwrap(),
        member: MemberId::from_bytes(&[6; 16]).unwrap(),
        command: GameCommand::Speak {
            speaker: EntityId::from_bytes(&[7; 16]).unwrap(),
            text: String::with_capacity(heap_capacity),
            conversation: None,
        },
    })
}

fn canonical_variants() -> Vec<GameInput> {
    vec![
        input(1, 0),
        GameInput::Host(HostInput {
            basis: basis(),
            operation: OperationId::from_bytes(&[2; 16]).unwrap(),
            host: MemberId::from_bytes(&[6; 16]).unwrap(),
            command: HostCommand::RequestCheckpoint,
        }),
        GameInput::Job(JobCompletion {
            basis: basis(),
            operation: OperationId::from_bytes(&[3; 16]).unwrap(),
            job: JobId::from_bytes(&[8; 16]).unwrap(),
            generation: 9,
            outcome: JobOutcome::Failed(NativeFailure::Unavailable),
        }),
        GameInput::Timer(TimerExpiry {
            basis: basis(),
            timer: TimerId::from_bytes(&[10; 16]).unwrap(),
            generation: 11,
            observed_time: LogicalTime {
                ticks: 12,
                ticks_per_second: 13,
            },
        }),
        GameInput::Presentation(PresentationReport {
            basis: basis(),
            binding: ClientBindingId::from_bytes(&[14; 16]).unwrap(),
            cue: RecordId::from_bytes(&[15; 16]).unwrap(),
            observation: PresentationObservation::Finished,
            presentation_ticks: 16,
        }),
    ]
}

fn submit(handle: &InboxHandle<GameInput>, input: GameInput) -> AdmissionSequence {
    match handle.try_submit(input) {
        Ok(sequence) => sequence,
        Err(refused) => panic!("{:?}", refused.reason),
    }
}

struct RecordingReducer {
    observed: Vec<(AdmissionSequence, GameInput)>,
}
impl Reducer<GameInput> for RecordingReducer {
    fn reduce(&mut self, sequence: AdmissionSequence, input: GameInput) {
        self.observed.push((sequence, input));
    }
}

#[test]
fn item_overflow_preserves_canonical_input_and_does_not_advance_sequence() {
    let (handle, actor) = inbox_with_limits(2, MAX_INBOX_BYTES);
    assert_eq!(submit(&handle, input(10, 0)), AdmissionSequence(1));
    assert_eq!(submit(&handle, input(20, 0)), AdmissionSequence(2));
    let refused = handle.try_submit(input(30, 0)).err().unwrap();
    assert_eq!(refused.reason, AdmissionRefusal::ItemCapacity);
    assert_eq!(refused.input, input(30, 0));
    assert_eq!(handle.usage().unwrap().retained_items, 2);
    handle.stop().unwrap();
    let mut reducer = RecordingReducer {
        observed: Vec::new(),
    };
    let outcome = actor.run(&mut reducer).unwrap();
    assert_eq!(
        reducer.observed,
        vec![
            (AdmissionSequence(1), input(10, 0)),
            (AdmissionSequence(2), input(20, 0))
        ]
    );
    assert_eq!(
        outcome,
        DrainOutcome {
            reduced_inputs: 2,
            last_sequence: Some(AdmissionSequence(2))
        }
    );
}

#[test]
fn byte_overflow_uses_actual_model_capacity_even_when_string_is_empty() {
    let inline = size_of::<GameInput>();
    let first = input(1, 8);
    let exact_bytes = first.retained_bytes().unwrap() + inline;
    assert!(first.retained_heap_bytes().unwrap() >= 8);
    let (handle, actor) = inbox_with_limits(4, exact_bytes);
    submit(&handle, first);
    submit(&handle, input(2, 0));
    let refused = handle.try_submit(input(3, 0)).err().unwrap();
    assert_eq!(refused.reason, AdmissionRefusal::ByteCapacity);
    assert_eq!(handle.usage().unwrap().retained_bytes, exact_bytes);
    handle.stop().unwrap();
    actor
        .run(&mut RecordingReducer {
            observed: Vec::new(),
        })
        .unwrap();
    assert_eq!(handle.usage().unwrap().retained_bytes, 0);
}

#[test]
fn oversized_and_unrepresentable_inputs_refuse_before_queue_reservation() {
    let oversized = input(1, MAX_INBOX_BYTES);
    let (handle, actor) = bounded_inbox();
    let refused = handle.try_submit(oversized).err().unwrap();
    assert_eq!(refused.reason, AdmissionRefusal::Oversize);
    assert_eq!(handle.usage().unwrap().retained_items, 0);
    handle.stop().unwrap();
    actor
        .run(&mut RecordingReducer {
            observed: Vec::new(),
        })
        .unwrap();
    struct Overflow(bool);
    impl ActorInput for Overflow {
        fn retained_heap_bytes(&self) -> Option<usize> {
            Some(usize::MAX)
        }
    }
    let (overflow, _actor) = bounded_inbox::<Overflow>();
    let refused = overflow.try_submit(Overflow(true)).err().unwrap();
    assert_eq!(refused.reason, AdmissionRefusal::RetainedBytesOverflow);
    assert!(refused.input.0);
    assert_eq!(overflow.usage().unwrap().retained_bytes, 0);
}

#[test]
fn all_five_real_model_variants_share_one_fifo_and_stop_drain_path() {
    let (handle, actor) = bounded_inbox();
    let producer = handle.clone();
    let mut expected = Vec::new();
    for input in canonical_variants() {
        let sequence = submit(&producer, input.clone());
        expected.push((sequence, input));
    }
    handle.stop().unwrap();
    producer.stop().unwrap();
    assert_eq!(
        producer.try_submit(input(20, 0)).err().unwrap().reason,
        AdmissionRefusal::Closed
    );
    let mut reducer = RecordingReducer {
        observed: Vec::new(),
    };
    let outcome = actor.run(&mut reducer).unwrap();
    assert_eq!(reducer.observed, expected);
    assert_eq!(outcome.reduced_inputs, 5);
    assert_eq!(
        handle.usage().unwrap(),
        InboxUsage {
            accepting: false,
            retained_items: 0,
            retained_bytes: 0
        }
    );
}

struct PausedReducer {
    entered: SyncSender<()>,
    resume: Receiver<()>,
    observed: Vec<GameInput>,
}
impl Reducer<GameInput> for PausedReducer {
    fn reduce(&mut self, _sequence: AdmissionSequence, input: GameInput) {
        self.entered.send(()).unwrap();
        self.resume.recv().unwrap();
        self.observed.push(input);
    }
}

#[test]
fn active_reduction_keeps_both_budgets_reserved_and_stop_does_not_cancel_it() {
    let retained_bytes = input(7, 8).retained_bytes().unwrap();
    let (handle, actor) = inbox_with_limits(1, retained_bytes);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    submit(&handle, input(7, 8));
    let worker = thread::spawn(move || {
        let mut reducer = PausedReducer {
            entered: entered_tx,
            resume: resume_rx,
            observed: Vec::new(),
        };
        let outcome = actor.run(&mut reducer).unwrap();
        (outcome, reducer.observed)
    });
    entered_rx.recv().unwrap();
    assert_eq!(handle.usage().unwrap().retained_items, 1);
    assert_eq!(handle.usage().unwrap().retained_bytes, retained_bytes);
    assert_eq!(
        handle.try_submit(input(8, 0)).err().unwrap().reason,
        AdmissionRefusal::ItemCapacity
    );
    handle.stop().unwrap();
    assert_eq!(
        handle.try_submit(input(9, 0)).err().unwrap().reason,
        AdmissionRefusal::Closed
    );
    resume_tx.send(()).unwrap();
    let (outcome, observed) = worker.join().unwrap();
    assert_eq!(outcome.reduced_inputs, 1);
    assert_eq!(observed, vec![input(7, 8)]);
    assert_eq!(handle.usage().unwrap().retained_items, 0);
}

#[test]
fn concurrent_producers_reduce_by_atomic_admission_sequence() {
    let (handle, actor) = bounded_inbox();
    let workers: Vec<_> = (1..=16)
        .map(|value| {
            let handle = handle.clone();
            thread::spawn(move || (submit(&handle, input(value, 0)), input(value, 0)))
        })
        .collect();
    let mut admitted: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    admitted.sort_by_key(|(sequence, _)| sequence.0);
    handle.stop().unwrap();
    let mut reducer = RecordingReducer {
        observed: Vec::new(),
    };
    let outcome = actor.run(&mut reducer).unwrap();
    assert_eq!(reducer.observed, admitted);
    assert_eq!(outcome.reduced_inputs, 16);
}

#[test]
fn losing_receiver_closes_all_producers_without_fabricating_a_decision() {
    let (handle, actor) = bounded_inbox();
    let producer = handle.clone();
    submit(&handle, input(1, 0));
    drop(actor);
    assert_eq!(
        producer.try_submit(input(2, 0)).err().unwrap().reason,
        AdmissionRefusal::Closed
    );
    assert_eq!(
        handle.usage().unwrap(),
        InboxUsage {
            accepting: false,
            retained_items: 0,
            retained_bytes: 0
        }
    );
}

#[test]
fn default_item_bound_refuses_the_two_hundred_fifty_seventh_canonical_input() {
    let (handle, actor) = bounded_inbox();
    for _ in 0..MAX_INBOX_ITEMS {
        submit(&handle, input(1, 0));
    }
    assert_eq!(
        handle.try_submit(input(2, 0)).err().unwrap().reason,
        AdmissionRefusal::ItemCapacity
    );
    assert_eq!(handle.usage().unwrap().retained_items, MAX_INBOX_ITEMS);
    handle.stop().unwrap();
    let outcome = actor
        .run(&mut RecordingReducer {
            observed: Vec::new(),
        })
        .unwrap();
    assert_eq!(outcome.reduced_inputs, MAX_INBOX_ITEMS as u64);
}

#[test]
fn default_byte_bound_admits_exact_input_capacity_then_refuses_another_input() {
    let payload_capacity = MAX_INBOX_BYTES - size_of::<GameInput>();
    let exact = input(1, payload_capacity);
    assert_eq!(exact.retained_bytes().unwrap(), MAX_INBOX_BYTES);
    let (handle, actor) = bounded_inbox();
    submit(&handle, exact);
    assert_eq!(handle.usage().unwrap().retained_bytes, MAX_INBOX_BYTES);
    assert_eq!(
        handle.try_submit(input(2, 0)).err().unwrap().reason,
        AdmissionRefusal::ByteCapacity
    );
    handle.stop().unwrap();
    let outcome = actor
        .run(&mut RecordingReducer {
            observed: Vec::new(),
        })
        .unwrap();
    assert_eq!(outcome.reduced_inputs, 1);
}

struct ReplenishingReducer {
    handle: InboxHandle<GameInput>,
    entered: SyncSender<usize>,
    resume: Receiver<()>,
    deadline: Option<Instant>,
    observed: Vec<(AdmissionSequence, GameInput)>,
    wakes: usize,
}

impl Reducer<GameInput> for ReplenishingReducer {
    fn reduce(&mut self, sequence: AdmissionSequence, input: GameInput) {
        self.observed.push((sequence, input));
        self.entered.send(self.observed.len()).unwrap();
        self.resume
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
    }
}

#[test]
fn due_owner_wake_progresses_while_a_producer_keeps_admitted_input_queued() {
    const INPUTS: usize = 32;
    let (handle, actor) = bounded_inbox();
    submit(&handle, input(1, 0));
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let owner_handle = handle.clone();
    let worker = thread::spawn(move || {
        let mut reducer = ReplenishingReducer {
            handle: owner_handle,
            entered: entered_tx,
            resume: resume_rx,
            deadline: Some(Instant::now()),
            observed: Vec::new(),
            wakes: 0,
        };
        let outcome = actor
            .run_with_owner_wake(
                &mut reducer,
                |reducer| reducer.deadline,
                |reducer| {
                    assert!(reducer.handle.usage().is_ok());
                    reducer.wakes += 1;
                    reducer.deadline = None;
                },
            )
            .unwrap();
        (reducer, outcome)
    });

    // The successor is queued before each reduction returns. Neither a sleep nor
    // a timing-sensitive flood is needed to keep every owner turn nonempty.
    for processed in 1..=INPUTS {
        assert_eq!(
            entered_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            processed
        );
        if processed < INPUTS {
            assert_eq!(
                submit(&handle, input((processed + 1) as u8, 0)),
                AdmissionSequence((processed + 1) as u64)
            );
            assert_eq!(handle.usage().unwrap().retained_items, 2);
        } else {
            handle.stop().unwrap();
        }
        resume_tx.send(()).unwrap();
    }

    let (reducer, outcome) = worker.join().unwrap();
    assert_eq!(reducer.wakes, 1);
    assert_eq!(
        reducer.observed,
        (1..=INPUTS)
            .map(|value| (AdmissionSequence(value as u64), input(value as u8, 0)))
            .collect::<Vec<_>>()
    );
    assert_eq!(outcome.reduced_inputs, INPUTS as u64);
    assert_eq!(
        outcome.last_sequence,
        Some(AdmissionSequence(INPUTS as u64))
    );
    assert_eq!(
        handle.usage().unwrap(),
        InboxUsage {
            accepting: false,
            retained_items: 0,
            retained_bytes: 0,
        }
    );
}

struct DueRecordingReducer {
    handle: InboxHandle<GameInput>,
    observed: Vec<(AdmissionSequence, GameInput)>,
    stop_after: usize,
    wakes_before_inputs: Vec<usize>,
}

impl Reducer<GameInput> for DueRecordingReducer {
    fn reduce(&mut self, sequence: AdmissionSequence, input: GameInput) {
        self.observed.push((sequence, input));
        if self.observed.len() == self.stop_after {
            self.handle.stop().unwrap();
        }
    }
}

#[test]
fn persistently_due_owner_work_alternates_with_fifo_inputs_until_stop() {
    let (handle, actor) = bounded_inbox();
    submit(&handle, input(1, 0));
    submit(&handle, input(2, 0));
    let due = Instant::now();
    let mut reducer = DueRecordingReducer {
        handle: handle.clone(),
        observed: Vec::new(),
        stop_after: 2,
        wakes_before_inputs: Vec::new(),
    };
    let outcome = actor
        .run_with_owner_wake(
            &mut reducer,
            |_| Some(due),
            |reducer| {
                assert!(reducer.handle.usage().is_ok());
                reducer.wakes_before_inputs.push(reducer.observed.len());
                // Keep a broken wake-priority implementation finite: its third
                // callback closes ingress instead of spinning forever.
                if reducer.wakes_before_inputs.len() == 3 {
                    reducer.handle.stop().unwrap();
                }
            },
        )
        .unwrap();
    assert_eq!(reducer.wakes_before_inputs, vec![0, 1]);
    assert_eq!(
        reducer.observed,
        vec![
            (AdmissionSequence(1), input(1, 0)),
            (AdmissionSequence(2), input(2, 0)),
        ]
    );
    assert_eq!(outcome.reduced_inputs, 2);
    assert_eq!(handle.usage().unwrap().retained_items, 0);
    assert_eq!(handle.usage().unwrap().retained_bytes, 0);
}

#[test]
fn stop_from_due_owner_wake_cancels_later_wakes_and_drains_admitted_inputs() {
    let (handle, actor) = bounded_inbox();
    submit(&handle, input(1, 0));
    submit(&handle, input(2, 0));
    let due = Instant::now();
    let mut reducer = DueRecordingReducer {
        handle: handle.clone(),
        observed: Vec::new(),
        stop_after: usize::MAX,
        wakes_before_inputs: Vec::new(),
    };
    let outcome = actor
        .run_with_owner_wake(
            &mut reducer,
            |_| Some(due),
            |reducer| {
                reducer.wakes_before_inputs.push(reducer.observed.len());
                reducer.handle.stop().unwrap();
            },
        )
        .unwrap();
    assert_eq!(reducer.wakes_before_inputs, vec![0]);
    assert_eq!(
        reducer.observed,
        vec![
            (AdmissionSequence(1), input(1, 0)),
            (AdmissionSequence(2), input(2, 0)),
        ]
    );
    assert_eq!(outcome.reduced_inputs, 2);
    assert_eq!(
        handle.usage().unwrap(),
        InboxUsage {
            accepting: false,
            retained_items: 0,
            retained_bytes: 0,
        }
    );
}

/// A reducer that must never run: this test admits no input.
struct NoInputReducer;

impl Reducer<GameInput> for NoInputReducer {
    fn reduce(&mut self, _sequence: AdmissionSequence, _input: GameInput) {
        panic!("no input is admitted in this test");
    }
}

/// A wake whose deadline stays due with nothing admitted used to be called again at once, in a
/// tight loop that spun the owner thread. Before the pause, five wakes took microseconds; now
/// each pair of wakes is at least one pause apart.
#[test]
fn a_due_wake_that_stays_due_without_input_is_paused_between_calls() {
    let (handle, actor) = bounded_inbox::<GameInput>();
    let due = Instant::now();
    let mut reducer = NoInputReducer;
    let mut wake_times = Vec::new();
    let outcome = actor
        .run_with_owner_wake(
            &mut reducer,
            |_| Some(due),
            |_| {
                wake_times.push(Instant::now());
                if wake_times.len() == 5 {
                    handle.stop().unwrap();
                }
            },
        )
        .unwrap();
    assert_eq!(outcome.reduced_inputs, 0);
    assert_eq!(wake_times.len(), 5);
    // Four gaps of at least the 10 ms pause. The bound is written out, not read from the
    // constant, so shrinking the pause fails this test instead of weakening it.
    let minimum = std::time::Duration::from_millis(40);
    let spread = wake_times[4].duration_since(wake_times[0]);
    assert!(
        spread >= minimum,
        "five stalled wakes spanned {spread:?}, expected at least {minimum:?}"
    );
}

/// Records owner turns in order: each wake, then each reduction.
struct OrderedReducer {
    handle: InboxHandle<GameInput>,
    events: Vec<&'static str>,
}

impl Reducer<GameInput> for OrderedReducer {
    fn reduce(&mut self, _sequence: AdmissionSequence, _input: GameInput) {
        self.events.push("reduce");
        self.handle.stop().unwrap();
    }
}

/// An input admitted while the owner schedules its next wake runs outside the queue lock. The
/// owner must reduce that input before any further wake or idle pause. Before the fix, the owner
/// released the lock, scheduled, retook it and slept 10 ms with the input queued, so the events
/// were wake, wake, reduce.
#[test]
fn input_admitted_in_the_post_wake_gap_is_reduced_without_a_pause() {
    let (handle, actor) = bounded_inbox::<GameInput>();
    let due = Instant::now();
    // Call 1 schedules the first turn. Call 2 is scheduled right after the first wake.
    let schedules = std::cell::Cell::new(0usize);
    let mut reducer = OrderedReducer {
        handle: handle.clone(),
        events: Vec::new(),
    };
    let outcome = actor
        .run_with_owner_wake(
            &mut reducer,
            |reducer| {
                let call = schedules.get() + 1;
                schedules.set(call);
                if call == 2 {
                    submit(&reducer.handle, input(1, 0));
                }
                Some(due)
            },
            |reducer| reducer.events.push("wake"),
        )
        .unwrap();
    assert_eq!(reducer.events, vec!["wake", "reduce"]);
    assert_eq!(outcome.reduced_inputs, 1);
}
