//! The fake adapters behind the owner's ports: the dev engine's stand-ins for the model, the
//! rules owner, the clock and the dice (O.4, H0).
//!
//! Each implements a port of `df-table` and answers from a rule a reader can state; none reaches
//! a network, and none panics when asked something it cannot answer (the pure crate's testkit
//! fakes panic by design, which a running host must not). The interpreter reads a line that is
//! exactly one of the choices it was shown and otherwise asks the player to clarify: it is the
//! stand-in for the Jev call (meaning comes from a schema judgement, never a word list), and a
//! real adapter replaces it behind the same port. The clock is the only place the host reads
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
    offset_ms: u64,
}

/// Milliseconds since the host started, from the monotonic clock, that a dev verb can freeze and
/// advance (H.16 `clock`). The only `Instant` read of the engine.
pub struct DevClock {
    origin: Instant,
    state: Mutex<ClockState>,
}

impl DevClock {
    pub fn starting_now() -> Self {
        Self {
            origin: Instant::now(),
            state: Mutex::new(ClockState {
                frozen_at_ms: None,
                offset_ms: 0,
            }),
        }
    }

    fn running_ms(&self, state: &ClockState) -> u64 {
        let elapsed = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        elapsed.saturating_add(state.offset_ms)
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, ClockState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Moves the clock forward by `ms`, whether it is running or frozen.
    pub fn advance_ms(&self, ms: u64) {
        let mut state = self.locked();
        match state.frozen_at_ms.as_mut() {
            Some(frozen) => *frozen = frozen.saturating_add(ms),
            None => state.offset_ms = state.offset_ms.saturating_add(ms),
        }
    }

    /// Stops the clock where it stands.
    pub fn freeze(&self) {
        let mut state = self.locked();
        if state.frozen_at_ms.is_none() {
            state.frozen_at_ms = Some(self.running_ms(&state));
        }
    }

    /// Lets a frozen clock run again from where it stands.
    pub fn resume(&self) {
        let mut state = self.locked();
        if let Some(frozen) = state.frozen_at_ms.take() {
            let elapsed = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX);
            state.offset_ms = frozen.saturating_sub(elapsed);
        }
    }

    pub fn is_frozen(&self) -> bool {
        self.locked().frozen_at_ms.is_some()
    }
}

impl Clock for DevClock {
    fn now_ms(&self) -> u64 {
        let state = self.locked();
        state
            .frozen_at_ms
            .unwrap_or_else(|| self.running_ms(&state))
    }
}

/// The stand-in for the Jev call: a line that is exactly one of the offered choices means that
/// choice, and any other line needs clarifying.
#[derive(Debug, Default, Clone, Copy)]
pub struct DevInterpreter;

impl Interpreter for DevInterpreter {
    fn interpret(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        text: &PlayerText,
        legal_choices: &[ActionId],
    ) -> Result<Interpretation, PortError> {
        let line = text.expose().trim();
        Ok(legal_choices
            .iter()
            .find(|choice| choice.as_str() == line)
            .map_or(Interpretation::NeedsClarification, |choice| {
                Interpretation::Action(choice.clone())
            }))
    }
}

/// The dev interpreter with a switch that takes it down, as a model that is unreachable is
/// (H.16 `chaos interpreter`).
#[derive(Debug, Default)]
pub struct ChaosInterpreter {
    inner: DevInterpreter,
    down: AtomicBool,
}

impl ChaosInterpreter {
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

/// A rules owner for the dev table: one check, rolled on a d20 and always applied, and every
/// other action declined `not_supported_yet` (RES31).
#[derive(Debug, Clone)]
pub struct DevRulesOwner {
    check: ActionId,
}

impl DevRulesOwner {
    pub fn new(check: ActionId) -> Self {
        Self { check }
    }
}

impl RulesOwner for DevRulesOwner {
    fn legal_choices(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
    ) -> Result<Vec<ActionId>, PortError> {
        Ok(vec![self.check.clone()])
    }

    fn prepare(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        action: &ActionId,
    ) -> Result<Preparation, PortError> {
        if *action != self.check {
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
        let rolled_the_check_die = matches!(draws, [only] if only.sides() == CHECK_DIE);
        if *action == self.check && rolled_the_check_die {
            Ok(Resolution::Applied(Resolved::default()))
        } else {
            Ok(Resolution::Declined(Decline::with_default_reason(
                DeclineCode::NotViable,
            )))
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
}

impl BrokerSide {
    /// The built-in purposes over fake providers that always succeed after the dev latencies.
    pub fn dev(
        clock: Arc<dyn Clock>,
        audit: Arc<dyn BrokerAudit>,
    ) -> Result<Self, crate::broker::purpose::PurposeError> {
        let hub = Arc::new(BrokerHub::new(
            PurposeTable::built_in()?,
            BrokerLimits::default(),
            audit,
            clock,
        ));
        let worker = FakeWorker::new(
            Arc::clone(&hub),
            WorkerId::new(1),
            FakeProvider::new(ResourceKind::Text, DEV_TEXT_LATENCY_MS),
            FakeProvider::new(ResourceKind::Image, DEV_IMAGE_LATENCY_MS),
        );
        Ok(Self { hub, worker })
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
