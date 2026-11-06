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

use df_table::element::NameError;
use df_table::identity::SceneId;
use df_table::lookahead::settings::LookaheadSettings;
use df_table::lookahead::{
    CERTAIN_PERMILLE, Expectation, Expected, PlanView, Take, lookahead_candidates,
};
use df_table::scene_plan::brief::SceneBrief;
use df_table::table_state::{DisplayName, TableSnapshot};
use df_table::units::MILLISECONDS_PER_SECOND;

use super::request::{LookaheadRequest, LookaheadSettingName, is_built_ahead};
use super::worker::LookaheadWorker;
use crate::broker::BrokerHub;
use crate::broker::providers::ScriptedCall;
use crate::broker::request::ProviderFault;
use crate::scene_builder::dev_world::dev_brief;
use crate::scene_builder::events::{SceneBuilderAudit, SceneBuilderEvent};
use crate::scene_builder::fakes::FakeStageWorker;
use crate::scene_builder::inputs::StageInputs;
use crate::scene_builder::purposes::{BuildPriority, dev_stage_purpose};
use crate::scene_builder::request::{DevStage, SceneBuilderError, SlowStage};
use crate::scene_builder::workbench::WorkbenchError;

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

/// The likelihood of a target of `kind`: certain for a certain kind, which takes none, and the
/// stated thousandths, 0 to 1,000, for a likely kind, which needs one.
fn target_likelihood(
    kind: Expectation,
    likelihood_permille: Option<u16>,
) -> Result<u16, SceneBuilderError> {
    let invalid = SceneBuilderError::InvalidLikelihood {
        kind,
        likelihood_permille,
    };
    match (kind.is_certain(), likelihood_permille) {
        (true, None) => Ok(CERTAIN_PERMILLE),
        (false, Some(permille)) if permille <= CERTAIN_PERMILLE => Ok(permille),
        (true, Some(_)) | (false, _) => Err(invalid),
    }
}

impl LookaheadBench {
    pub fn new(
        hub: Arc<BrokerHub>,
        audit: Arc<dyn SceneBuilderAudit>,
        inputs: Arc<StageInputs>,
        settings: LookaheadSettings,
    ) -> Result<Self, NameError> {
        Ok(Self {
            worker: LookaheadWorker::new(hub, Arc::clone(&audit), inputs, settings)?,
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
    ) -> Result<(), SceneBuilderError> {
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
                .map_err(WorkbenchError::failed)?,
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
    fn set(&mut self, setting: LookaheadSettingName, value: u64) -> Result<(), SceneBuilderError> {
        let invalid = SceneBuilderError::InvalidSetting { setting, value };
        let settings = self.worker.settings();
        let changed = match (setting.numeric(), value) {
            // The refusal names the same setting and value, so it says what the planner's would.
            (Some(numeric), _) => settings.with(numeric, value).map_err(|_| invalid)?,
            (None, 0) => settings.with_enabled(false),
            (None, 1) => settings.with_enabled(true),
            (None, _) => return Err(invalid),
        };
        self.worker.set_settings(changed);
        Ok(())
    }

    /// States the scene the next `scene build` makes as the one the story expects, after checking
    /// every value, and scripts its slow or failing stage.
    fn target(
        &mut self,
        context: &mut BenchContext<'_>,
        place: &DisplayName,
        kind: Expectation,
        likelihood_permille: Option<u16>,
        fail: Option<DevStage>,
        slow: Option<&SlowStage>,
    ) -> Result<(), SceneBuilderError> {
        if let Some(stage) = fail
            .into_iter()
            .chain(slow.map(|slow| slow.stage))
            .find(|stage| !is_built_ahead(*stage))
        {
            return Err(SceneBuilderError::InvalidLookaheadStage { stage });
        }
        let scene = SceneId::new(context.next_scene);
        let expected = Expected {
            brief: dev_brief(scene, place.as_str()).map_err(WorkbenchError::failed)?,
            expectation: kind,
            likelihood_permille: target_likelihood(kind, likelihood_permille)?,
        };
        lookahead_candidates(context.snapshot, context.current, vec![expected.clone()])
            .map_err(WorkbenchError::failed)?;
        let purpose_of = |stage| dev_stage_purpose(BuildPriority::Lookahead, stage);
        let slow_purpose = slow
            .map(|slow| purpose_of(slow.stage))
            .transpose()
            .map_err(WorkbenchError::failed)?;
        let fail_purpose = fail
            .map(purpose_of)
            .transpose()
            .map_err(WorkbenchError::failed)?;
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
            .map_err(WorkbenchError::failed)?;
        Ok(())
    }
}
