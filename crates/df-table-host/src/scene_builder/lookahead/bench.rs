//! The workbench side of the look-ahead worker: the one scene the dev table expects next, the
//! scripted providers of the look-ahead purposes, and the verbs that change them.
//!
//! The dev table expects one scene at a time, the one the next `scene build` makes (its id is the
//! workbench's next scene id, so the worker's candidate and the table's scene are the same scene).
//! `target` states it as a certain or likely expectation, `clear` withdraws it, and the worker then
//! builds it on the look-ahead class while the clock runs (`plates run`, or a build's own steps).
//! A `slow=` or `fail=` on a target scripts the look-ahead provider calls of that stage, to see a
//! build that misses its transition or fails. It reads no clock: every call carries the workbench's
//! time.

use std::sync::Arc;

use df_table::identity::SceneId;
use df_table::lookahead::{
    CERTAIN_PERMILLE, Expectation, Expected, PlanView, Take, lookahead_candidates,
};
use df_table::lookahead_settings::LookaheadSettings;
use df_table::ports::PurposeName;
use df_table::scene_plan_brief::SceneBrief;
use df_table::table_state::TableSnapshot;

use super::request::{LookaheadRequest, LookaheadSettingName, is_built_ahead};
use super::worker::LookaheadWorker;
use crate::broker::BrokerHub;
use crate::broker::providers::ScriptedCall;
use crate::broker::request::ProviderFault;
use crate::scene_builder::dev_world::dev_brief;
use crate::scene_builder::events::{SceneBuilderAudit, SceneBuilderEvent};
use crate::scene_builder::fakes::FakeStageWorker;
use crate::scene_builder::inputs::StageInputs;
use crate::scene_builder::purposes::{BuildPriority, StagePurposes};
use crate::scene_builder::request::{DevStage, SceneError, SlowStage};

const MILLISECONDS_PER_SECOND: u64 = 1_000;

/// What a verb needs from the workbench for one call.
pub struct BenchContext<'a> {
    pub now_ms: u64,
    pub snapshot: &'a TableSnapshot,
    /// The scene on screen, if any.
    pub current: Option<SceneId>,
    /// The id the next `scene build` gives its scene.
    pub next_scene: u64,
    pub view: PlanView,
    pub providers: &'a mut FakeStageWorker,
}

/// The look-ahead worker with the dev table's expectation.
pub struct LookaheadBench {
    worker: LookaheadWorker,
    audit: Arc<dyn SceneBuilderAudit>,
}

fn workbench_error(reason: impl std::fmt::Display) -> SceneError {
    SceneError::Workbench {
        reason: reason.to_string(),
    }
}

fn bad_request(reason: impl std::fmt::Display) -> SceneError {
    SceneError::BadRequest {
        reason: reason.to_string(),
    }
}

fn purpose_of(stage: DevStage) -> Result<PurposeName, SceneError> {
    let purposes =
        StagePurposes::for_priority(BuildPriority::Lookahead).map_err(workbench_error)?;
    purposes
        .of(stage.scene_stage())
        .cloned()
        .ok_or_else(|| workbench_error("a dev stage has no purpose"))
}

impl LookaheadBench {
    pub fn new(
        hub: Arc<BrokerHub>,
        audit: Arc<dyn SceneBuilderAudit>,
        inputs: Arc<StageInputs>,
        settings: LookaheadSettings,
    ) -> Result<Self, SceneError> {
        Ok(Self {
            worker: LookaheadWorker::new(hub, Arc::clone(&audit), inputs, settings)
                .map_err(workbench_error)?,
            audit,
        })
    }

    pub fn worker(&self) -> &LookaheadWorker {
        &self.worker
    }

    pub fn advance(&mut self, now_ms: u64, snapshot: &TableSnapshot, view: PlanView) {
        self.worker.advance(now_ms, snapshot, view);
    }

    /// The table is entering `scene`: what the look-ahead found.
    pub fn enter(
        &mut self,
        now_ms: u64,
        scene: SceneId,
        snapshot: &TableSnapshot,
        brief: &SceneBrief,
    ) -> Take {
        self.worker.enter(now_ms, scene, snapshot, brief)
    }

    /// Carries out one verb. A target is validated before anything is scripted, so a refused one
    /// leaves the providers as they were.
    pub fn handle(
        &mut self,
        request: &LookaheadRequest,
        mut context: BenchContext<'_>,
    ) -> Result<(), SceneError> {
        match request {
            LookaheadRequest::Pause => self.worker.pause(),
            LookaheadRequest::Resume => self.worker.resume(),
            LookaheadRequest::Clear => self
                .worker
                .set_expected(
                    context.now_ms,
                    context.snapshot,
                    context.current,
                    Vec::new(),
                )
                .map_err(workbench_error)?,
            LookaheadRequest::Set { setting, value } => self.set(*setting, *value)?,
            LookaheadRequest::Target {
                place,
                kind,
                likelihood_permille,
                fail,
                slow,
            } => self.target(
                &mut context,
                place,
                *kind,
                *likelihood_permille,
                *fail,
                slow.as_ref(),
            )?,
        }
        self.worker
            .advance(context.now_ms, context.snapshot, context.view);
        Ok(())
    }

    /// Changes one setting, or refuses a value outside its range naming the setting.
    fn set(&mut self, setting: LookaheadSettingName, value: u64) -> Result<(), SceneError> {
        let settings = self.worker.settings();
        let changed = match setting.numeric() {
            Some(numeric) => settings.with(numeric, value).map_err(bad_request)?,
            None => match value {
                0 => settings.with_enabled(false),
                1 => settings.with_enabled(true),
                _ => {
                    return Err(bad_request(format!(
                        "lookahead_enabled is 0 or 1, not {value}"
                    )));
                }
            },
        };
        self.worker.set_settings(changed);
        Ok(())
    }

    fn target(
        &mut self,
        context: &mut BenchContext<'_>,
        place: &str,
        kind: Expectation,
        likelihood_permille: Option<u16>,
        fail: Option<DevStage>,
        slow: Option<&SlowStage>,
    ) -> Result<(), SceneError> {
        for stage in fail.iter().chain(slow.map(|slow| &slow.stage)) {
            if !is_built_ahead(*stage) {
                return Err(bad_request("only describe and background are built ahead"));
            }
        }
        let permille = match (kind.is_certain(), likelihood_permille) {
            (true, None) => CERTAIN_PERMILLE,
            (true, Some(_)) => {
                return Err(bad_request(format!(
                    "a {} scene is certain and takes no likelihood",
                    kind.key()
                )));
            }
            (false, Some(permille)) => permille,
            (false, None) => {
                return Err(bad_request(format!(
                    "a {} scene needs a likelihood in thousandths",
                    kind.key()
                )));
            }
        };
        let scene = SceneId::new(context.next_scene);
        let expected = Expected {
            brief: dev_brief(scene, place).map_err(workbench_error)?,
            expectation: kind,
            likelihood_permille: permille,
        };
        lookahead_candidates(context.snapshot, context.current, vec![expected.clone()])
            .map_err(workbench_error)?;
        let slow_purpose = slow.map(|slow| purpose_of(slow.stage)).transpose()?;
        let fail_purpose = fail.map(purpose_of).transpose()?;
        if let (Some(slow), Some(purpose)) = (slow, &slow_purpose) {
            let latency_ms = slow.seconds.saturating_mul(MILLISECONDS_PER_SECOND);
            context
                .providers
                .script_mut()
                .set_latency(purpose, latency_ms);
            self.audit.record(&SceneBuilderEvent::LookaheadSlowed {
                stage: slow.stage.scene_stage(),
                latency_ms,
            });
        }
        if let (Some(stage), Some(purpose)) = (fail, &fail_purpose) {
            context.providers.script_mut().push_calls(
                purpose,
                [ScriptedCall::fails_after(0, ProviderFault::Refused)],
            );
            self.audit.record(&SceneBuilderEvent::LookaheadRefusing {
                stage: stage.scene_stage(),
            });
        }
        self.worker
            .set_expected(
                context.now_ms,
                context.snapshot,
                context.current,
                vec![expected],
            )
            .map_err(workbench_error)
    }
}
