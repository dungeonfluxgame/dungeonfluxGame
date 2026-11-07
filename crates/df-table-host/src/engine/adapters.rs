//! The fake adapters behind the owner's ports: the dev engine's stand-ins for the model, the
//! rules owner, the clock and the dice (O.4, H0).
//!
//! Each implements a port of `df-table` and answers from a rule a reader can state; none reaches
//! a network, and none panics when asked something it cannot answer (the pure crate's testkit
//! fakes panic by design, which a running host must not). The interpreter port is served by
//! `jev::JevInterpreter` (meaning comes from a schema judgement, never a word list), which
//! `ChaosInterpreter` wraps with its down switch. The clock is the only place the host reads
//! `Instant`; the pure crate never does.
//!
//! `Adapters` bundles one adapter per port for the owner, and `BrokerSide` pairs the broker's hub
//! with the fake worker that serves it. `DevControls` holds the concrete handles that the dev
//! verbs `clock`, `dice` and `chaos` drive; a table built from other adapters has none, and the
//! verbs answer `NotAvailable`. Tests build their own `Adapters` with scripted or failing
//! adapters through the same struct.

use std::collections::VecDeque;
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use df_table::envelope::PendingId;
use df_table::event::FailureCause;
use df_table::identity::{Correlation, FactId, SeatId};
use df_table::outcome::{Decline, DeclineCode};
use df_table::ports::{
    ActionId, Audience, Clock, CommittedOutcome, ConfirmationReason, Dice, Draw, DrawRequest,
    Interpretation, Interpreter, KnowledgeView, Narrator, PlanSoFar, PortError, PortName,
    Preparation, Resolution, Resolved, Resumption, RulesOwner, RulingRequest,
};
use df_table::private_data::{Narration, PlayerText, Secret};
use df_types::saturating_millis;

use crate::broker::providers::{FakeProvider, FakeWorker};
use crate::broker::request::WorkerId;
use crate::broker::scheduler::BrokerLimits;
use crate::broker::{BrokerAudit, BrokerHub, PurposeTable, ResourceKind};

use super::commit::ScriptedCommitStore;
use super::limits::{DEV_IMAGE_LATENCY_MS, DEV_TEXT_LATENCY_MS};

/// The die the dev rules owner asks for to make the explore check: 20 sides (the smallest
/// non-zero number, plus 19).
const CHECK_DIE: NonZeroU16 = NonZeroU16::MIN.saturating_add(19);

#[derive(Debug)]
struct ClockState {
    frozen_at_ms: Option<u64>,
    /// The clock's reading at `base`; a running clock reads this plus the time since `base`.
    base_ms: u64,
    base: Instant,
}

impl ClockState {
    fn running_ms(&self) -> u64 {
        let elapsed = saturating_millis(self.base.elapsed());
        self.base_ms.saturating_add(elapsed)
    }
}

/// Milliseconds since the host started, from the monotonic clock, that a dev verb can freeze and
/// advance (H.16 `clock`). The only `Instant` read of the engine.
pub struct DevClock {
    state: Mutex<ClockState>,
}

impl DevClock {
    pub fn starting_now() -> Self {
        Self {
            state: Mutex::new(ClockState {
                frozen_at_ms: None,
                base_ms: 0,
                base: Instant::now(),
            }),
        }
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, ClockState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Moves the clock forward by `ms`, whether it is running or frozen.
    pub fn advance_ms(&self, ms: u64) {
        let mut state = self.locked();
        match state.frozen_at_ms.as_mut() {
            Some(frozen) => *frozen = frozen.saturating_add(ms),
            None => state.base_ms = state.base_ms.saturating_add(ms),
        }
    }

    /// Stops the clock where it stands.
    pub fn freeze(&self) {
        let mut state = self.locked();
        if state.frozen_at_ms.is_none() {
            state.frozen_at_ms = Some(state.running_ms());
        }
    }

    /// Lets a frozen clock run again from where it stands.
    pub fn resume(&self) {
        let mut state = self.locked();
        if let Some(frozen) = state.frozen_at_ms.take() {
            state.base_ms = frozen;
            state.base = Instant::now();
        }
    }

    pub fn is_frozen(&self) -> bool {
        self.locked().frozen_at_ms.is_some()
    }
}

impl Clock for DevClock {
    fn now_ms(&self) -> u64 {
        let state = self.locked();
        state.frozen_at_ms.unwrap_or_else(|| state.running_ms())
    }
}

/// Any interpreter with a switch that takes it down, as a model that is unreachable is (H.16
/// `chaos interpreter`).
pub struct ChaosInterpreter {
    inner: Arc<dyn Interpreter>,
    down: AtomicBool,
}

impl ChaosInterpreter {
    pub fn new(inner: Arc<dyn Interpreter>) -> Self {
        Self {
            inner,
            down: AtomicBool::new(false),
        }
    }

    pub fn set_down(&self, down: bool) {
        // Relaxed: the flag orders no other data.
        self.down.store(down, Ordering::Relaxed);
    }

    pub fn is_down(&self) -> bool {
        self.down.load(Ordering::Relaxed)
    }
}

impl Interpreter for ChaosInterpreter {
    fn interpret(
        &self,
        correlation: Correlation,
        seat: SeatId,
        text: &PlayerText,
        legal_choices: &[ActionId],
    ) -> Result<Interpretation, PortError> {
        if self.is_down() {
            return Err(PortError::new(
                PortName::Interpreter,
                FailureCause::Unavailable,
            ));
        }
        self.inner.interpret(correlation, seat, text, legal_choices)
    }
}

/// The lowest d20 face that moves the speaker of the dev table's conversation (SRD 5.2.1 page 6
/// puts a Medium difficulty at 15; the stand-in uses the middle of the die so a forced 20 moves
/// them and a forced 1 does not).
const URGING_SUCCESS_FACE: u16 = 11;

/// A rules owner for the dev table: one check, rolled on a d20 and always applied, and every
/// other action declined `not_supported_yet` (RES31). The story table adds one urging, rolled on a
/// d20 and reported with the typed fact `succeeded` (a face of 11 or more).
#[derive(Debug, Clone)]
pub struct DevRulesOwner {
    check: ActionId,
    urging: Option<ActionId>,
}

impl DevRulesOwner {
    pub fn new(check: ActionId) -> Self {
        Self {
            check,
            urging: None,
        }
    }

    /// Also runs `urging`, the conversation's action, as a d20 that either moves the speaker or
    /// not.
    pub fn with_urging(mut self, urging: ActionId) -> Self {
        self.urging = Some(urging);
        self
    }

    fn runs(&self, action: &ActionId) -> bool {
        *action == self.check || self.urging.as_ref() == Some(action)
    }
}

impl RulesOwner for DevRulesOwner {
    fn legal_choices(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
    ) -> Result<Vec<ActionId>, PortError> {
        Ok(std::iter::once(self.check.clone())
            .chain(self.urging.clone())
            .collect())
    }

    fn prepare(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        action: &ActionId,
    ) -> Result<Preparation, PortError> {
        if !self.runs(action) {
            return Ok(Preparation::Declined(Decline::with_default_reason(
                DeclineCode::NotSupportedYet,
            )));
        }
        Ok(Preparation::Ready {
            draws: vec![DrawRequest { sides: CHECK_DIE }],
        })
    }

    fn resolve(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        action: &ActionId,
        draws: &[Draw],
    ) -> Result<Resolution, PortError> {
        let rolled = match draws {
            [only] if only.sides() == CHECK_DIE => Some(only.value()),
            _ => None,
        };
        match rolled {
            Some(_) if *action == self.check => Ok(Resolution::Applied(Resolved::default())),
            Some(face) if self.urging.as_ref() == Some(action) => {
                let succeeded = u64::from(face >= URGING_SUCCESS_FACE);
                Resolved::default()
                    .with_fact("succeeded", succeeded)
                    .map(Resolution::Applied)
                    .map_err(|_| PortError::new(PortName::RulesOwner, FailureCause::InvalidAnswer))
            }
            _ => Ok(Resolution::Declined(Decline::with_default_reason(
                DeclineCode::NotViable,
            ))),
        }
    }

    // The dev table tracks no action economy, reserves nothing, asks for no confirmation and no
    // ruling, and has no windows, so each of these says so explicitly.
    fn prepare_in_plan(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        _so_far: &PlanSoFar<'_>,
    ) -> Result<Preparation, PortError> {
        self.prepare(correlation, seat, action)
    }

    fn abandon(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        _action: &ActionId,
        _pending: PendingId,
    ) -> Result<(), PortError> {
        Ok(())
    }

    fn confirmation_needed(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        _action: &ActionId,
    ) -> Result<Option<ConfirmationReason>, PortError> {
        Ok(None)
    }

    fn ruling_needed(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        _action: &ActionId,
    ) -> Result<Option<RulingRequest>, PortError> {
        Ok(None)
    }

    fn resume(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        resumption: &Resumption<'_>,
    ) -> Result<Resolution, PortError> {
        self.resolve(correlation, seat, action, resumption.draws)
    }
}

/// A narrator that states the committed outcome in one structured line, with no model.
#[derive(Debug, Default, Clone, Copy)]
pub struct DevNarrator;

impl Narrator for DevNarrator {
    fn narrate(
        &self,
        _correlation: Correlation,
        committed: &CommittedOutcome,
    ) -> Result<Narration, PortError> {
        Ok(Narration::new(format!(
            "Seat {} did {}: {}.",
            committed.seat,
            committed.action.as_str(),
            committed.outcome.key()
        )))
    }
}

/// A knowledge view that holds no facts: the dev table has no secrets, so every audience may know
/// nothing beyond what the table's own state shows (`None` is "may not know", not a failure).
#[derive(Debug, Default, Clone, Copy)]
pub struct DevKnowledge;

impl KnowledgeView for DevKnowledge {
    fn fact_for(
        &self,
        _correlation: Correlation,
        _audience: Audience,
        _fact: &FactId,
    ) -> Result<Option<Secret<String>>, PortError> {
        Ok(None)
    }
}

#[derive(Debug)]
struct DiceState {
    queued: VecDeque<u16>,
    generator: u64,
    seed: u64,
}

/// Dice that give the queued values first, in order, and then a seeded, reproducible sequence.
#[derive(Debug)]
pub struct DevDice {
    state: Mutex<DiceState>,
}

impl DevDice {
    pub fn seeded(seed: u64) -> Self {
        Self::queued_then_seeded([], seed)
    }

    pub fn queued_then_seeded(queued: impl IntoIterator<Item = u16>, seed: u64) -> Self {
        Self {
            state: Mutex::new(DiceState {
                queued: queued.into_iter().collect(),
                generator: seed,
                seed,
            }),
        }
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, DiceState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Restarts the seeded sequence from `seed`; values already forced still come first.
    pub fn reseed(&self, seed: u64) {
        let mut state = self.locked();
        state.generator = seed;
        state.seed = seed;
    }

    /// Queues values to be rolled next, in order (H.3: a forced draw is a driven one).
    pub fn force(&self, values: impl IntoIterator<Item = u16>) {
        self.locked().queued.extend(values);
    }

    pub fn forced_waiting(&self) -> usize {
        self.locked().queued.len()
    }

    pub fn seed(&self) -> u64 {
        self.locked().seed
    }
}

/// One step of the SplitMix64 generator: small, reproducible and not a security boundary.
fn next_random(generator: &mut u64) -> u64 {
    *generator = generator.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut mixed = *generator;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^ (mixed >> 31)
}

impl Dice for DevDice {
    fn draw(&self, _correlation: Correlation, request: DrawRequest) -> Result<Draw, PortError> {
        let mut state = self.locked();
        let sides = u64::from(request.sides.get());
        let value = match state.queued.pop_front() {
            Some(queued) => queued,
            None => u16::try_from(next_random(&mut state.generator) % sides + 1).unwrap_or(1),
        };
        Draw::new(request.sides, value)
            .ok_or(PortError::new(PortName::Dice, FailureCause::InvalidAnswer))
    }

    fn give_back(&self, _correlation: Correlation, draws: &[Draw]) {
        let mut state = self.locked();
        for draw in draws.iter().rev() {
            state.queued.push_front(draw.value());
        }
    }
}

/// The broker's hub and the fake worker that serves it, which the owner pumps when it is idle.
pub struct BrokerSide {
    pub hub: Arc<BrokerHub>,
    pub worker: FakeWorker,
    /// The limits the hub was built with, which `broker queues` shows.
    pub limits: BrokerLimits,
}

impl BrokerSide {
    /// The built-in purposes over fake providers that always succeed after the dev latencies.
    pub fn dev(
        clock: Arc<dyn Clock>,
        audit: Arc<dyn BrokerAudit>,
    ) -> Result<Self, crate::broker::purpose::PurposeError> {
        let limits = BrokerLimits::default();
        let hub = Arc::new(BrokerHub::new(
            PurposeTable::built_in()?,
            limits,
            audit,
            clock,
        ));
        let worker = FakeWorker::new(
            Arc::clone(&hub),
            WorkerId::new(1),
            FakeProvider::new(ResourceKind::Text, DEV_TEXT_LATENCY_MS),
            FakeProvider::new(ResourceKind::Image, DEV_IMAGE_LATENCY_MS),
        );
        Ok(Self {
            hub,
            worker,
            limits,
        })
    }
}

/// The concrete handles the dev verbs drive: absent for a table that is not the dev table.
#[derive(Clone, Default)]
pub struct DevControls {
    pub clock: Option<Arc<DevClock>>,
    pub dice: Option<Arc<DevDice>>,
    pub interpreter: Option<Arc<ChaosInterpreter>>,
    pub commit: Option<Arc<ScriptedCommitStore>>,
}

/// One adapter for each port the owner calls.
pub struct Adapters {
    pub clock: Arc<dyn Clock>,
    pub rules: Arc<dyn RulesOwner>,
    pub interpreter: Arc<dyn Interpreter>,
    pub narrator: Arc<dyn Narrator>,
    pub dice: Arc<dyn Dice>,
    pub knowledge: Arc<dyn KnowledgeView>,
    pub broker: BrokerSide,
    pub controls: DevControls,
}
