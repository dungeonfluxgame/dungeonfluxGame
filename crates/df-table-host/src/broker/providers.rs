//! Fake providers for the text and image lanes, with scripted latency and failures, and the worker
//! that pumps them against a hub.
//!
//! A fake never sleeps and never spawns: it schedules each call it is given for `now + latency`
//! and hands back what is due when asked, so a test or the dev engine drives it with the clock it
//! owns. The worker is the stand-in for the one that owns real provider tasks: it claims work for
//! each lane, marks each call sent before it starts it, cancels what the hub abandoned, delivers
//! what is due and ticks the hub. A real provider is a new worker over the same hub calls.
//!
//! A fake answers from its script, one entry per call in order, and from its default latency and
//! success when the script has run out. A call that `Hangs` never answers, which is how a deadline
//! or a lease is made to expire.

use std::collections::VecDeque;
use std::sync::Arc;

use df_table::ports::PurposeName;
use df_table::private_data::Narration;

use super::hub::BrokerHub;
use super::limits::MAXIMUM_SCHEDULED_CALLS;
use super::purpose::{ResourceKind, SizeRule};
use super::request::{AttemptId, ProviderFault, ResourceResult, WorkerId};
use super::scheduler::ClaimedCall;

/// The length of the text a fake produces for a success: short, so a test and the dev engine never
/// build a page of text for nothing.
const FAKE_TEXT_CHARACTERS: usize = 64;

/// What a scripted call does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeOutcome {
    Succeeds,
    Fails(ProviderFault),
    /// Never answers.
    Hangs,
    /// Answers with a result one past the size rule.
    ReturnsOversize,
}

/// One scripted call: how long it takes and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptedCall {
    pub latency_ms: u64,
    pub outcome: FakeOutcome,
}

impl ScriptedCall {
    pub fn succeeds_after(latency_ms: u64) -> Self {
        Self {
            latency_ms,
            outcome: FakeOutcome::Succeeds,
        }
    }

    pub fn fails_after(latency_ms: u64, fault: ProviderFault) -> Self {
        Self {
            latency_ms,
            outcome: FakeOutcome::Fails(fault),
        }
    }

    pub fn hangs() -> Self {
        Self {
            latency_ms: 0,
            outcome: FakeOutcome::Hangs,
        }
    }

    pub fn returns_oversize_after(latency_ms: u64) -> Self {
        Self {
            latency_ms,
            outcome: FakeOutcome::ReturnsOversize,
        }
    }
}

struct Scheduled {
    attempt: AttemptId,
    ready_at_ms: u64,
    outcome: FakeOutcome,
    size: SizeRule,
}

/// A provider that answers from a script, on the time it is given.
pub struct FakeProvider {
    kind: ResourceKind,
    default_latency_ms: u64,
    script: VecDeque<ScriptedCall>,
    scheduled: Vec<Scheduled>,
    started: VecDeque<PurposeName>,
    started_count: u64,
}

impl FakeProvider {
    /// A provider whose calls succeed after `default_latency_ms` unless the script says otherwise.
    pub fn new(kind: ResourceKind, default_latency_ms: u64) -> Self {
        Self {
            kind,
            default_latency_ms,
            script: VecDeque::new(),
            scheduled: Vec::new(),
            started: VecDeque::new(),
            started_count: 0,
        }
    }

    pub fn kind(&self) -> ResourceKind {
        self.kind
    }

    /// Adds the next scripted calls, in order.
    pub fn scripted(mut self, calls: impl IntoIterator<Item = ScriptedCall>) -> Self {
        self.script.extend(calls);
        self
    }

    /// Starts a call at `now_ms`.
    pub fn submit(&mut self, call: &ClaimedCall, now_ms: u64) {
        let scripted = self
            .script
            .pop_front()
            .unwrap_or_else(|| ScriptedCall::succeeds_after(self.default_latency_ms));
        self.started_count += 1;
        self.started.push_back(call.purpose.clone());
        while self.started.len() > MAXIMUM_SCHEDULED_CALLS {
            self.started.pop_front();
        }
        if scripted.outcome == FakeOutcome::Hangs || self.scheduled.len() >= MAXIMUM_SCHEDULED_CALLS
        {
            return;
        }
        self.scheduled.push(Scheduled {
            attempt: call.attempt,
            ready_at_ms: now_ms.saturating_add(scripted.latency_ms),
            outcome: scripted.outcome,
            size: call.size,
        });
    }

    /// Forgets a call: the hub abandoned it.
    pub fn cancel(&mut self, attempt: AttemptId) -> bool {
        let before = self.scheduled.len();
        self.scheduled.retain(|call| call.attempt != attempt);
        self.scheduled.len() != before
    }

    /// The answers due at `now_ms`, in the order they became ready.
    pub fn due(&mut self, now_ms: u64) -> Vec<(AttemptId, Result<ResourceResult, ProviderFault>)> {
        let (mut ready, waiting): (Vec<Scheduled>, Vec<Scheduled>) = self
            .scheduled
            .drain(..)
            .partition(|call| call.ready_at_ms <= now_ms);
        self.scheduled = waiting;
        ready.sort_by_key(|call| (call.ready_at_ms, call.attempt));
        ready
            .into_iter()
            .map(|call| (call.attempt, answer_for(call.outcome, call.size)))
            .collect()
    }

    /// How many calls this provider was given.
    pub fn calls_started(&self) -> u64 {
        self.started_count
    }

    /// The purposes of the calls it was given, oldest first (the newest
    /// `MAXIMUM_SCHEDULED_CALLS`).
    pub fn started_purposes(&self) -> impl Iterator<Item = &PurposeName> {
        self.started.iter()
    }
}

fn answer_for(outcome: FakeOutcome, size: SizeRule) -> Result<ResourceResult, ProviderFault> {
    match outcome {
        FakeOutcome::Fails(fault) => Err(fault),
        FakeOutcome::Hangs => Err(ProviderFault::Unavailable),
        FakeOutcome::Succeeds => Ok(result_within(size)),
        FakeOutcome::ReturnsOversize => Ok(result_past(size)),
    }
}

fn result_within(size: SizeRule) -> ResourceResult {
    match size {
        SizeRule::TextCharacters { maximum } => {
            let length = usize::try_from(maximum).map_or(FAKE_TEXT_CHARACTERS, |maximum| {
                maximum.min(FAKE_TEXT_CHARACTERS)
            });
            ResourceResult::Text(Narration::new("a".repeat(length)))
        }
        SizeRule::ImagePixels { width, height } => ResourceResult::Image {
            width,
            height,
            bytes: u64::from(width) * u64::from(height),
        },
    }
}

/// A result one character or pixel past the rule.
fn result_past(size: SizeRule) -> ResourceResult {
    match size {
        SizeRule::TextCharacters { maximum } => {
            let length = usize::try_from(maximum).map_or(usize::MAX, |maximum| maximum + 1);
            ResourceResult::Text(Narration::new("a".repeat(length)))
        }
        SizeRule::ImagePixels { width, height } => {
            let (width, height) = (width.saturating_add(1), height.saturating_add(1));
            ResourceResult::Image {
                width,
                height,
                bytes: u64::from(width) * u64::from(height),
            }
        }
    }
}

/// What one pump did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PumpReport {
    pub started: usize,
    pub delivered: usize,
}

/// The stand-in for the worker that owns real provider tasks.
///
/// It never heartbeats: a fake call finishes in well under a lease (`LEASE_MS`, 60 s). A real
/// worker whose calls take longer (images take 120 to 240 s) must call `BrokerHub::heartbeat`
/// for each running attempt, or its calls fail as expired leases.
pub struct FakeWorker {
    hub: Arc<BrokerHub>,
    worker: WorkerId,
    text: FakeProvider,
    image: FakeProvider,
}

impl FakeWorker {
    pub fn new(
        hub: Arc<BrokerHub>,
        worker: WorkerId,
        text: FakeProvider,
        image: FakeProvider,
    ) -> Self {
        Self {
            hub,
            worker,
            text,
            image,
        }
    }

    pub fn text(&self) -> &FakeProvider {
        &self.text
    }

    pub fn image(&self) -> &FakeProvider {
        &self.image
    }

    fn provider(&mut self, kind: ResourceKind) -> &mut FakeProvider {
        match kind {
            ResourceKind::Text => &mut self.text,
            ResourceKind::Image => &mut self.image,
        }
    }

    /// One round of work at `now_ms`, the hub's time: cancel what was abandoned, start what waits,
    /// deliver what is due, and let the hub enforce its deadlines and leases.
    pub fn pump(&mut self, now_ms: u64) -> PumpReport {
        let mut report = PumpReport::default();
        self.hub.tick();
        for attempt in self.hub.take_cancellations() {
            self.text.cancel(attempt);
            self.image.cancel(attempt);
        }
        for kind in ResourceKind::ALL {
            while let Some(call) = self.hub.claim(*kind, self.worker) {
                if self.hub.mark_sent(call.attempt).is_ok() {
                    self.provider(*kind).submit(&call, now_ms);
                    report.started += 1;
                }
            }
        }
        let mut due = self.text.due(now_ms);
        due.extend(self.image.due(now_ms));
        for (attempt, answer) in due {
            self.hub.complete(attempt, answer);
            report.delivered += 1;
        }
        self.hub.tick();
        report
    }
}
