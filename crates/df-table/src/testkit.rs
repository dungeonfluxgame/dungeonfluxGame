//! Scripted fakes and sample data for this crate's tests and for hosts that test against it.
//!
//! Every port has a fake here (O.4). A fake answers from a script a test wrote, never from a
//! guess. Asking it something the script does not cover panics with a message naming the port and
//! the question, so a forgotten line fails the test loudly: it is never an `Unavailable` error
//! (which the pipeline must survive, C.7) and never a plausible decline (which is a valid answer).
//! A fault a test wants is scripted on purpose (`failing`, an `Err` in the script). The fakes hold
//! no clock, no dice and no network, every one is `Send + Sync` like the port it stands for, and
//! the two that remember history (`RecordingSink`, `ScriptedBroker`) keep a bounded amount of it.

use std::collections::{BTreeMap, VecDeque};
use std::num::NonZeroU128;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::event::{FailureCause, TableEvent};
use crate::identity::{
    Correlation, FactId, OperationId, RecoveryEpoch, SeatId, SessionRevision, TurnSeq,
};
use crate::kind::KindRegistry;
use crate::kind::file::KindFile;
use crate::limits::{MAXIMUM_RECORDED_EVENTS, MAXIMUM_REMEMBERED_REQUESTS};
use crate::outcome::{Decline, DeclineCode};
use crate::ports::{
    ActionId, Audience, Broker, Clock, CommittedOutcome, ConfirmationReason, Dice, Draw,
    DrawRequest, EventSink, Interpretation, Interpreter, KnowledgeView, Narrator, PlanSoFar,
    PortError, PortName, Preparation, PurposeName, Readiness, Resolution, ResourceHandle,
    Resumption, RulesOwner, RulingAnswer, RulingRequest,
};
use crate::private_data::{Narration, PlayerText, Secret};

/// A kind with one of each field type, a label field and a color field.
pub const CHARACTER_KIND: &str = r##"{
    "name": "character", "shape": "circle", "width": 6, "height": 6,
    "label_field": "name", "color_field": "color",
    "fields": [
        {"name": "name", "type": "text", "maximum_length": 20, "required": true},
        {"name": "color", "type": "color", "default": "#4f8cff"},
        {"name": "hit_points", "type": "number", "minimum": 0, "maximum": 999, "default": 10},
        {"name": "role", "type": "choice", "choices": ["hero", "npc", "foe"], "default": "hero"},
        {"name": "hidden", "type": "flag", "default": false}
    ]
}"##;

/// A kind with no fields at all.
pub const MARKER_KIND: &str = r#"{"name": "marker", "shape": "circle", "width": 2, "height": 2}"#;

/// The text of a fact every audience may know. A test searches output for it to prove it shows.
pub const SENTINEL_PUBLIC_FACT: &str = "SENTINEL-PUBLIC-3c9e";

/// The text of a secret one seat knows. A test searches every other audience's output for it to
/// prove it never leaks.
pub const SENTINEL_SECRET: &str = "SENTINEL-SECRET-7f3a";

/// Reads kind-file texts into a registry, for tests whose kinds are known to be valid.
pub fn registry_from(sources: &[&str]) -> KindRegistry {
    let files: Vec<KindFile> = sources
        .iter()
        .map(|source| serde_json::from_str(source).expect("the test kind file parses"))
        .collect();
    KindRegistry::from_files(&files).expect("the test kinds are valid")
}

/// A correlation for the `turn_seq`th input of the first epoch: its operation id is that number
/// plus one (an id is never zero) and its revision sequence is that number, so two correlations
/// from here differ in every field.
pub fn correlation(turn_seq: u64) -> Correlation {
    let operation = NonZeroU128::MIN.saturating_add(u128::from(turn_seq));
    Correlation {
        operation_id: OperationId::from_nonzero_u128(operation),
        turn_seq: TurnSeq::new(turn_seq),
        revision: SessionRevision::new(RecoveryEpoch::FIRST, turn_seq),
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Fails the test: the script holds no answer for this question.
fn unscripted(port: PortName, question: &str) -> ! {
    panic!(
        "testkit: the {} fake was asked {question}, which its script does not cover",
        port.key()
    )
}

#[derive(Debug, Default)]
struct RecordedEvents {
    kept: VecDeque<TableEvent>,
    dropped: u64,
}

/// An event sink that keeps the newest events it is given, in order, and counts the rest.
#[derive(Debug)]
pub struct RecordingSink {
    recorded: Mutex<RecordedEvents>,
    maximum_events: usize,
}

impl Default for RecordingSink {
    fn default() -> Self {
        Self::keeping_newest(MAXIMUM_RECORDED_EVENTS)
    }
}

impl RecordingSink {
    /// A sink that keeps at most `maximum_events`, dropping the oldest first.
    pub fn keeping_newest(maximum_events: usize) -> Self {
        Self {
            recorded: Mutex::default(),
            maximum_events,
        }
    }

    /// A copy of the kept events, oldest first.
    pub fn events(&self) -> Vec<TableEvent> {
        locked(&self.recorded).kept.iter().cloned().collect()
    }

    /// How many events were dropped to stay within the maximum.
    pub fn dropped_events(&self) -> u64 {
        locked(&self.recorded).dropped
    }
}

impl EventSink for RecordingSink {
    fn record(&self, event: &TableEvent) {
        let mut recorded = locked(&self.recorded);
        recorded.kept.push_back(event.clone());
        while recorded.kept.len() > self.maximum_events {
            recorded.kept.pop_front();
            recorded.dropped += 1;
        }
    }
}

/// A clock that moves only when a test moves it.
#[derive(Debug, Default)]
pub struct ManualClock {
    now_ms: AtomicU64,
}

impl ManualClock {
    pub fn starting_at(now_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(now_ms),
        }
    }

    pub fn advance_ms(&self, delta_ms: u64) {
        // Relaxed: the clock orders no other data, a test reads it from the thread that moved it.
        self.now_ms.fetch_add(delta_ms, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::Relaxed)
    }
}

type ReadinessAnswers = VecDeque<Result<Readiness, FailureCause>>;

#[derive(Debug)]
struct IssuedRequest {
    purpose: PurposeName,
    asked_by: Correlation,
    answers: ReadinessAnswers,
}

#[derive(Debug, Default)]
struct BrokerScript {
    next_handle: u64,
    scripts_for_next_requests: VecDeque<ReadinessAnswers>,
    issued: BTreeMap<u64, IssuedRequest>,
    forgotten: u64,
}

/// A broker whose readiness answers are scripted per request: the first request gets the first
/// script, the second the second, and each handle answers only from its own script.
#[derive(Debug)]
pub struct ScriptedBroker {
    script: Mutex<BrokerScript>,
    maximum_remembered: usize,
}

impl Default for ScriptedBroker {
    fn default() -> Self {
        Self::remembering_newest(MAXIMUM_REMEMBERED_REQUESTS)
    }
}

impl ScriptedBroker {
    /// A broker that remembers at most `maximum_remembered` requests, forgetting the oldest first.
    pub fn remembering_newest(maximum_remembered: usize) -> Self {
        Self {
            script: Mutex::default(),
            maximum_remembered,
        }
    }

    /// Scripts the answers the next request that has no script yet will get, in the order they
    /// are asked for. A request with no script cannot be asked about.
    pub fn with_answers_for_next_request(
        mut self,
        answers: impl IntoIterator<Item = Result<Readiness, FailureCause>>,
    ) -> Self {
        self.script
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .scripts_for_next_requests
            .push_back(answers.into_iter().collect());
        self
    }

    /// The purposes of the remembered requests, oldest first.
    pub fn requests(&self) -> Vec<PurposeName> {
        locked(&self.script)
            .issued
            .values()
            .map(|request| request.purpose.clone())
            .collect()
    }

    /// The correlation each remembered request was made under, oldest first.
    pub fn request_correlations(&self) -> Vec<Correlation> {
        locked(&self.script)
            .issued
            .values()
            .map(|request| request.asked_by)
            .collect()
    }

    /// How many requests were forgotten to stay within the maximum.
    pub fn forgotten_requests(&self) -> u64 {
        locked(&self.script).forgotten
    }
}

impl Broker for ScriptedBroker {
    fn request(
        &self,
        correlation: Correlation,
        purpose: &PurposeName,
    ) -> Result<ResourceHandle, PortError> {
        let mut script = locked(&self.script);
        script.next_handle += 1;
        let handle = script.next_handle;
        let answers = script
            .scripts_for_next_requests
            .pop_front()
            .unwrap_or_default();
        script.issued.insert(
            handle,
            IssuedRequest {
                purpose: purpose.clone(),
                asked_by: correlation,
                answers,
            },
        );
        while script.issued.len() > self.maximum_remembered {
            script.issued.pop_first();
            script.forgotten += 1;
        }
        Ok(ResourceHandle::new(handle))
    }

    fn readiness(
        &self,
        _correlation: Correlation,
        handle: ResourceHandle,
    ) -> Result<Readiness, PortError> {
        let mut script = locked(&self.script);
        let request = script
            .issued
            .get_mut(&handle.get())
            .ok_or(PortError::new(PortName::Broker, FailureCause::Stale))?;
        match request.answers.pop_front() {
            Some(answer) => answer.map_err(|cause| PortError::new(PortName::Broker, cause)),
            None => unscripted(
                PortName::Broker,
                &format!("the readiness of handle {}", handle.get()),
            ),
        }
    }
}

/// An interpreter that knows exactly the lines a test gave it, answers a queue of attempts in
/// order, or fails every call. A queued attempt serves the first call and the retry alike.
#[derive(Debug, Default)]
pub struct ScriptedInterpreter {
    lines: Vec<(String, Interpretation)>,
    failure: Option<FailureCause>,
    attempts: Mutex<VecDeque<Result<Interpretation, FailureCause>>>,
    calls: Mutex<InterpreterCalls>,
}

/// How often the pipeline asked the scripted interpreter, and how often it was the retry.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InterpreterCalls {
    pub first_attempts: usize,
    pub retries: usize,
}

impl ScriptedInterpreter {
    pub fn with_line(mut self, line: &str, interpretation: Interpretation) -> Self {
        self.lines.push((line.to_owned(), interpretation));
        self
    }

    /// The answers of the next calls, one per call (the first attempt and then its retry), taking
    /// precedence over the lines.
    pub fn with_attempts(
        self,
        attempts: impl IntoIterator<Item = Result<Interpretation, FailureCause>>,
    ) -> Self {
        locked(&self.attempts).extend(attempts);
        self
    }

    /// An interpreter whose every call fails, as a model that is down does.
    pub fn failing(cause: FailureCause) -> Self {
        Self {
            failure: Some(cause),
            ..Self::default()
        }
    }

    pub fn calls(&self) -> InterpreterCalls {
        *locked(&self.calls)
    }

    fn answer(&self, text: &PlayerText) -> Result<Interpretation, PortError> {
        if let Some(cause) = self.failure {
            return Err(PortError::new(PortName::Interpreter, cause));
        }
        if let Some(attempt) = locked(&self.attempts).pop_front() {
            return attempt.map_err(|cause| PortError::new(PortName::Interpreter, cause));
        }
        match self.lines.iter().find(|(line, _)| line == text.expose()) {
            Some((_, interpretation)) => Ok(interpretation.clone()),
            None => unscripted(
                PortName::Interpreter,
                &format!("to interpret a line ({text:?})"),
            ),
        }
    }
}

impl Interpreter for ScriptedInterpreter {
    fn interpret(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        text: &PlayerText,
        _legal_choices: &[ActionId],
    ) -> Result<Interpretation, PortError> {
        locked(&self.calls).first_attempts += 1;
        self.answer(text)
    }

    fn retry_with_choices_only(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        text: &PlayerText,
        _legal_choices: &[ActionId],
    ) -> Result<Interpretation, PortError> {
        locked(&self.calls).retries += 1;
        self.answer(text)
    }
}

/// One question the pipeline asked the scripted rules owner, kept so a test can assert what was
/// asked and with how many dice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesCall {
    LegalChoices(SeatId),
    Prepare(ActionId),
    /// The rules owner was told a resolution that awaited answers will not be resumed.
    Abandon(ActionId),
    Resolve {
        action: ActionId,
        draws: usize,
    },
    Resume {
        action: ActionId,
        draws: usize,
        answers: Vec<(SeatId, ActionId)>,
        ruling: Option<RulingAnswer>,
    },
}

#[derive(Debug, Default)]
struct RulesScript {
    resolutions: BTreeMap<ActionId, VecDeque<Resolution>>,
    resumptions: BTreeMap<ActionId, VecDeque<Resolution>>,
    calls: Vec<RulesCall>,
    plan_checks: Vec<PlanCheck>,
}

/// One step of a plan checked against what the steps before it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCheck {
    pub action: ActionId,
    pub steps_before: usize,
    pub effects_before: usize,
}

/// A rules owner with a table of choices per seat and an answer per action. An action or seat the
/// table does not hold is a forgotten script line, not a decline: a test that wants
/// `not_supported_yet` scripts it. An action may also have a scripted sequence of resolutions and
/// of resumptions (one is used per call), a confirmation and a ruling; every question asked is
/// kept for `calls`.
#[derive(Debug, Default)]
pub struct ScriptedRulesOwner {
    choices: BTreeMap<SeatId, Vec<ActionId>>,
    actions: BTreeMap<ActionId, (Preparation, Resolution)>,
    confirmations: BTreeMap<ActionId, ConfirmationReason>,
    rulings: BTreeMap<ActionId, RulingRequest>,
    /// Actions that decline as a later step once their own name is among the earlier steps.
    once_per_plan: BTreeMap<ActionId, Decline>,
    failure: Option<FailureCause>,
    script: Mutex<RulesScript>,
}

impl ScriptedRulesOwner {
    pub fn with_choices(mut self, seat: SeatId, choices: Vec<ActionId>) -> Self {
        self.choices.insert(seat, choices);
        self
    }

    pub fn with_action(
        mut self,
        action: ActionId,
        preparation: Preparation,
        resolution: Resolution,
    ) -> Self {
        self.actions.insert(action, (preparation, resolution));
        self
    }

    /// An action the rules owner cannot run yet: both steps decline `not_supported_yet`.
    pub fn with_unsupported_action(self, action: ActionId) -> Self {
        let decline = Decline::with_default_reason(DeclineCode::NotSupportedYet);
        self.with_action(
            action,
            Preparation::Declined(decline.clone()),
            Resolution::Declined(decline),
        )
    }

    /// The successive answers of `resolve` for the action, one per call; the action's fixed
    /// resolution answers once they run out.
    pub fn with_resolutions(self, action: ActionId, resolutions: Vec<Resolution>) -> Self {
        self.locked_script()
            .resolutions
            .insert(action, resolutions.into());
        self
    }

    /// The successive answers of `resume` for the action, one per call; once they run out `resume`
    /// answers with the action's fixed resolution, like the trait's default.
    pub fn with_resumptions(self, action: ActionId, resumptions: Vec<Resolution>) -> Self {
        self.locked_script()
            .resumptions
            .insert(action, resumptions.into());
        self
    }

    pub fn with_confirmation(mut self, action: ActionId, reason: ConfirmationReason) -> Self {
        self.confirmations.insert(action, reason);
        self
    }

    pub fn with_ruling(mut self, action: ActionId, request: RulingRequest) -> Self {
        self.rulings.insert(action, request);
        self
    }

    /// The action spends something a plan has only once: checked after a step that is the same
    /// action, it declines with `decline` (the economy of C.2).
    pub fn with_once_per_plan(mut self, action: ActionId, decline: Decline) -> Self {
        self.once_per_plan.insert(action, decline);
        self
    }

    /// A rules owner whose every call fails, as one that is down does.
    pub fn failing(cause: FailureCause) -> Self {
        Self {
            failure: Some(cause),
            ..Self::default()
        }
    }

    /// Every step of a plan checked so far, oldest first, with what the steps before it did.
    pub fn plan_checks(&self) -> Vec<PlanCheck> {
        self.locked_script().plan_checks.clone()
    }

    /// Every question asked so far, oldest first.
    pub fn calls(&self) -> Vec<RulesCall> {
        self.locked_script().calls.clone()
    }

    fn locked_script(&self) -> MutexGuard<'_, RulesScript> {
        locked(&self.script)
    }

    fn scripted_action(&self, action: &ActionId) -> &(Preparation, Resolution) {
        match self.actions.get(action) {
            Some(scripted) => scripted,
            None => unscripted(
                PortName::RulesOwner,
                &format!("about the action {}", action.as_str()),
            ),
        }
    }

    fn fail_if_down(&self) -> Result<(), PortError> {
        match self.failure {
            Some(cause) => Err(PortError::new(PortName::RulesOwner, cause)),
            None => Ok(()),
        }
    }
}

impl RulesOwner for ScriptedRulesOwner {
    fn legal_choices(
        &self,
        _correlation: Correlation,
        seat: SeatId,
    ) -> Result<Vec<ActionId>, PortError> {
        self.fail_if_down()?;
        self.locked_script()
            .calls
            .push(RulesCall::LegalChoices(seat));
        match self.choices.get(&seat) {
            Some(choices) => Ok(choices.clone()),
            None => unscripted(
                PortName::RulesOwner,
                &format!("for the legal choices of seat {seat}"),
            ),
        }
    }

    fn prepare(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        action: &ActionId,
    ) -> Result<Preparation, PortError> {
        self.fail_if_down()?;
        self.locked_script()
            .calls
            .push(RulesCall::Prepare(action.clone()));
        Ok(self.scripted_action(action).0.clone())
    }

    fn prepare_in_plan(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        so_far: &PlanSoFar<'_>,
    ) -> Result<Preparation, PortError> {
        self.fail_if_down()?;
        self.locked_script().plan_checks.push(PlanCheck {
            action: action.clone(),
            steps_before: so_far.steps.len(),
            effects_before: so_far.effects.len(),
        });
        if let Some(decline) = self.once_per_plan.get(action)
            && so_far.steps.contains(action)
        {
            return Ok(Preparation::Declined(decline.clone()));
        }
        self.prepare(correlation, seat, action)
    }

    fn abandon(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        action: &ActionId,
        _pending: crate::envelope::PendingId,
    ) -> Result<(), PortError> {
        self.fail_if_down()?;
        self.locked_script()
            .calls
            .push(RulesCall::Abandon(action.clone()));
        Ok(())
    }

    fn resolve(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        action: &ActionId,
        draws: &[Draw],
    ) -> Result<Resolution, PortError> {
        self.fail_if_down()?;
        let mut script = self.locked_script();
        script.calls.push(RulesCall::Resolve {
            action: action.clone(),
            draws: draws.len(),
        });
        let scripted = script
            .resolutions
            .get_mut(action)
            .and_then(VecDeque::pop_front);
        drop(script);
        Ok(scripted.unwrap_or_else(|| self.scripted_action(action).1.clone()))
    }

    fn confirmation_needed(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        action: &ActionId,
    ) -> Result<Option<ConfirmationReason>, PortError> {
        self.fail_if_down()?;
        Ok(self.confirmations.get(action).copied())
    }

    fn ruling_needed(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        action: &ActionId,
    ) -> Result<Option<RulingRequest>, PortError> {
        self.fail_if_down()?;
        Ok(self.rulings.get(action).cloned())
    }

    fn resume(
        &self,
        _correlation: Correlation,
        _seat: SeatId,
        action: &ActionId,
        resumption: &Resumption<'_>,
    ) -> Result<Resolution, PortError> {
        self.fail_if_down()?;
        let mut script = self.locked_script();
        script.calls.push(RulesCall::Resume {
            action: action.clone(),
            draws: resumption.draws.len(),
            answers: resumption
                .answers
                .iter()
                .map(|answer| (answer.seat, answer.choice.clone()))
                .collect(),
            ruling: resumption.ruling.cloned(),
        });
        let scripted = script
            .resumptions
            .get_mut(action)
            .and_then(VecDeque::pop_front);
        drop(script);
        Ok(scripted.unwrap_or_else(|| self.scripted_action(action).1.clone()))
    }
}

// A narrator that speaks the lines a test queued, in order. Asking for more lines than were
/// queued is a forgotten script line.
#[derive(Debug, Default)]
pub struct ScriptedNarrator {
    lines: Mutex<VecDeque<String>>,
}

impl ScriptedNarrator {
    pub fn with_line(mut self, line: &str) -> Self {
        self.lines
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(line.to_owned());
        self
    }
}

impl Narrator for ScriptedNarrator {
    fn narrate(
        &self,
        _correlation: Correlation,
        _committed: &CommittedOutcome,
    ) -> Result<Narration, PortError> {
        match locked(&self.lines).pop_front() {
            Some(line) => Ok(Narration::new(line)),
            None => unscripted(PortName::Narrator, "to narrate beyond its queued lines"),
        }
    }
}

/// A narrator whose every call fails with one cause, to exercise the structured-readout fallback.
#[derive(Debug, Clone, Copy)]
pub struct FailingNarrator {
    pub cause: FailureCause,
}

impl Narrator for FailingNarrator {
    fn narrate(
        &self,
        _correlation: Correlation,
        _committed: &CommittedOutcome,
    ) -> Result<Narration, PortError> {
        Err(PortError::new(PortName::Narrator, self.cause))
    }
}

/// Dice that give the values a test queued, in order. Asking for more dice than were queued is a
/// forgotten script line, and a value that is not on the die asked for is `invalid_answer`; neither
/// is ever replaced by a roll.
#[derive(Debug, Default)]
pub struct QueuedDice {
    values: Mutex<VecDeque<u16>>,
}

impl QueuedDice {
    pub fn new(values: impl IntoIterator<Item = u16>) -> Self {
        Self {
            values: Mutex::new(values.into_iter().collect()),
        }
    }
}

impl Dice for QueuedDice {
    fn draw(&self, _correlation: Correlation, request: DrawRequest) -> Result<Draw, PortError> {
        let Some(value) = locked(&self.values).pop_front() else {
            unscripted(PortName::Dice, "for a die beyond its queued values")
        };
        Draw::new(request.sides, value)
            .ok_or(PortError::new(PortName::Dice, FailureCause::InvalidAnswer))
    }

    fn give_back(&self, _correlation: Correlation, draws: &[Draw]) {
        let mut values = locked(&self.values);
        for draw in draws.iter().rev() {
            values.push_front(draw.value());
        }
    }
}

/// A knowledge view over a fixed set of facts. A seat sees the public facts and its own; the public
/// audience sees only the public ones. Whatever else is asked is `None`, which is a real answer
/// ("the audience may not know it") and not a forgotten script line.
#[derive(Debug, Default)]
pub struct FixedKnowledgeView {
    facts: Vec<(Audience, FactId, String)>,
}

impl FixedKnowledgeView {
    pub fn with_fact(mut self, audience: Audience, fact: FactId, text: &str) -> Self {
        self.facts.push((audience, fact, text.to_owned()));
        self
    }

    fn find(&self, audience: Audience, fact: &FactId) -> Option<&str> {
        self.facts
            .iter()
            .find(|(held_for, held_fact, _)| *held_for == audience && held_fact == fact)
            .map(|(_, _, text)| text.as_str())
    }
}

impl KnowledgeView for FixedKnowledgeView {
    fn fact_for(
        &self,
        _correlation: Correlation,
        audience: Audience,
        fact: &FactId,
    ) -> Result<Option<Secret<String>>, PortError> {
        let own = match audience {
            Audience::Seat(_) => self.find(audience, fact),
            Audience::Public => None,
        };
        let known = own.or_else(|| self.find(Audience::Public, fact));
        Ok(known.map(|text| Secret::new(text.to_owned())))
    }
}
