//! Transform-only execution transitions; provider and recovery operations refuse explicitly.
use super::state::Store;
use crate::execution::*;
use crate::{ImageJobId, ImageOwner, InterestState, JobState};

impl ImageExecutionJournal for Store {
    async fn enqueue(&self, command: &EnqueueCommand) -> Result<EnqueueOutcome, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let entry = state.job(&command.owner, command.job)?;
        if entry.stage.stage != command.stage
            || entry.basis.kind != command.kind
            || entry.basis.input_digest != command.input_digest
            || entry.basis.config_digest != command.config_digest
        {
            return Err(ExecutionError::BasisConflict);
        }
        Ok(EnqueueOutcome::Existing(entry.stage.clone()))
    }
    async fn claim(&self, command: &ClaimCommand) -> Result<StageView, ExecutionError> {
        let mut state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        if !state.gate.enabled {
            return Err(ExecutionError::Disabled);
        }
        if state.gate.applied_revision != command.expected_config_revision {
            return Err(ExecutionError::ConfigFenced {
                current_revision: state.gate.applied_revision,
            });
        }
        let pending_jobs = state
            .jobs
            .iter()
            .filter(|entry| {
                entry.stage.state == StageState::Planned && entry.job.state != JobState::Retired
            })
            .count();
        let pending_bytes: usize = state
            .jobs
            .iter()
            .filter(|entry| {
                entry.stage.state == StageState::Planned && entry.job.state != JobState::Retired
            })
            .map(|entry| entry.job.intent.bytes().len() + entry.job.config.bytes().len())
            .sum();
        let pending_fits = i64::try_from(pending_jobs)
            .map_err(|_| ExecutionError::InvalidRecord)?
            <= state.bounds.max_pending_jobs
            && i64::try_from(pending_bytes).map_err(|_| ExecutionError::InvalidRecord)?
                <= state.bounds.max_pending_bytes;
        let entry = state.job_mut(&command.owner, command.job)?;
        if entry.stage.stage != command.stage {
            return Err(ExecutionError::NotFound);
        }
        if entry.job.state == JobState::Retired || entry.interest.state != InterestState::Attached {
            return Err(ExecutionError::DemandLapsed);
        }
        if command.now_ms >= entry.job.absolute_deadline_ms {
            return Err(ExecutionError::Expired);
        }
        if entry.stage.lease_until_ms > command.now_ms {
            return if entry.claim == Some(command.claim) {
                Ok(entry.stage.clone())
            } else {
                Err(ExecutionError::LostClaim)
            };
        }
        if command.now_ms < entry.not_before_ms {
            return Err(ExecutionError::Deferred {
                not_before_ms: entry.not_before_ms,
            });
        }
        if !pending_fits {
            return Err(ExecutionError::CapacityFull);
        }
        let until = command
            .now_ms
            .checked_add(command.lease_ms)
            .ok_or(ExecutionError::InvalidRecord)?;
        if until > entry.job.absolute_deadline_ms {
            return Err(ExecutionError::Expired);
        }
        entry.claim = Some(command.claim);
        entry.deferred = false;
        entry.queued_notice = None;
        entry.stage.claim_generation = entry
            .stage
            .claim_generation
            .checked_add(1)
            .ok_or(ExecutionError::InvalidRecord)?;
        entry.stage.lease_until_ms = until;
        entry.stage.state = StageState::Admitted;
        entry.bump(command.now_ms)?;
        Ok(entry.stage.clone())
    }
    async fn scan_queue(
        &self,
        scope: &str,
        limit: u32,
        now_ms: i64,
    ) -> Result<Vec<QueuedStage>, ExecutionError> {
        Ok(self
            .scan_queue_after(scope, None, limit, now_ms)
            .await?
            .stages)
    }
    async fn scan_queue_after(
        &self,
        scope: &str,
        after: Option<QueueCursor>,
        limit: u32,
        now_ms: i64,
    ) -> Result<QueuePage, ExecutionError> {
        if scope != self.scope.as_str() {
            return Err(ExecutionError::NotFound);
        }
        if !(1..=128).contains(&limit)
            || now_ms <= 0
            || after.is_some_and(|cursor| {
                cursor.ranking_at_ms <= 0
                    || cursor.ranking_at_ms > now_ms
                    || cursor.effective_urgency > 2
                    || cursor.created_at_ms < 0
                    || cursor.deadline_ms <= 0
            })
        {
            return Err(ExecutionError::InvalidRecord);
        }
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        if state.bounds.urgency_promotion_ms <= 0 {
            return Err(ExecutionError::InvalidRecord);
        }
        let ranking_at_ms = after.map_or(now_ms, |cursor| cursor.ranking_at_ms);
        let key = |cursor: &QueueCursor| {
            (
                cursor.effective_urgency,
                cursor.created_at_ms,
                cursor.deadline_ms,
                *cursor.stage.as_bytes(),
                *cursor.job.as_bytes(),
            )
        };
        let mut rows = Vec::new();
        for entry in &state.jobs {
            if entry.dispatch.is_some()
                || entry.job.state == JobState::Retired
                || entry.interest.state != InterestState::Attached
                || entry.job.absolute_deadline_ms <= now_ms
                || entry.stage.lease_until_ms > now_ms
                || entry.not_before_ms > now_ms
                || !matches!(
                    entry.stage.state,
                    StageState::Planned | StageState::Admitted
                )
            {
                continue;
            }
            let rank: u8 = match entry.urgency {
                crate::Urgency::Visible => 0,
                crate::Urgency::Present => 1,
                crate::Urgency::Speculative => 2,
            };
            let waited = ranking_at_ms.saturating_sub(entry.job.created_at_ms).max(0);
            let promoted = u8::try_from((waited / state.bounds.urgency_promotion_ms).min(2))
                .map_err(|_| ExecutionError::InvalidRecord)?;
            let cursor = QueueCursor {
                ranking_at_ms,
                effective_urgency: rank.saturating_sub(promoted),
                created_at_ms: entry.job.created_at_ms,
                deadline_ms: entry.job.absolute_deadline_ms,
                stage: entry.stage.stage,
                job: entry.job.job,
            };
            if after.is_some_and(|prior| key(&cursor) <= key(&prior)) {
                continue;
            }
            rows.push((
                cursor,
                QueuedStage {
                    owner: entry.job.owner.clone(),
                    job: entry.job.job,
                    stage: entry.stage.stage,
                    kind: entry.stage.kind,
                    state: entry.stage.state,
                    claim_generation: entry.stage.claim_generation,
                    lease_until_ms: entry.stage.lease_until_ms,
                    created_at_ms: entry.job.created_at_ms,
                },
            ));
        }
        rows.sort_by_key(|(cursor, _)| key(cursor));
        let count = usize::try_from(limit).map_err(|_| ExecutionError::InvalidRecord)?;
        rows.truncate(count);
        Ok(QueuePage {
            next: if rows.len() == count {
                rows.last().map(|(cursor, _)| *cursor)
            } else {
                None
            },
            stages: rows.into_iter().map(|(_, stage)| stage).collect(),
        })
    }
    async fn scan_recovery(
        &self,
        scope: &str,
        _after: Option<DispatchId>,
        _limit: u32,
        _now_ms: i64,
    ) -> Result<Vec<RecoveryStage>, ExecutionError> {
        if scope != self.scope.as_str() {
            return Err(ExecutionError::NotFound);
        }
        Ok(Vec::new())
    }
    async fn read_stage_basis(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> Result<StageBasis, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let entry = state.job(owner, *job)?;
        if entry.stage.stage != *stage {
            return Err(ExecutionError::NotFound);
        }
        Ok(entry.basis.clone())
    }
    async fn read_current_dispatch(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> Result<Option<DispatchView>, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let entry = state.job(owner, *job)?;
        if entry.stage.stage != *stage {
            return Err(ExecutionError::NotFound);
        }
        Ok(entry.dispatch.clone())
    }
    async fn read_child_stage(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        parent_dispatch: &DispatchId,
        kind: StageKind,
    ) -> Result<Option<StageView>, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let mut matches = state.jobs.iter().filter(|entry| {
            &entry.job.owner == owner
                && entry.job.job == *job
                && entry.basis.parent_dispatch == Some(*parent_dispatch)
                && entry.stage.kind == kind
        });
        let found = matches.next().map(|entry| entry.stage.clone());
        if matches.next().is_some() {
            return Err(ExecutionError::IdentityConflict);
        }
        Ok(found)
    }
    async fn defer_queued(&self, command: &DeferQueuedCommand) -> Result<(), ExecutionError> {
        let DeferQueuedCommand {
            scope,
            not_before_ms,
            notice,
            now_ms,
        } = command;
        if *now_ms <= 0 || not_before_ms <= now_ms {
            return Err(ExecutionError::InvalidRecord);
        }
        let mut state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let entry = state.job_mut(&scope.owner, scope.job)?;
        if entry.stage.stage != scope.stage {
            return Err(ExecutionError::NotFound);
        }
        if entry.stage.kind != StageKind::LocalTransform
            || !entry.stage.local_accounting
            || entry.dispatch.is_some()
            || entry.sent
        {
            return Err(ExecutionError::Superseded);
        }
        if entry.stage.state == StageState::Released {
            return Err(ExecutionError::Stale {
                current_revision: entry.job.revision,
            });
        }
        if entry.stage.lease_until_ms > *now_ms {
            return Err(ExecutionError::LostClaim);
        }
        if entry.job.state == JobState::Retired || entry.interest.state != InterestState::Attached {
            return Err(ExecutionError::DemandLapsed);
        }
        let due = entry.not_before_ms.max(*not_before_ms);
        let queued_notice = notice.map(|reason| QueueDeferralNotice {
            reason,
            retry_at_ms: due,
        });
        if entry.not_before_ms == due && entry.queued_notice == queued_notice {
            return Ok(());
        }
        entry.not_before_ms = due;
        entry.queued_notice = queued_notice;
        entry.deferred = true;
        entry.stage.state = StageState::Planned;
        entry.stage.lease_until_ms = 0;
        entry.claim = None;
        entry.bump(*now_ms)?;
        Ok(())
    }
    async fn read_capacity_bounds(&self, scope: &str) -> Result<CapacityBounds, ExecutionError> {
        if scope != self.scope.as_str() {
            return Err(ExecutionError::NotFound);
        }
        Ok(self.lock().map_err(|_| ExecutionError::Unavailable)?.bounds)
    }
    async fn prepare(&self, command: &PrepareCommand) -> Result<DispatchView, ExecutionError> {
        let mut state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        if !state.gate.enabled {
            return Err(ExecutionError::Disabled);
        }
        let entry = state.job_mut(&command.owner, command.job)?;
        if entry.stage.stage != command.stage || !command.mode.is_local() {
            return Err(ExecutionError::InvalidRecord);
        }
        entry.check_claim(command.claim, command.claim_generation)?;
        if let Some(dispatch) = &entry.dispatch {
            return if dispatch.dispatch == command.dispatch {
                Ok(dispatch.clone())
            } else {
                Err(ExecutionError::Superseded)
            };
        }
        let dispatch = DispatchView {
            owner: command.owner.clone(),
            job: command.job,
            stage: command.stage,
            dispatch: command.dispatch,
            sequence: 1,
            state: DispatchState::Prepared,
            cause: None,
            settlement: DispatchSettlement::Held,
            finance: None,
            local_accounting: true,
            supplier_job_id: None,
            usage_revision: 0,
            book_call_id: book_call_id_for(&command.dispatch),
            lineage_parent: None,
            claim_generation: command.claim_generation,
            created_at_ms: command.now_ms,
            updated_at_ms: command.now_ms,
        };
        entry.dispatch = Some(dispatch.clone());
        entry.bump(command.now_ms)?;
        Ok(dispatch)
    }
    async fn mark_sent(&self, command: &MarkSentCommand) -> Result<DispatchView, ExecutionError> {
        let mut state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        if !state.gate.enabled {
            return Err(ExecutionError::Disabled);
        }
        let entry = state.job_mut(&command.owner, command.job)?;
        if entry.stage.stage != command.stage {
            return Err(ExecutionError::NotFound);
        }
        entry.check_claim(command.claim, command.claim_generation)?;
        let dispatch = entry.dispatch.as_mut().ok_or(ExecutionError::NotFound)?;
        if dispatch.dispatch != command.dispatch {
            return Err(ExecutionError::Superseded);
        }
        if dispatch.state == DispatchState::Sent {
            return Ok(dispatch.clone());
        }
        if dispatch.state != DispatchState::Prepared {
            return Err(ExecutionError::OperationConflict);
        }
        dispatch.state = DispatchState::Sent;
        dispatch.updated_at_ms = command.now_ms;
        let dispatch = dispatch.clone();
        entry.bump(command.now_ms)?;
        Ok(dispatch)
    }
    async fn read_output_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
    ) -> Result<Option<OutputCheckpoint>, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let entry = state.dispatch(owner, *dispatch)?;
        if entry.job.job != *job || entry.stage.stage != *stage {
            return Err(ExecutionError::NotFound);
        }
        Ok(entry.checkpoint.clone())
    }
    async fn complete(&self, command: &CompleteCommand) -> Result<DispatchView, ExecutionError> {
        let mut state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let entry = state.job_mut(&command.owner, command.job)?;
        if entry.stage.stage != command.stage {
            return Err(ExecutionError::NotFound);
        }
        entry.check_claim(command.claim, command.claim_generation)?;
        let current = entry.dispatch.as_ref().ok_or(ExecutionError::NotFound)?;
        if current.dispatch != command.dispatch {
            return Err(ExecutionError::Superseded);
        }
        if matches!(command.outcome, CompletionOutcome::Declined { .. }) {
            return Err(ExecutionError::ObservationMissing);
        }
        let unknown_cause = match &command.outcome {
            CompletionOutcome::Unknown => Some(None),
            CompletionOutcome::UnknownWithCause { cause } => Some(Some(*cause)),
            _ => None,
        };
        if let Some(cause) = unknown_cause {
            if matches!(
                current.state,
                DispatchState::Succeeded
                    | DispatchState::Failed
                    | DispatchState::Released
                    | DispatchState::Declined
            ) {
                return Ok(current.clone());
            }
            if current.state == DispatchState::Unknown {
                match (current.cause, cause) {
                    (_, None) => return Ok(current.clone()),
                    (Some(stored), Some(incoming)) if stored == incoming => {
                        return Ok(current.clone());
                    }
                    (Some(_), Some(_)) => return Err(ExecutionError::OperationConflict),
                    (None, Some(_)) => {}
                }
            }
        }
        if let Some(outcome) = &entry.completion {
            if *outcome == command.outcome {
                return Ok(current.clone());
            }
            if current.state != DispatchState::Unknown {
                return Err(ExecutionError::OperationConflict);
            }
        }
        let dispatch = entry.dispatch.as_mut().ok_or(ExecutionError::NotFound)?;
        if !matches!(dispatch.state, DispatchState::Sent | DispatchState::Unknown) {
            return Err(ExecutionError::Superseded);
        }
        match &command.outcome {
            CompletionOutcome::Succeeded {
                usage_revision,
                checkpoint,
            } => {
                if entry.checkpoint.as_ref() != Some(checkpoint) || *usage_revision != 0 {
                    return Err(ExecutionError::InvalidRecord);
                }
                dispatch.state = DispatchState::Succeeded;
                dispatch.cause = None;
                dispatch.settlement = DispatchSettlement::Settled;
            }
            CompletionOutcome::Failed { cause } => {
                dispatch.state = DispatchState::Failed;
                dispatch.cause = Some(*cause);
                dispatch.settlement = DispatchSettlement::Released;
            }
            CompletionOutcome::Unknown | CompletionOutcome::UnknownWithCause { .. } => {
                dispatch.state = DispatchState::Unknown;
                dispatch.cause = unknown_cause.flatten();
                dispatch.settlement = DispatchSettlement::UnknownOutstanding;
            }
            _ => return Err(ExecutionError::InvalidRecord),
        }
        dispatch.updated_at_ms = command.now_ms;
        let dispatch = dispatch.clone();
        entry.completion = Some(command.outcome.clone());
        entry.bump(command.now_ms)?;
        Ok(dispatch)
    }
    async fn read_stage(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> Result<StageView, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let entry = state.job(owner, *job)?;
        if entry.stage.stage != *stage {
            return Err(ExecutionError::NotFound);
        }
        Ok(entry.stage.clone())
    }
    async fn read_dispatch(
        &self,
        owner: &ImageOwner,
        dispatch: &DispatchId,
    ) -> Result<DispatchView, ExecutionError> {
        self.lock()
            .map_err(|_| ExecutionError::Unavailable)?
            .dispatch(owner, *dispatch)?
            .dispatch
            .clone()
            .ok_or(ExecutionError::NotFound)
    }
    async fn set_capacity_bounds(
        &self,
        scope: &str,
        bounds: &CapacityBounds,
        _now_ms: i64,
    ) -> Result<(), ExecutionError> {
        if scope != self.scope.as_str() {
            return Err(ExecutionError::NotFound);
        }
        self.lock().map_err(|_| ExecutionError::Unavailable)?.bounds = *bounds;
        Ok(())
    }
    async fn set_execution_gate(
        &self,
        scope: &str,
        applied_revision: u64,
        enabled: bool,
        _now_ms: i64,
    ) -> Result<(), ExecutionError> {
        if scope != self.scope.as_str() {
            return Err(ExecutionError::NotFound);
        }
        let mut state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        if applied_revision < state.gate.applied_revision {
            return Err(ExecutionError::ConfigFenced {
                current_revision: state.gate.applied_revision,
            });
        }
        state.gate = ExecutionGate {
            scope: scope.to_owned(),
            applied_revision,
            enabled,
        };
        Ok(())
    }
    async fn read_execution_gate(&self, scope: &str) -> Result<ExecutionGate, ExecutionError> {
        if scope != self.scope.as_str() {
            return Err(ExecutionError::NotFound);
        }
        Ok(self
            .lock()
            .map_err(|_| ExecutionError::Unavailable)?
            .gate
            .clone())
    }
    async fn scan_unsent_terminal(
        &self,
        scope: &str,
        after: Option<TerminalCursor>,
        limit: u32,
        now_ms: i64,
    ) -> Result<Vec<UnsentTerminal>, ExecutionError> {
        if scope != self.scope.as_str() {
            return Err(ExecutionError::NotFound);
        }
        if !(1..=128).contains(&limit) || now_ms <= 0 {
            return Err(ExecutionError::InvalidRecord);
        }
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let mut found = Vec::new();
        for entry in &state.jobs {
            let stage_scope = StageScope {
                owner: entry.job.owner.clone(),
                job: entry.job.job,
                stage: entry.stage.stage,
            };
            let key = (*stage_scope.job.as_bytes(), *stage_scope.stage.as_bytes());
            if after.is_some_and(|cursor| key <= (*cursor.job.as_bytes(), *cursor.stage.as_bytes()))
                || entry.dispatch.is_some()
                || entry.sent
                || entry.checkpoint.is_some()
                || entry.completion.is_some()
                || (entry.job.state != JobState::Retired
                    && entry.interest.state == InterestState::Attached
                    && entry.job.absolute_deadline_ms > now_ms)
            {
                continue;
            }
            let terminal = match entry.stage.state {
                StageState::Planned
                    if entry.stage.claim_generation == 0 && entry.claim.is_none() =>
                {
                    UnsentTerminal::Unclaimed(UnclaimedTerminal {
                        scope: stage_scope,
                        job_revision: entry.job.revision,
                    })
                }
                StageState::Admitted if entry.stage.claim_generation > 0 => {
                    UnsentTerminal::Claimed(ClaimedTerminal {
                        claim: StageClaim {
                            scope: stage_scope,
                            token: entry.claim.ok_or(ExecutionError::InvalidRecord)?,
                            generation: entry.stage.claim_generation,
                        },
                        job_revision: entry.job.revision,
                    })
                }
                StageState::Planned
                    if entry.stage.claim_generation > 0
                        && entry.deferred
                        && entry.claim.is_none()
                        && entry.stage.lease_until_ms == 0 =>
                {
                    UnsentTerminal::Deferred(DeferredTerminal {
                        scope: stage_scope,
                        job_revision: entry.job.revision,
                        claim_generation: entry.stage.claim_generation,
                        not_before_ms: entry.not_before_ms,
                    })
                }
                // This fixture never has a paid Candidate or its pending catalog allocation.
                StageState::Released => continue,
                _ => return Err(ExecutionError::InvalidRecord),
            };
            found.push(terminal);
        }
        found.sort_by_key(|row| (*row.scope().job.as_bytes(), *row.scope().stage.as_bytes()));
        found.truncate(usize::try_from(limit).map_err(|_| ExecutionError::InvalidRecord)?);
        Ok(found)
    }
    async fn scan_unsent_prepared(
        &self,
        scope: &str,
        after: Option<DispatchId>,
        limit: u32,
        now_ms: i64,
    ) -> Result<Vec<UnsentPrepared>, ExecutionError> {
        if scope != self.scope.as_str() {
            return Err(ExecutionError::NotFound);
        }
        if !(1..=128).contains(&limit) || now_ms <= 0 {
            return Err(ExecutionError::InvalidRecord);
        }
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let mut found = Vec::new();
        for entry in &state.jobs {
            let Some(current) = &entry.dispatch else {
                continue;
            };
            if current.state != DispatchState::Prepared
                || entry.sent
                || after.is_some_and(|cursor| current.dispatch.to_hex() <= cursor.to_hex())
            {
                continue;
            }
            if entry.stage.state == StageState::Released {
                // The local-only fixture never creates held financial release tails.
                return Err(ExecutionError::InvalidRecord);
            }
            let terminal = entry.job.state == JobState::Retired
                || entry.job.absolute_deadline_ms <= now_ms
                || entry.interest.state != InterestState::Attached;
            if entry.stage.lease_until_ms > now_ms && !terminal {
                continue;
            }
            let token = entry.claim.ok_or(ExecutionError::InvalidRecord)?;
            found.push(UnsentPrepared {
                scope: ExecutionScope {
                    owner: entry.job.owner.clone(),
                    job: entry.job.job,
                    stage: entry.stage.stage,
                    dispatch: current.dispatch,
                },
                claim: StageClaim {
                    scope: StageScope {
                        owner: entry.job.owner.clone(),
                        job: entry.job.job,
                        stage: entry.stage.stage,
                    },
                    token,
                    generation: entry.stage.claim_generation,
                },
                current: current.clone(),
                job_revision: entry.job.revision,
                original_lease_until_ms: entry.stage.lease_until_ms,
                terminal_condition: terminal,
                sealed: false,
            });
        }
        found.sort_by_key(|row| row.current.dispatch.to_hex());
        found.truncate(usize::try_from(limit).map_err(|_| ExecutionError::InvalidRecord)?);
        Ok(found)
    }
    async fn claim_recovery(
        &self,
        _command: &ClaimRecoveryCommand,
    ) -> Result<RecoveryLease, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn finish_recovery(
        &self,
        _command: &FinishRecoveryCommand,
    ) -> Result<(), ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn prepare_reconciliation(
        &self,
        _parent: &RecoveryLease,
        _command: &ReconciliationCommand,
    ) -> Result<ReconciliationDispatch, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn record_decision_observation(
        &self,
        _observation: &DecisionObservation,
    ) -> Result<DecisionObservation, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn read_decision_observation(
        &self,
        _owner: &ImageOwner,
        _job: &ImageJobId,
        _stage: &ExecutionStageId,
        _dispatch: &DispatchId,
        _version: Option<u32>,
    ) -> Result<Option<DecisionObservation>, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn read_decision_checkpoint(
        &self,
        _owner: &ImageOwner,
        _job: &ImageJobId,
        _stage: &ExecutionStageId,
        _dispatch: &DispatchId,
    ) -> Result<Option<DecisionObservation>, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn record_candidate_observation(
        &self,
        _observation: &CandidateObservation,
    ) -> Result<CandidateReceipt, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn read_candidate_observation(
        &self,
        _owner: &ImageOwner,
        _job: &ImageJobId,
        _stage: &ExecutionStageId,
        _dispatch: &DispatchId,
        _version: Option<u32>,
    ) -> Result<Option<CandidateReceipt>, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn read_candidate_checkpoint(
        &self,
        _owner: &ImageOwner,
        _job: &ImageJobId,
        _stage: &ExecutionStageId,
        _dispatch: &DispatchId,
    ) -> Result<Option<CandidateReceipt>, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn record_local_inspection(
        &self,
        command: &LocalInspectionCommand,
    ) -> Result<LocalInspectionCheckpoint, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let claim = command.claim();
        let entry = state.job(&claim.scope.owner, claim.scope.job)?;
        if entry.stage.stage != claim.scope.stage {
            return Err(ExecutionError::NotFound);
        }
        entry.check_claim(claim.token, claim.generation)?;
        // This fixture admits only LocalTransform; no selected supplier Candidate exists.
        Err(ExecutionError::InvalidRecord)
    }
    async fn read_local_inspection(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> Result<Option<LocalInspectionCheckpoint>, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let entry = state.job(owner, *job)?;
        if entry.stage.stage != *stage {
            return Err(ExecutionError::NotFound);
        }
        Ok(None)
    }
    async fn read_analysis_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
    ) -> Result<Option<AnalysisCheckpoint>, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        state.job(owner, *job)?;
        Ok(None)
    }
    async fn select_analysis_checkpoint(
        &self,
        proof: &crate::runtime::QualifiedAnalysis,
        _now_ms: i64,
    ) -> Result<AnalysisCheckpoint, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        state.job(&proof.job().owner, proof.job().job)?;
        Err(ExecutionError::InvalidRecord)
    }
    async fn record_local_analysis(
        &self,
        proof: &crate::runtime::QualifiedLocalAnalysis,
        claim: &StageClaim,
        _now_ms: i64,
    ) -> Result<LocalAnalysisCheckpoint, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let entry = state.job(&proof.job().owner, proof.job().job)?;
        if claim.scope.owner != entry.job.owner
            || claim.scope.job != entry.job.job
            || claim.scope.stage != entry.stage.stage
        {
            return Err(ExecutionError::NotFound);
        }
        entry.check_claim(claim.token, claim.generation)?;
        Err(ExecutionError::InvalidRecord)
    }
    async fn read_local_analysis_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> Result<Option<LocalAnalysisCheckpoint>, ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let entry = state.job(owner, *job)?;
        if entry.stage.stage != *stage {
            return Err(ExecutionError::NotFound);
        }
        Ok(None)
    }
    async fn report_supplier(
        &self,
        _command: &SupplierReport,
    ) -> Result<DispatchView, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn record_supplier_checkpoint(
        &self,
        _checkpoint: &SupplierCheckpoint,
    ) -> Result<SupplierCheckpoint, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn read_supplier_checkpoint(
        &self,
        _owner: &ImageOwner,
        _job: &ImageJobId,
        _stage: &ExecutionStageId,
        _dispatch: &DispatchId,
    ) -> Result<Option<SupplierCheckpoint>, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn acknowledge_unsent_projection(
        &self,
        command: &AcknowledgeUnsentProjection,
    ) -> Result<(), ExecutionError> {
        self.check_cpu_scope(&command.scope)?;
        Err(ExecutionError::OperationConflict)
    }
    async fn record_decline_observation(
        &self,
        observation: &DeclineObservation,
    ) -> Result<DeclineReceipt, ExecutionError> {
        self.check_cpu_scope(&observation.as_receipt().scope)?;
        Err(ExecutionError::BasisConflict)
    }
    async fn read_decline_reference(
        &self,
        scope: &ExecutionScope,
    ) -> Result<Option<DeclineReceipt>, ExecutionError> {
        self.check_cpu_scope(scope)?;
        Ok(None)
    }
    async fn requeue_declined(
        &self,
        command: &DeclineRetryCommand,
    ) -> Result<StageView, ExecutionError> {
        self.check_cpu_scope(&ExecutionScope {
            owner: command.claim.scope.owner.clone(),
            job: command.claim.scope.job,
            stage: command.claim.scope.stage,
            dispatch: command.dispatch,
        })?;
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        state
            .job(&command.claim.scope.owner, command.claim.scope.job)?
            .check_claim(command.claim.token, command.claim.generation)?;
        Err(ExecutionError::BasisConflict)
    }
    async fn cancel(&self, _command: &CancelCommand) -> Result<DispatchView, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn request_refetch(
        &self,
        _command: &RefetchCommand,
    ) -> Result<DispatchView, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn release(&self, _command: &ReleaseCommand) -> Result<StageView, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
    async fn resolve(
        &self,
        _owner: &ImageOwner,
        _identity: &ExecutionIdentity,
    ) -> Result<ResolvedExecution, ExecutionError> {
        Err(ExecutionError::InvalidRecord)
    }
}

impl Store {
    fn check_cpu_scope(&self, scope: &ExecutionScope) -> Result<(), ExecutionError> {
        let state = self.lock().map_err(|_| ExecutionError::Unavailable)?;
        let entry = state.job(&scope.owner, scope.job)?;
        if entry.stage.stage != scope.stage
            || entry.dispatch.as_ref().map(|row| row.dispatch) != Some(scope.dispatch)
        {
            return Err(ExecutionError::NotFound);
        }
        if entry.stage.kind != StageKind::LocalTransform || !entry.stage.local_accounting {
            return Err(ExecutionError::BasisConflict);
        }
        Ok(())
    }
}
