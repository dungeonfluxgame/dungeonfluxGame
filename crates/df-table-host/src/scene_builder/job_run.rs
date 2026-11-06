//! One plate job's run through the stages: the part of the scene builder that talks to the broker.
//!
//! `Driver::drive` moves a `JobRun` as far as it can without waiting. It asks the stage plan what is
//! next, makes that stage's broker request (one request for Describe, Background and Plate; one
//! per prompt for Locate, in rounds), and stores the output when the request is ready, so the next
//! stage starts only from a stored output. It never waits for a provider: a request that is not
//! settled leaves the run open and `drive` returns, and the next call looks again.
//!
//! Every step first asks the table whether the job is still current (D.2): a job of a scene that
//! has moved on cancels its open requests, stores nothing and ends `Stale`, and a paused table
//! holds the run where it is. Every failure becomes a typed `FailedStage` with the fallback the
//! stage's purpose names, and ends the job; Locate's failures do not end the job, because a plate
//! with every figure unlocated at its fallback point is still a plate (pattern 11). It makes no
//! request without recording its input first, and records each change once, as a
//! `SceneBuilderEvent`.

use df_table::event::FailureCause;
use df_table::identity::TableScope;
use df_table::plate::scheduler::JobEnd;
use df_table::ports::{PurposeName, ResourceHandle};
use df_table::scene_plan::locate::{
    FigureTarget, Instance, ItemTarget, LocateOutcome, LocatePlan, LocatePrompt,
};
use df_table::scene_plan::{
    FailedStage, PlanError, SceneStage, StageOutput, StagePlan, StoredPicture,
};
use df_table::table_state::{StaleCause, StaleResultError, TableSnapshot};

use super::answers::parse_locate_answer;
use super::builder::{BuilderOutput, JobOutcome, SceneRecord};
use super::events::{LocateCounts, SceneBuilderAudit, SceneBuilderEvent, StageSource};
use super::inputs::{StageInput, StageInputs};
use crate::broker::BrokerHub;
use crate::broker::purpose::{Fallback, PurposeTable};
use crate::broker::request::{
    Epoch, NewRequest, RequestError, RequestStatus, ResourceResult, ResultError,
};

/// The requests a stage has out.
enum Open {
    Single {
        stage: SceneStage,
        handle: ResourceHandle,
    },
    Locate(LocateRun),
}

/// Locate's rounds: the plan and the requests of the round in flight.
struct LocateRun {
    plan: LocatePlan,
    round: u32,
    requests: Vec<(ResourceHandle, LocatePrompt)>,
}

/// One job, from its plan to the requests it has out.
pub(super) struct JobRun {
    plan: StagePlan,
    open: Option<Open>,
    stage_started_ms: u64,
}

impl JobRun {
    pub(super) fn new(plan: StagePlan) -> Self {
        Self {
            plan,
            open: None,
            stage_started_ms: 0,
        }
    }

    pub(super) fn plan(&self) -> &StagePlan {
        &self.plan
    }
}

/// What a drive left the run as.
pub(super) enum Driven {
    Alive(JobRun),
    Ended(BuilderOutput),
}

/// What polling an open stage found.
enum Polled {
    Waiting(Open),
    /// The stage was stored, so the next one may start.
    Stored,
    Ended(BuilderOutput),
}

/// What starting a stage did.
enum Started {
    Open(Open),
    /// The stage needed no request and was stored at once.
    Stored,
    Ended(BuilderOutput),
}

/// The broker, the audit and the table, for one pass over the jobs of a scene.
pub(super) struct Driver<'a> {
    pub hub: &'a BrokerHub,
    pub audit: &'a dyn SceneBuilderAudit,
    pub inputs: &'a StageInputs,
    pub table: &'a PurposeTable,
    pub scope: &'a TableScope,
    pub now_ms: u64,
    pub snapshot: &'a TableSnapshot,
}

fn counts_of(outcome: &LocateOutcome) -> LocateCounts {
    LocateCounts {
        located_figures: outcome.located_figures(),
        unlocated_figures: outcome.unlocated_figures(),
        located_items: outcome.located_items(),
        own_picture_items: outcome.own_picture_items(),
        unlocated_items: outcome.unlocated_items(),
    }
}

impl Driver<'_> {
    fn record(&self, event: SceneBuilderEvent) {
        self.audit.record(&event);
    }

    /// Moves `run` as far as it goes without waiting.
    pub(super) fn drive(&self, record: &mut SceneRecord, mut run: JobRun) -> Driven {
        if let Err(stale) = self.snapshot.judge_result(&run.plan.job().stamp()) {
            if stale.cause == StaleCause::TablePaused {
                return Driven::Alive(run);
            }
            return Driven::Ended(self.end_stale(&run, stale));
        }
        loop {
            if let Some(open) = run.open.take() {
                match self.poll(record, &mut run, open) {
                    Polled::Waiting(open) => {
                        run.open = Some(open);
                        return Driven::Alive(run);
                    }
                    Polled::Stored => {}
                    Polled::Ended(output) => return Driven::Ended(output),
                }
                continue;
            }
            let Some(stage) = run.plan.next_stage() else {
                return Driven::Alive(run);
            };
            if stage == SceneStage::Ready {
                return self.finish(record, run);
            }
            match self.start(record, &mut run, stage) {
                Started::Open(open) => run.open = Some(open),
                Started::Stored => {}
                Started::Ended(output) => return Driven::Ended(output),
            }
        }
    }

    /// Ends a job that is no longer current: cancels what it has out, records why, and stores
    /// nothing (D.2).
    pub(super) fn end_stale(&self, run: &JobRun, stale: StaleResultError) -> BuilderOutput {
        cancel_open(self.hub, run);
        let job = run.plan.job();
        let stage = run.plan.running().unwrap_or(SceneStage::Describe);
        self.record(SceneBuilderEvent::Table(
            stale.event(self.scope, job.correlation(stage, 0)),
        ));
        self.ended(run, JobEnd::Stale, None)
    }

    fn ended(&self, run: &JobRun, end: JobEnd, failure: Option<FailedStage>) -> BuilderOutput {
        let job = run.plan.job();
        self.record(SceneBuilderEvent::JobEnded {
            scene: job.scene(),
            generation: job.stamp().generation,
            job: job.id(),
            end,
            duration_ms: self.now_ms.saturating_sub(job.tick_ms()),
        });
        BuilderOutput::JobEnded(JobOutcome {
            job: job.id(),
            scene: job.scene(),
            end,
            failure,
        })
    }

    fn fallback_of(&self, purpose: Option<&PurposeName>) -> Fallback {
        purpose
            .and_then(|purpose| self.table.get(purpose))
            .map_or(Fallback::Nothing, |spec| spec.fallback())
    }

    /// Ends the job with the running stage failed for `cause`.
    fn fail(
        &self,
        record: &SceneRecord,
        run: &mut JobRun,
        stage: SceneStage,
        cause: FailureCause,
    ) -> BuilderOutput {
        let purpose = record.purposes.of(stage);
        let fallback = self.fallback_of(purpose);
        let failure = run.plan.fail(stage, cause).ok();
        let job = run.plan.job();
        self.record(SceneBuilderEvent::StageFailed {
            scene: job.scene(),
            generation: job.stamp().generation,
            job: job.id(),
            stage,
            cause,
            fallback,
            duration_ms: self.now_ms.saturating_sub(run.stage_started_ms),
        });
        self.ended(run, JobEnd::Failed, failure)
    }

    fn input_for(&self, run: &JobRun, stage: SceneStage) -> StageInput {
        let job = run.plan.job();
        let outputs = run.plan.outputs();
        StageInput {
            scene: job.scene(),
            generation: job.stamp().generation,
            job: job.id(),
            stage,
            staging_digest: job.staging_digest(),
            source: match stage {
                SceneStage::Plate => outputs.background,
                SceneStage::Locate => outputs.plate,
                SceneStage::Describe | SceneStage::Background | SceneStage::Ready => None,
            },
            prompt: None,
        }
    }

    /// Makes one broker request for `input` under `purpose`, recording the input first.
    fn request(
        &self,
        run: &JobRun,
        purpose: &PurposeName,
        input: StageInput,
        discriminant: u32,
    ) -> Result<ResourceHandle, RequestError> {
        let job = run.plan.job();
        let key = input.key(purpose);
        let stage = input.stage;
        self.inputs.record(key, input);
        self.hub.request_keyed(NewRequest {
            purpose: purpose.clone(),
            key,
            epoch: Epoch::new(job.stamp().generation.get()),
            correlation: job.correlation(stage, discriminant),
        })
    }

    fn start(&self, record: &SceneRecord, run: &mut JobRun, stage: SceneStage) -> Started {
        // Set before `begin`, so a stage the plan refuses to begin fails after no time.
        run.stage_started_ms = self.now_ms;
        match run.plan.begin(self.snapshot, stage) {
            Ok(()) => {}
            Err(PlanError::Stale(stale)) => return Started::Ended(self.end_stale(run, stale)),
            Err(_) => {
                return Started::Ended(self.fail(record, run, stage, FailureCause::InvalidAnswer));
            }
        }
        if stage == SceneStage::Locate {
            return self.start_locate(record, run);
        }
        let Some(purpose) = record.purposes.of(stage) else {
            return Started::Ended(self.fail(record, run, stage, FailureCause::InvalidAnswer));
        };
        let input = self.input_for(run, stage);
        match self.request(run, purpose, input, 0) {
            Ok(handle) => {
                let job = run.plan.job();
                self.record(SceneBuilderEvent::StageRequested {
                    scene: job.scene(),
                    generation: job.stamp().generation,
                    job: job.id(),
                    stage,
                    purpose: purpose.clone(),
                    requests: 1,
                });
                Started::Open(Open::Single { stage, handle })
            }
            Err(error) => Started::Ended(self.refused(record, run, stage, &error)),
        }
    }

    fn refused(
        &self,
        record: &SceneRecord,
        run: &mut JobRun,
        stage: SceneStage,
        error: &RequestError,
    ) -> BuilderOutput {
        match error {
            RequestError::StaleEpoch { .. } => {
                let _ = run.plan.fail(stage, FailureCause::Stale);
                self.ended(run, JobEnd::Stale, None)
            }
            RequestError::UnknownPurpose { .. }
            | RequestError::QueueFull { .. }
            | RequestError::Closed => self.fail(record, run, stage, FailureCause::Unavailable),
        }
    }

    fn start_locate(&self, record: &SceneRecord, run: &mut JobRun) -> Started {
        let job = run.plan.job();
        let figures: Vec<FigureTarget> = job
            .figures()
            .iter()
            .map(|figure| FigureTarget {
                id: figure.id.clone(),
                stated: figure.stated,
            })
            .collect();
        let items: Vec<ItemTarget> = job
            .set()
            .iter()
            .filter(|piece| piece.is_item())
            .map(|piece| ItemTarget {
                id: piece.id().clone(),
                stated: piece.position(),
                phrases: piece.phrases().to_vec(),
                picture: piece.picture().cloned(),
            })
            .collect();
        let Ok(plan) = LocatePlan::new(figures, items) else {
            return Started::Ended(self.fail(
                record,
                run,
                SceneStage::Locate,
                FailureCause::InvalidAnswer,
            ));
        };
        let mut locate = LocateRun {
            plan,
            round: 0,
            requests: Vec::new(),
        };
        match self.issue_round(record, run, &mut locate) {
            Some(()) => Started::Open(Open::Locate(locate)),
            None => self.store_locate(record, run, locate.plan, StageSource::NothingToAsk),
        }
    }

    /// Sends the next round's prompts, each as one request. `None` when there is nothing left to
    /// ask.
    fn issue_round(
        &self,
        record: &SceneRecord,
        run: &JobRun,
        locate: &mut LocateRun,
    ) -> Option<()> {
        let prompts = locate.plan.next_round();
        if prompts.is_empty() {
            return None;
        }
        let job = run.plan.job();
        locate.round += 1;
        let purpose = &record.purposes.locate;
        self.record(if locate.round == 1 {
            SceneBuilderEvent::StageRequested {
                scene: job.scene(),
                generation: job.stamp().generation,
                job: job.id(),
                stage: SceneStage::Locate,
                purpose: purpose.clone(),
                requests: u32::try_from(prompts.len()).unwrap_or(u32::MAX),
            }
        } else {
            SceneBuilderEvent::LocateRound {
                scene: job.scene(),
                generation: job.stamp().generation,
                job: job.id(),
                round: locate.round,
                prompts: u32::try_from(prompts.len()).unwrap_or(u32::MAX),
            }
        });
        for (index, prompt) in prompts.into_iter().enumerate() {
            let mut input = self.input_for(run, SceneStage::Locate);
            input.prompt = Some(prompt.clone());
            let discriminant = locate
                .round
                .saturating_mul(1_000)
                .saturating_add(u32::try_from(index).unwrap_or(u32::MAX));
            match self.request(run, purpose, input, discriminant) {
                Ok(handle) => locate.requests.push((handle, prompt)),
                Err(error) => {
                    let cause = match error {
                        RequestError::StaleEpoch { .. } => FailureCause::Stale,
                        _ => FailureCause::Unavailable,
                    };
                    // A refused request is recorded as the target's answer, once, so the plan
                    // falls back for it and the round does not wait for a handle it never got.
                    let _ = locate.plan.record(&prompt, Err(cause));
                }
            }
        }
        Some(())
    }

    fn poll(&self, record: &mut SceneRecord, run: &mut JobRun, open: Open) -> Polled {
        match open {
            Open::Single { stage, handle } => self.poll_single(record, run, stage, handle),
            Open::Locate(locate) => self.poll_locate(record, run, locate),
        }
    }

    fn poll_single(
        &self,
        record: &mut SceneRecord,
        run: &mut JobRun,
        stage: SceneStage,
        handle: ResourceHandle,
    ) -> Polled {
        let Some(status) = self.hub.status(handle) else {
            return Polled::Ended(self.end_unwanted(run, JobEnd::Stale));
        };
        match status {
            RequestStatus::Queued { .. }
            | RequestStatus::BackingOff { .. }
            | RequestStatus::InFlight { .. } => Polled::Waiting(Open::Single { stage, handle }),
            RequestStatus::Ready => self.store_single(record, run, stage, handle),
            RequestStatus::Failed { cause } => Polled::Ended(self.fail(record, run, stage, cause)),
            RequestStatus::Cancelled | RequestStatus::Superseded => {
                Polled::Ended(self.end_unwanted(run, JobEnd::Stale))
            }
            RequestStatus::Dropped => Polled::Ended(self.end_unwanted(run, JobEnd::Dropped)),
        }
    }

    /// Ends a job whose request was cancelled, superseded or dropped: its result is not wanted,
    /// and what it has out is cancelled.
    fn end_unwanted(&self, run: &mut JobRun, end: JobEnd) -> BuilderOutput {
        cancel_open(self.hub, run);
        if let Some(stage) = run.plan.running() {
            let _ = run.plan.fail(stage, FailureCause::Stale);
        }
        self.ended(run, end, None)
    }

    fn store_single(
        &self,
        record: &mut SceneRecord,
        run: &mut JobRun,
        stage: SceneStage,
        handle: ResourceHandle,
    ) -> Polled {
        let output = match self.hub.take_result(handle) {
            Ok(result) => output_of(stage, handle, result),
            Err(ResultError::NotSettled) => {
                return Polled::Waiting(Open::Single { stage, handle });
            }
            Err(ResultError::Unknown | ResultError::NoResult { .. }) => {
                Err(FailureCause::InvalidAnswer)
            }
        };
        let output = match output {
            Ok(output) => output,
            Err(cause) => return Polled::Ended(self.fail(record, run, stage, cause)),
        };
        match run.plan.store(self.snapshot, output) {
            Ok(()) => {
                record.keep_scene_outputs(run.plan.outputs());
                self.stored(run, stage, StageSource::Requested);
                Polled::Stored
            }
            Err(PlanError::Stale(stale)) => Polled::Ended(self.end_stale(run, stale)),
            Err(_) => Polled::Ended(self.fail(record, run, stage, FailureCause::InvalidAnswer)),
        }
    }

    fn stored(&self, run: &JobRun, stage: SceneStage, source: StageSource) {
        let job = run.plan.job();
        self.record(SceneBuilderEvent::StageStored {
            scene: job.scene(),
            generation: job.stamp().generation,
            job: job.id(),
            stage,
            source,
            ms: self.now_ms.saturating_sub(run.stage_started_ms),
        });
    }

    fn poll_locate(
        &self,
        record: &mut SceneRecord,
        run: &mut JobRun,
        mut locate: LocateRun,
    ) -> Polled {
        let mut settled = Vec::with_capacity(locate.requests.len());
        for (handle, prompt) in &locate.requests {
            let Some(status) = self.hub.status(*handle) else {
                return Polled::Ended(self.end_unwanted(run, JobEnd::Stale));
            };
            match status {
                RequestStatus::Queued { .. }
                | RequestStatus::BackingOff { .. }
                | RequestStatus::InFlight { .. } => return Polled::Waiting(Open::Locate(locate)),
                RequestStatus::Cancelled | RequestStatus::Superseded => {
                    return Polled::Ended(self.end_unwanted(run, JobEnd::Stale));
                }
                RequestStatus::Ready => {
                    settled.push((prompt.clone(), self.locate_answer(*handle)));
                }
                RequestStatus::Failed { cause } => settled.push((prompt.clone(), Err(cause))),
                RequestStatus::Dropped => {
                    settled.push((prompt.clone(), Err(FailureCause::Stale)));
                }
            }
        }
        locate.requests.clear();
        for (prompt, answer) in settled {
            if locate.plan.record(&prompt, answer).is_err() {
                return Polled::Ended(self.fail(
                    record,
                    run,
                    SceneStage::Locate,
                    FailureCause::InvalidAnswer,
                ));
            }
        }
        match self.issue_round(record, run, &mut locate) {
            Some(()) => Polled::Waiting(Open::Locate(locate)),
            None => match self.store_locate(record, run, locate.plan, StageSource::Requested) {
                Started::Ended(output) => Polled::Ended(output),
                Started::Open(_) | Started::Stored => Polled::Stored,
            },
        }
    }

    /// The instances one segmentation request found, or why its answer cannot be used.
    fn locate_answer(&self, handle: ResourceHandle) -> Result<Vec<Instance>, FailureCause> {
        match self.hub.take_result(handle) {
            Ok(ResourceResult::Text(text)) => {
                parse_locate_answer(&text).map_err(|_| FailureCause::TextInvalid)
            }
            Ok(ResourceResult::Image { .. }) | Err(_) => Err(FailureCause::InvalidAnswer),
        }
    }

    fn store_locate(
        &self,
        record: &SceneRecord,
        run: &mut JobRun,
        plan: LocatePlan,
        source: StageSource,
    ) -> Started {
        let outcome = plan.finish();
        let counts = counts_of(&outcome);
        match run.plan.store(self.snapshot, StageOutput::Locate(outcome)) {
            Ok(()) => {
                let job = run.plan.job();
                self.record(SceneBuilderEvent::Located {
                    scene: job.scene(),
                    generation: job.stamp().generation,
                    job: job.id(),
                    counts,
                });
                self.stored(run, SceneStage::Locate, source);
                Started::Stored
            }
            Err(PlanError::Stale(stale)) => Started::Ended(self.end_stale(run, stale)),
            Err(_) => Started::Ended(self.fail(
                record,
                run,
                SceneStage::Locate,
                FailureCause::InvalidAnswer,
            )),
        }
    }

    fn finish(&self, record: &mut SceneRecord, mut run: JobRun) -> Driven {
        match run.plan.finish(self.snapshot, self.now_ms) {
            Ok(ready) => {
                self.record(SceneBuilderEvent::JobReady {
                    scene: ready.job.scene(),
                    generation: ready.job.stamp().generation,
                    job: ready.job.id(),
                    ms: ready.build_ms(),
                });
                record.first_plate_ready = true;
                Driven::Ended(BuilderOutput::PlateReady(ready))
            }
            Err(PlanError::Stale(stale)) => Driven::Ended(self.end_stale(&run, stale)),
            Err(_) => Driven::Ended(self.fail(
                record,
                &mut run,
                SceneStage::Ready,
                FailureCause::InvalidAnswer,
            )),
        }
    }
}

/// A stage's output from what its request produced, or why it cannot be used.
pub(super) fn output_of(
    stage: SceneStage,
    handle: ResourceHandle,
    result: ResourceResult,
) -> Result<StageOutput, FailureCause> {
    let picture = |result: ResourceResult| match result {
        ResourceResult::Image { width, height, .. } => Ok(StoredPicture {
            handle: handle.get(),
            width,
            height,
        }),
        ResourceResult::Text(_) => Err(FailureCause::InvalidAnswer),
    };
    let text = |result: ResourceResult| match result {
        ResourceResult::Text(text) if !text.expose().trim().is_empty() => Ok(text),
        ResourceResult::Text(_) => Err(FailureCause::TextInvalid),
        ResourceResult::Image { .. } => Err(FailureCause::InvalidAnswer),
    };
    match stage {
        SceneStage::Describe => text(result).map(StageOutput::Description),
        SceneStage::Background => picture(result).map(StageOutput::Background),
        SceneStage::Plate => picture(result).map(StageOutput::Plate),
        SceneStage::Locate | SceneStage::Ready => Err(FailureCause::InvalidAnswer),
    }
}

/// Cancels every request the run has out: the job will not use their results.
pub(super) fn cancel_open(hub: &BrokerHub, run: &JobRun) {
    match &run.open {
        Some(Open::Single { handle, .. }) => {
            hub.cancel(*handle);
        }
        Some(Open::Locate(locate)) => {
            for (handle, _) in &locate.requests {
                hub.cancel(*handle);
            }
        }
        None => {}
    }
}
