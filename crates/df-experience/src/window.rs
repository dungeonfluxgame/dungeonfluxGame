use std::mem::size_of;
use std::time::Duration;

use df_model::checkpoint::{
    AcceptedDecision, Basis, Checkpoint, CheckpointError, CheckpointLimits, CheckpointPins, FactId,
    GameFact, ReferenceInventory,
};
use df_types::{MemberId, OperationId, RunId, SessionId, SessionRevision};

const MAX_EVENTS: usize = 4096;
const MAX_RECORD_BYTES: usize = 1024 * 1024;

/// A permitted, committed interaction supplied by the authoritative caller.
///
/// These IDs describe provenance; constructing a value does not authenticate a
/// participant, grant access, or prove a database commit. The caller owns event
/// identity and participant validation. IDs must be fixed-size nonsecret values with
/// bounded equality; borrowed content is not an ID. No speech content is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedActivity<ParticipantId, EventId> {
    pub session: SessionId,
    pub run: RunId,
    pub operation: OperationId,
    pub revision: SessionRevision,
    pub participant: ParticipantId,
    pub event: EventId,
    /// Elapsed logical time from the caller's fixed run origin.
    pub at: Duration,
}

/// Explicit retention and work bounds; no default silently chooses a policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityWindowLimits {
    /// At most 4096 inline records, additionally bounded to one MiB.
    pub max_events: usize,
    pub horizon: Duration,
}

/// Typed admission failures. A rejected observation never changes the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowError {
    InvalidLimits,
    RecordBudget,
    TimeRegression,
    FutureActivity,
    ExpiredActivity,
    WrongSession,
    WrongRun,
    WrongEpoch,
    StaleBasis,
    FutureRevision,
    EventIdConflict,
    Capacity,
}

/// Successful aggregation, including visible expiry work on an exact retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    Inserted { removed_expired: usize },
    Duplicate { removed_expired: usize },
}

/// A pure, owned window of admitted activity counts, with exact retained dedupe.
///
/// The supplied clock must be monotonic. At `now`, activity in
/// `(now - horizon, now]` is retained; before the horizon has elapsed, logical
/// origin zero is included. Expired input is rejected rather than recounted.
/// Overflow refuses admission without evicting still-current records. This value
/// owns no timer, persistence, recommendation, or mechanical state. Event identity
/// remains immutable under its caller owner; dedupe memory covers retained events,
/// not a lifetime operation ledger. Counts describe the last successful supplied
/// clock, including after an admission refusal.
///
/// The first successful observation fixes the recovery epoch. Later observations
/// require a nonregressing owner basis, even after all retained activity expires.
/// The caller creates a new window for a different recovery epoch.
#[derive(Debug)]
pub struct ActivityWindow<ParticipantId, EventId> {
    session: SessionId,
    run: RunId,
    limits: ActivityWindowLimits,
    now: Duration,
    basis: Option<SessionRevision>,
    events: Vec<AcceptedActivity<ParticipantId, EventId>>,
}

impl<ParticipantId: Copy + Eq, EventId: Copy + Eq> ActivityWindow<ParticipantId, EventId> {
    pub fn new(
        session: SessionId,
        run: RunId,
        limits: ActivityWindowLimits,
    ) -> Result<Self, WindowError> {
        if limits.max_events == 0 || limits.max_events > MAX_EVENTS || limits.horizon.is_zero() {
            return Err(WindowError::InvalidLimits);
        }
        let record_bytes = size_of::<AcceptedActivity<ParticipantId, EventId>>()
            .checked_mul(limits.max_events)
            .ok_or(WindowError::RecordBudget)?;
        if record_bytes > MAX_RECORD_BYTES {
            return Err(WindowError::RecordBudget);
        }
        let mut events = Vec::new();
        events
            .try_reserve_exact(limits.max_events)
            .map_err(|_| WindowError::Capacity)?;
        Ok(Self {
            session,
            run,
            limits,
            now: Duration::ZERO,
            basis: None,
            events,
        })
    }

    /// Adds one caller-admitted event under the supplied current committed basis.
    /// Exact event identity retries return `Duplicate`; conflicting reuse refuses.
    /// One operation can contain several independently identified events.
    pub fn observe(
        &mut self,
        event: AcceptedActivity<ParticipantId, EventId>,
        basis: SessionRevision,
        now: Duration,
    ) -> Result<Observation, WindowError> {
        if now < self.now {
            return Err(WindowError::TimeRegression);
        }
        if event.session != self.session {
            return Err(WindowError::WrongSession);
        }
        if event.run != self.run {
            return Err(WindowError::WrongRun);
        }
        if event.revision.epoch() != basis.epoch() {
            return Err(WindowError::WrongEpoch);
        }
        if let Some(previous_basis) = self.basis {
            if previous_basis.epoch() != basis.epoch() {
                return Err(WindowError::WrongEpoch);
            }
            if basis.sequence() < previous_basis.sequence() {
                return Err(WindowError::StaleBasis);
            }
        }
        if event.revision.sequence() > basis.sequence() {
            return Err(WindowError::FutureRevision);
        }
        if event.at > now {
            return Err(WindowError::FutureActivity);
        }
        let cutoff = now.checked_sub(self.limits.horizon);
        if expired(event.at, cutoff) {
            return Err(WindowError::ExpiredActivity);
        }
        let duplicate = match self
            .events
            .iter()
            .find(|stored| !expired(stored.at, cutoff) && stored.event == event.event)
        {
            Some(stored) if *stored == event => true,
            Some(_) => return Err(WindowError::EventIdConflict),
            None => false,
        };
        let retained = self
            .events
            .iter()
            .filter(|stored| !expired(stored.at, cutoff))
            .count();
        if !duplicate && retained >= self.limits.max_events {
            return Err(WindowError::Capacity);
        }
        let removed_expired = self.events.len() - retained;
        self.events.retain(|stored| !expired(stored.at, cutoff));
        self.now = now;
        self.basis = Some(basis);
        if duplicate {
            Ok(Observation::Duplicate { removed_expired })
        } else {
            self.events.push(event);
            Ok(Observation::Inserted { removed_expired })
        }
    }

    /// Advances only the supplied logical clock and reports expired record count.
    pub fn advance(&mut self, now: Duration) -> Result<usize, WindowError> {
        if now < self.now {
            return Err(WindowError::TimeRegression);
        }
        let before = self.events.len();
        let cutoff = now.checked_sub(self.limits.horizon);
        self.events.retain(|event| !expired(event.at, cutoff));
        self.now = now;
        Ok(before - self.events.len())
    }

    /// Checks that a recommendation borrows this exact current owner snapshot.
    /// A window without observations still retains its constructor's session/run.
    /// Its absent revision is accepted only while empty; no revision is invented.
    /// A later caller clock requires explicit `advance` before recommendation.
    pub fn validate_current(
        &self,
        session: SessionId,
        run: RunId,
        basis: SessionRevision,
        now: Duration,
    ) -> Result<(), WindowError> {
        if session != self.session {
            return Err(WindowError::WrongSession);
        }
        if run != self.run {
            return Err(WindowError::WrongRun);
        }
        if now < self.now {
            return Err(WindowError::TimeRegression);
        }
        if now != self.now {
            return Err(WindowError::StaleBasis);
        }
        match self.basis {
            Some(current) if current.epoch() != basis.epoch() => Err(WindowError::WrongEpoch),
            Some(current) if current != basis => Err(WindowError::StaleBasis),
            None if !self.events.is_empty() => Err(WindowError::StaleBasis),
            _ => Ok(()),
        }
    }

    /// Returns retained accepted-event count without ranking or interpreting activity.
    pub fn activity_count(&self, participant: &ParticipantId) -> usize {
        self.events
            .iter()
            .filter(|event| event.participant == *participant)
            .count()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Exact retained checkpoint evidence and caller-supplied logical time.
/// Construction does not authenticate a member or prove a durable commit.
pub struct CheckpointActivityRequest<'a> {
    pub checkpoint: &'a Checkpoint,
    pub basis: Basis,
    pub pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub checkpoint_limits: CheckpointLimits,
    pub activity: AcceptedActivity<MemberId, FactId>,
    pub ordinal: u32,
    pub now: Duration,
}

/// Native source admission owns participant mapping and stable logical event time.
/// It must admit this exact retained fact/decision under the current consent and
/// supplied run clock. A fact's subject alone never establishes participation.
pub trait ActivitySourceOwner {
    fn admits_activity(
        &self,
        checkpoint: &Checkpoint,
        fact: &GameFact,
        decision: &AcceptedDecision,
        activity: &AcceptedActivity<MemberId, FactId>,
        now: Duration,
    ) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointActivityError {
    Checkpoint(CheckpointError),
    Window(WindowError),
    MissingMember,
    NotOptedIn,
    AmbiguousConsent,
    UnacceptedFact,
    FactMismatch,
    SourceRefused,
}

impl ActivityWindow<MemberId, FactId> {
    /// Observes only source-admitted accepted activity under current explicit consent.
    /// Every admission check precedes the existing atomic window observation.
    /// Neither checkpoint facts nor preferences are changed. Production native
    /// callback registration and durable consent administration remain caller-owned.
    pub fn observe_checkpoint(
        &mut self,
        request: CheckpointActivityRequest<'_>,
        owner: &impl ActivitySourceOwner,
    ) -> Result<Observation, CheckpointActivityError> {
        let checkpoint = request
            .checkpoint
            .validate_admitted(
                request.basis,
                request.pins,
                request.inventory,
                request.checkpoint_limits,
            )
            .map_err(CheckpointActivityError::Checkpoint)?;
        let activity = request.activity;
        if activity.session != request.basis.session || activity.run != request.basis.run {
            return Err(CheckpointActivityError::FactMismatch);
        }
        if !checkpoint
            .state()
            .members
            .iter()
            .any(|member| member.member == activity.participant)
        {
            return Err(CheckpointActivityError::MissingMember);
        }
        let mut consent = checkpoint
            .state()
            .activity
            .iter()
            .filter(|preference| preference.member == activity.participant);
        let opted_in = consent
            .next()
            .is_some_and(|preference| preference.spotlight_opt_in);
        if consent.next().is_some() {
            return Err(CheckpointActivityError::AmbiguousConsent);
        }
        if !opted_in {
            return Err(CheckpointActivityError::NotOptedIn);
        }
        let (fact, decision) = checkpoint
            .committed_outcome(activity.event)
            .ok_or(CheckpointActivityError::UnacceptedFact)?;
        if fact.operation != activity.operation
            || fact.revision != activity.revision
            || fact.ordinal != request.ordinal
        {
            return Err(CheckpointActivityError::FactMismatch);
        }
        if !owner.admits_activity(checkpoint, fact, decision, &activity, request.now) {
            return Err(CheckpointActivityError::SourceRefused);
        }
        self.observe(activity, request.basis.revision, request.now)
            .map_err(CheckpointActivityError::Window)
    }
}

fn expired(at: Duration, cutoff: Option<Duration>) -> bool {
    cutoff.is_some_and(|cutoff| at <= cutoff)
}
