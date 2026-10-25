use df_model::checkpoint::GameInput;
use std::collections::VecDeque;
use std::mem::size_of;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

pub const MAX_INBOX_ITEMS: usize = 256;
pub const MAX_INBOX_BYTES: usize = 2 * 1024 * 1024;
/// Pause before the owner wake runs again when its deadline is still due after the wake and
/// nothing was admitted. A reducer that keeps a deadline due with no work is a defect; this bound
/// is a safety net so the owner polls instead of spinning. The first stall of an episode is
/// reported on the audit line `session.owner_wake_stalled`. Admitted input ends the pause early.
const STALLED_WAKE_PAUSE: std::time::Duration = std::time::Duration::from_millis(10);

/// Heap capacity retained by an owned canonical input or caller-owned envelope. The inbox also
/// charges the input's inline representation. No client-supplied size is trusted.
/// Implementations must count all retained allocation capacities with checked
/// arithmetic. The GameInput implementation uses the canonical model calculation.
/// Admission provides no authentication or authority: the native caller owns
/// trusted ingress mapping and revalidation inside the serialization owner.
pub trait ActorInput: Send {
    fn retained_heap_bytes(&self) -> Option<usize>;
}

impl ActorInput for GameInput {
    fn retained_heap_bytes(&self) -> Option<usize> {
        GameInput::retained_heap_bytes(self)
    }
}

/// The concrete session owner implements this boundary. Reduction sends
/// the durable decision result separately; mailbox admission is never success.
/// A reducer must complete each call with a bounded lifetime and explicit input
/// outcome. This queue does not supply a replacement engine or commit authority.
pub trait Reducer<I: ActorInput> {
    fn reduce(&mut self, sequence: AdmissionSequence, input: I);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionSequence(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionRefusal {
    Closed,
    ItemCapacity,
    ByteCapacity,
    Oversize,
    RetainedBytesOverflow,
    SequenceExhausted,
    AllocationUnavailable,
    Poisoned,
}

pub struct RefusedInput<I> {
    pub reason: AdmissionRefusal,
    pub input: I,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InboxUsage {
    pub accepting: bool,
    pub retained_items: usize,
    pub retained_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InboxFailure {
    Poisoned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainOutcome {
    pub reduced_inputs: u64,
    pub last_sequence: Option<AdmissionSequence>,
}

struct QueuedInput<I> {
    input: I,
    sequence: AdmissionSequence,
    retained_bytes: usize,
}

struct Queue<I> {
    inputs: VecDeque<QueuedInput<I>>,
    usage: InboxUsage,
    next_sequence: u64,
    item_limit: usize,
    byte_limit: usize,
}

struct Shared<I> {
    queue: Mutex<Queue<I>>,
    ready: Condvar,
}

pub struct InboxHandle<I> {
    shared: Arc<Shared<I>>,
}

/// A single, uncloneable receiver owns consumption. The native runtime owns the
/// call to run (and joins any thread it starts); this primitive spawns no tasks.
/// Normal shutdown calls stop and then joins run after all admitted inputs drain.
/// Dropping the receiver aborts unprocessed admissions and closes all producers;
/// caller-owned receipt channels must distinguish that abort from a decision.
pub struct ActorLoop<I> {
    shared: Arc<Shared<I>>,
}

pub fn bounded_inbox<I: ActorInput>() -> (InboxHandle<I>, ActorLoop<I>) {
    inbox_with_limits(MAX_INBOX_ITEMS, MAX_INBOX_BYTES)
}

fn inbox_with_limits<I: ActorInput>(
    item_limit: usize,
    byte_limit: usize,
) -> (InboxHandle<I>, ActorLoop<I>) {
    let shared = Arc::new(Shared {
        queue: Mutex::new(Queue {
            inputs: VecDeque::new(),
            usage: InboxUsage {
                accepting: true,
                retained_items: 0,
                retained_bytes: 0,
            },
            next_sequence: 0,
            item_limit,
            byte_limit,
        }),
        ready: Condvar::new(),
    });
    (
        InboxHandle {
            shared: Arc::clone(&shared),
        },
        ActorLoop { shared },
    )
}

impl<I> Clone for InboxHandle<I> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<I: ActorInput> InboxHandle<I> {
    pub fn try_submit(&self, input: I) -> Result<AdmissionSequence, RefusedInput<I>> {
        let Some(bytes) = input
            .retained_heap_bytes()
            .and_then(|heap| heap.checked_add(size_of::<I>()))
        else {
            return Err(RefusedInput {
                reason: AdmissionRefusal::RetainedBytesOverflow,
                input,
            });
        };
        let mut queue = match self.shared.queue.lock() {
            Ok(queue) => queue,
            Err(_) => {
                return Err(RefusedInput {
                    reason: AdmissionRefusal::Poisoned,
                    input,
                });
            }
        };
        let refusal = if !queue.usage.accepting {
            Some(AdmissionRefusal::Closed)
        } else if bytes > queue.byte_limit {
            Some(AdmissionRefusal::Oversize)
        } else if queue.usage.retained_items == queue.item_limit {
            Some(AdmissionRefusal::ItemCapacity)
        } else if bytes > queue.byte_limit - queue.usage.retained_bytes {
            Some(AdmissionRefusal::ByteCapacity)
        } else if queue.next_sequence == u64::MAX {
            Some(AdmissionRefusal::SequenceExhausted)
        } else if queue.inputs.try_reserve(1).is_err() {
            Some(AdmissionRefusal::AllocationUnavailable)
        } else {
            None
        };
        if let Some(reason) = refusal {
            return Err(RefusedInput { reason, input });
        }
        queue.next_sequence += 1;
        let sequence = AdmissionSequence(queue.next_sequence);
        queue.inputs.push_back(QueuedInput {
            input,
            sequence,
            retained_bytes: bytes,
        });
        queue.usage.retained_items += 1;
        queue.usage.retained_bytes += bytes;
        drop(queue);
        self.shared.ready.notify_one();
        Ok(sequence)
    }

    /// Close under the admission lock, then wake the owner. All accepted inputs
    /// drain in admission order; cloned producers cannot reopen the inbox.
    pub fn stop(&self) -> Result<(), InboxFailure> {
        let mut queue = self
            .shared
            .queue
            .lock()
            .map_err(|_| InboxFailure::Poisoned)?;
        queue.usage.accepting = false;
        drop(queue);
        self.shared.ready.notify_one();
        Ok(())
    }

    pub fn usage(&self) -> Result<InboxUsage, InboxFailure> {
        let queue = self
            .shared
            .queue
            .lock()
            .map_err(|_| InboxFailure::Poisoned)?;
        Ok(queue.usage)
    }
}

impl<I: ActorInput> ActorLoop<I> {
    pub fn run<R: Reducer<I>>(self, reducer: &mut R) -> Result<DrainOutcome, InboxFailure> {
        self.run_with_owner_wake(reducer, |_| None, |_| {})
    }

    /// Run with a monotonic deadline owned by the same reducer. Both callbacks run
    /// outside the queue lock; early/spurious notifications recheck the deadline.
    /// Owner work creates no admission sequence and does not reorder accepted inputs.
    /// Stop cancels future wakes and drains accepted inputs as in `run`. The caller
    /// still owns the bounded callback lifetime, shutdown and thread join.
    pub fn run_with_owner_wake<R: Reducer<I>>(
        self,
        reducer: &mut R,
        next_wake: impl Fn(&R) -> Option<Instant>,
        mut wake: impl FnMut(&mut R),
    ) -> Result<DrainOutcome, InboxFailure> {
        let mut outcome = DrainOutcome {
            reduced_inputs: 0,
            last_sequence: None,
        };
        // The deadline is recomputed only after the reducer or the wake changed state, then carried.
        let mut deadline = next_wake(reducer);
        let mut stall_reported = false;
        loop {
            let mut queue = self
                .shared
                .queue
                .lock()
                .map_err(|_| InboxFailure::Poisoned)?;
            while queue.inputs.is_empty() && queue.usage.accepting {
                if let Some(deadline) = deadline {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    queue = self
                        .shared
                        .ready
                        .wait_timeout(queue, deadline - now)
                        .map_err(|_| InboxFailure::Poisoned)?
                        .0;
                } else {
                    queue = self
                        .shared
                        .ready
                        .wait(queue)
                        .map_err(|_| InboxFailure::Poisoned)?;
                }
            }
            if queue.usage.accepting && deadline.is_some_and(|deadline| Instant::now() >= deadline)
            {
                drop(queue);
                wake(reducer);
                deadline = next_wake(reducer);
                queue = self
                    .shared
                    .queue
                    .lock()
                    .map_err(|_| InboxFailure::Poisoned)?;
                // One due wake may precede the next FIFO reduction. A callback
                // that leaves its deadline due must not starve admitted inputs.
                // With no input, start another turn with the carried deadline. A deadline still
                // due after the wake pauses first, so the same wake is not called in a tight loop.
                if queue.inputs.is_empty() {
                    if !queue.usage.accepting {
                        return Ok(outcome);
                    }
                    if deadline.is_some_and(|next| next <= Instant::now()) {
                        if !stall_reported {
                            stall_reported = true;
                            report_stalled_wake(&outcome);
                        }
                        self.pause_while_idle(queue)?;
                    } else {
                        stall_reported = false;
                    }
                    continue;
                }
            }
            let Some(queued) = queue.inputs.pop_front() else {
                if !queue.usage.accepting {
                    return Ok(outcome);
                }
                continue;
            };
            drop(queue);
            // Neither queue lock nor mutable state reference escapes to producers.
            // Capacity remains charged while the owner reduces this input.
            reducer.reduce(queued.sequence, queued.input);
            stall_reported = false;
            deadline = next_wake(reducer);
            let mut queue = self
                .shared
                .queue
                .lock()
                .map_err(|_| InboxFailure::Poisoned)?;
            queue.usage.retained_items -= 1;
            queue.usage.retained_bytes -= queued.retained_bytes;
            outcome.reduced_inputs += 1;
            outcome.last_sequence = Some(queued.sequence);
        }
    }

    /// Waits on the queue while it stays empty and open, for at most one stalled-wake pause.
    /// The lock is held from the emptiness check to the wait, so an input admitted at any point
    /// in the owner's wake gap is seen here and ends the pause at once. Returns with the lock
    /// released.
    fn pause_while_idle(&self, mut queue: MutexGuard<'_, Queue<I>>) -> Result<(), InboxFailure> {
        let until = Instant::now() + STALLED_WAKE_PAUSE;
        while queue.inputs.is_empty() && queue.usage.accepting {
            let now = Instant::now();
            if now >= until {
                break;
            }
            queue = self
                .shared
                .ready
                .wait_timeout(queue, until - now)
                .map_err(|_| InboxFailure::Poisoned)?
                .0;
        }
        Ok(())
    }
}

/// One audit line for the first stall of an episode. The correlation key is the admission
/// sequence last reduced, which names the owner's work stream in the audit log.
#[cfg(not(target_arch = "wasm32"))]
fn report_stalled_wake(outcome: &DrainOutcome) {
    let after = match outcome.last_sequence {
        Some(sequence) => sequence.0.to_string(),
        None => "none".to_owned(),
    };
    df_observe::record_audit_line(
        "session.owner_wake_stalled",
        format!(
            "OWNER wake stalled after_sequence={after} reduced={} pause_ms={}",
            outcome.reduced_inputs,
            STALLED_WAKE_PAUSE.as_millis()
        ),
    );
}

/// The browser build has no audit sink (`df-observe` is native-only), so a stall is only paused.
#[cfg(target_arch = "wasm32")]
fn report_stalled_wake(_outcome: &DrainOutcome) {}

impl<I> Drop for ActorLoop<I> {
    fn drop(&mut self) {
        // Closing admissions remains necessary even when a runtime owner aborts
        // its loop. A poisoned queue stays poisoned and rejects further work.
        let mut queue = self
            .shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.usage.accepting = false;
        let pending = std::mem::take(&mut queue.inputs);
        queue.usage.retained_items = 0;
        queue.usage.retained_bytes = 0;
        drop(queue);
        drop(pending);
        self.shared.ready.notify_one();
    }
}

#[cfg(test)]
mod tests;
