//! A fake provider worker for the scene builder's stages, with scripted latency, failures and
//! answers, on a clock it is given.
//!
//! The broker's `FakeWorker` answers every text call with filler and every picture call with a
//! blank picture, which is enough for the broker's own tests but cannot say what a segmentation
//! request found. This worker serves the same hub calls (`claim`, `mark_sent`, `complete`,
//! `tick`) and reads the input of each call it claimed from the book the builder wrote
//! (`StageInputs`), so a Locate call is answered for the prompt it was asked: the people, or the
//! phrase. Latency and success are scripted per purpose in call order, with a default latency for
//! each purpose and success when the script has run out; a call can fail, hang (a deadline then
//! fails it), or answer past its size rule.
//!
//! It never sleeps and never spawns: a call is scheduled for `now + latency` and delivered when a
//! pump is given a later time, so a test and the dev workbench drive it with the clock they own. It
//! makes no network call and spends nothing.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use df_table::ports::PurposeName;
use df_table::private_data::Narration;
use df_table::scene_plan::SceneStage;
use df_table::scene_plan_locate::{Instance, LocatePrompt};

use super::answers::encode_locate_answer;
use super::inputs::StageInputs;
use crate::broker::BrokerHub;
use crate::broker::providers::{FakeOutcome, ScriptedCall};
use crate::broker::purpose::{ResourceKind, SizeRule};
use crate::broker::request::{AttemptId, ProviderFault, ResourceResult, WorkerId};
use crate::broker::scheduler::ClaimedCall;

/// How long a fake text call takes unless the script says otherwise (the dev engine's figure).
pub const DEFAULT_TEXT_LATENCY_MS: u64 = 400;
/// How long a fake picture call takes unless the script says otherwise (the dev engine's figure).
pub const DEFAULT_IMAGE_LATENCY_MS: u64 = 2_000;
/// How long a fake segmentation call takes: the median of the S2 run, 1.48 s.
pub const DEFAULT_LOCATE_LATENCY_MS: u64 = 1_480;
/// The size of the picture a fake makes, inside every picture purpose's size rule.
pub const FAKE_PICTURE_WIDTH: u32 = 1_024;
pub const FAKE_PICTURE_HEIGHT: u32 = 576;
/// Most started calls remembered for `started`; the oldest are forgotten first. Deliberately huge.
const MAXIMUM_REMEMBERED_CALLS: usize = 1_000_000;

/// What the fake does: how long each purpose takes, what it answers and what it fails.
#[derive(Debug, Clone)]
pub struct StageScript {
    latency_ms: BTreeMap<PurposeName, u64>,
    calls: BTreeMap<PurposeName, VecDeque<ScriptedCall>>,
    people: Vec<Instance>,
    phrases: BTreeMap<String, Vec<Instance>>,
    description: String,
}

impl Default for StageScript {
    fn default() -> Self {
        Self {
            latency_ms: BTreeMap::new(),
            calls: BTreeMap::new(),
            people: Vec::new(),
            phrases: BTreeMap::new(),
            description: "A quiet place, much as it always is.".to_owned(),
        }
    }
}

impl StageScript {
    /// How long every call of `purpose` takes from now on, when the script has no call for it.
    pub fn with_latency(mut self, purpose: &PurposeName, latency_ms: u64) -> Self {
        self.latency_ms.insert(purpose.clone(), latency_ms);
        self
    }

    /// The next calls of `purpose`, in order: how long each takes and what it does.
    pub fn with_calls(
        mut self,
        purpose: &PurposeName,
        calls: impl IntoIterator<Item = ScriptedCall>,
    ) -> Self {
        self.calls.entry(purpose.clone()).or_default().extend(calls);
        self
    }

    /// What the segmentation model finds when asked for the people.
    pub fn with_people(mut self, instances: Vec<Instance>) -> Self {
        self.people = instances;
        self
    }

    /// What it finds when asked for `phrase`; a phrase not listed finds nothing.
    pub fn with_phrase(mut self, phrase: &str, instances: Vec<Instance>) -> Self {
        self.phrases.insert(phrase.to_owned(), instances);
        self
    }

    fn latency_of(
        &self,
        purpose: &PurposeName,
        kind: ResourceKind,
        stage: Option<SceneStage>,
    ) -> u64 {
        self.latency_ms
            .get(purpose)
            .copied()
            .unwrap_or(match (kind, stage) {
                (_, Some(SceneStage::Locate)) => DEFAULT_LOCATE_LATENCY_MS,
                (ResourceKind::Text, _) => DEFAULT_TEXT_LATENCY_MS,
                (ResourceKind::Image, _) => DEFAULT_IMAGE_LATENCY_MS,
            })
    }

    fn next_call(
        &mut self,
        purpose: &PurposeName,
        kind: ResourceKind,
        stage: Option<SceneStage>,
    ) -> ScriptedCall {
        let latency = self.latency_of(purpose, kind, stage);
        self.calls
            .get_mut(purpose)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| ScriptedCall::succeeds_after(latency))
    }

    fn instances_for(&self, prompt: Option<&LocatePrompt>) -> &[Instance] {
        match prompt {
            Some(LocatePrompt::People) => &self.people,
            Some(LocatePrompt::Item { phrase, .. }) => self
                .phrases
                .get(phrase.as_str())
                .map_or(&[][..], Vec::as_slice),
            None => &[],
        }
    }
}

/// A call the fake was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedCall {
    pub purpose: PurposeName,
    pub stage: Option<SceneStage>,
    pub prompt: Option<LocatePrompt>,
    pub started_ms: u64,
}

struct Scheduled {
    attempt: AttemptId,
    ready_at_ms: u64,
    answer: Result<ResourceResult, ProviderFault>,
}

/// What one pump did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PumpReport {
    pub started: usize,
    pub delivered: usize,
}

/// The stand-in for the worker that owns the real provider tasks of the scene builder's stages.
pub struct FakeStageWorker {
    hub: Arc<BrokerHub>,
    inputs: Arc<StageInputs>,
    worker: WorkerId,
    script: StageScript,
    scheduled: Vec<Scheduled>,
    started: VecDeque<StartedCall>,
    started_count: u64,
}

impl FakeStageWorker {
    pub fn new(
        hub: Arc<BrokerHub>,
        inputs: Arc<StageInputs>,
        worker: WorkerId,
        script: StageScript,
    ) -> Self {
        Self {
            hub,
            inputs,
            worker,
            script,
            scheduled: Vec::new(),
            started: VecDeque::new(),
            started_count: 0,
        }
    }

    /// Replaces the script: the calls it had not served yet are forgotten.
    pub fn set_script(&mut self, script: StageScript) {
        self.script = script;
    }

    /// How many calls the fake was given.
    pub fn calls_started(&self) -> u64 {
        self.started_count
    }

    /// The calls it was given, oldest first.
    pub fn started(&self) -> impl Iterator<Item = &StartedCall> {
        self.started.iter()
    }

    /// One round of work at `now_ms`, the hub's time: cancel what was abandoned, start what waits,
    /// deliver what is due, and let the hub enforce its deadlines and leases.
    pub fn pump(&mut self, now_ms: u64) -> PumpReport {
        let mut report = PumpReport::default();
        self.hub.tick();
        for attempt in self.hub.take_cancellations() {
            self.scheduled.retain(|call| call.attempt != attempt);
        }
        // A real worker renews the lease of every call it has running; a picture takes longer than
        // the 60 s lease, so a fake that did not would fail what it was asked to be slow at.
        for call in &self.scheduled {
            self.hub.heartbeat(call.attempt);
        }
        for kind in ResourceKind::ALL {
            while let Some(call) = self.hub.claim(*kind, self.worker) {
                if self.hub.mark_sent(call.attempt).is_ok() {
                    self.start(&call, now_ms);
                    report.started += 1;
                }
            }
        }
        let (due, waiting): (Vec<Scheduled>, Vec<Scheduled>) = self
            .scheduled
            .drain(..)
            .partition(|call| call.ready_at_ms <= now_ms);
        self.scheduled = waiting;
        let mut due = due;
        due.sort_by_key(|call| (call.ready_at_ms, call.attempt));
        for call in due {
            self.hub.complete(call.attempt, call.answer);
            report.delivered += 1;
        }
        self.hub.tick();
        report
    }

    fn start(&mut self, call: &ClaimedCall, now_ms: u64) {
        let input = self.inputs.get(call.key);
        let stage = input.as_ref().map(|input| input.stage);
        let prompt = input.and_then(|input| input.prompt);
        self.started_count += 1;
        self.started.push_back(StartedCall {
            purpose: call.purpose.clone(),
            stage,
            prompt: prompt.clone(),
            started_ms: now_ms,
        });
        while self.started.len() > MAXIMUM_REMEMBERED_CALLS {
            self.started.pop_front();
        }
        let scripted = self.script.next_call(&call.purpose, call.kind, stage);
        let answer = match scripted.outcome {
            FakeOutcome::Hangs => return,
            FakeOutcome::Fails(fault) => Err(fault),
            FakeOutcome::Succeeds => Ok(self.answer(call.size, stage, prompt.as_ref())),
            FakeOutcome::ReturnsOversize => Ok(oversize(call.size)),
        };
        self.scheduled.push(Scheduled {
            attempt: call.attempt,
            ready_at_ms: now_ms.saturating_add(scripted.latency_ms),
            answer,
        });
    }

    fn answer(
        &self,
        size: SizeRule,
        stage: Option<SceneStage>,
        prompt: Option<&LocatePrompt>,
    ) -> ResourceResult {
        match size {
            SizeRule::ImagePixels { .. } => ResourceResult::Image {
                width: FAKE_PICTURE_WIDTH,
                height: FAKE_PICTURE_HEIGHT,
                bytes: u64::from(FAKE_PICTURE_WIDTH) * u64::from(FAKE_PICTURE_HEIGHT),
            },
            SizeRule::TextCharacters { .. } if stage == Some(SceneStage::Locate) => {
                let text = encode_locate_answer(self.script.instances_for(prompt))
                    .unwrap_or_else(|_| Narration::new("{}"));
                ResourceResult::Text(text)
            }
            SizeRule::TextCharacters { .. } => {
                ResourceResult::Text(Narration::new(self.script.description.clone()))
            }
        }
    }
}

/// An answer one character or pixel past the rule.
fn oversize(size: SizeRule) -> ResourceResult {
    match size {
        SizeRule::TextCharacters { maximum } => {
            let length = usize::try_from(maximum).map_or(usize::MAX, |maximum| maximum + 1);
            ResourceResult::Text(Narration::new("a".repeat(length)))
        }
        SizeRule::ImagePixels { width, height } => ResourceResult::Image {
            width: width.saturating_add(1),
            height: height.saturating_add(1),
            bytes: u64::from(width.saturating_add(1)) * u64::from(height.saturating_add(1)),
        },
    }
}
