//! Atomic fixture receipt and recipe records, shared with the execution fake.
use super::state::{Job, MAXIMUM_RECORDS, Store};
use crate::execution::*;
use crate::runtime::reservation_for;
use crate::*;
use df_types::OperationId;

impl ImageJournal for Store {
    async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
        self.accept_command(command, Urgency::Present)
    }
    async fn accept_scheduled(
        &self,
        command: &AcceptCommand,
        urgency: Urgency,
    ) -> Result<AcceptOutcome, JournalError> {
        self.accept_command(command, urgency)
    }
    async fn lookup_operation(
        &self,
        owner: &ImageOwner,
        operation: OperationId,
    ) -> Result<Option<OperationView>, JournalError> {
        Ok(self
            .lock()
            .map_err(|_| JournalError::Unavailable)?
            .jobs
            .iter()
            .find(|entry| &entry.job.owner == owner && entry.receipt.operation == operation)
            .map(Job::view))
    }
    async fn read_job(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
    ) -> Result<Option<ImageJob>, JournalError> {
        Ok(self
            .lock()
            .map_err(|_| JournalError::Unavailable)?
            .jobs
            .iter()
            .find(|entry| &entry.job.owner == owner && entry.job.job == job)
            .map(|entry| entry.job.clone()))
    }
    async fn freeze_recipe(&self, command: &FreezeCommand) -> Result<FreezeOutcome, JournalError> {
        let mut state = self.lock().map_err(|_| JournalError::Unavailable)?;
        let entry = state
            .job_mut(&command.owner, command.job)
            .map_err(|_| JournalError::NotFound)?;
        if let Some(recipe) = &entry.job.recipe {
            return if *recipe == command.recipe && entry.job.resolved_key == Some(command.key) {
                Ok(FreezeOutcome::AlreadyFrozen {
                    job: entry.job.clone(),
                })
            } else {
                Err(JournalError::RecipeConflict)
            };
        }
        if entry.job.revision != command.expected_revision {
            return Err(JournalError::Stale {
                current_revision: entry.job.revision,
            });
        }
        if entry.job.state == JobState::Retired {
            return Err(JournalError::Retired);
        }
        if command.now_ms >= entry.job.absolute_deadline_ms {
            return Err(JournalError::Expired);
        }
        entry.job.recipe = Some(command.recipe.clone());
        entry.job.resolved_key = Some(command.key);
        entry.job.state = JobState::RecipeFrozen;
        entry
            .bump(command.now_ms)
            .map_err(|_| JournalError::InvalidRecord)?;
        Ok(FreezeOutcome::Frozen {
            job: entry.job.clone(),
        })
    }
    async fn withdraw_interest(
        &self,
        owner: &ImageOwner,
        interest: ImageInterestId,
        now_ms: i64,
    ) -> Result<WithdrawOutcome, JournalError> {
        let mut state = self.lock().map_err(|_| JournalError::Unavailable)?;
        let entry = state
            .jobs
            .iter_mut()
            .find(|entry| &entry.job.owner == owner && entry.interest.interest == interest)
            .ok_or(JournalError::NotFound)?;
        let replay = entry.interest.state == InterestState::Withdrawn;
        if !replay {
            entry.interest.state = InterestState::Withdrawn;
            entry.interest.withdrawn_at_ms = Some(now_ms);
            entry.job.state = JobState::Retired;
            entry
                .bump(now_ms)
                .map_err(|_| JournalError::InvalidRecord)?;
        }
        let receipt = Withdrawal {
            interest: entry.interest.clone(),
            attached_remaining: 0,
            job_revision: entry.job.revision,
        };
        Ok(if replay {
            WithdrawOutcome::AlreadyWithdrawn(receipt)
        } else {
            WithdrawOutcome::Withdrawn(receipt)
        })
    }
}

impl Store {
    fn accept_command(
        &self,
        command: &AcceptCommand,
        urgency: Urgency,
    ) -> Result<AcceptOutcome, JournalError> {
        let mut state = self.lock().map_err(|_| JournalError::Unavailable)?;
        if command.identity.owner != state.owner {
            return Err(JournalError::NotFound);
        }
        if let Some(entry) = state
            .jobs
            .iter()
            .find(|entry| entry.receipt.operation == command.identity.operation)
        {
            return if entry.receipt.fingerprint == command.identity.fingerprint
                && entry.job.intent == command.intent
            {
                Ok(AcceptOutcome::Replayed(entry.view()))
            } else {
                Err(JournalError::OperationConflict)
            };
        }
        if state.jobs.len() >= MAXIMUM_RECORDS {
            return Err(JournalError::Unavailable);
        }
        let (job, deadline) = match command.target {
            AcceptTarget::Create {
                job,
                absolute_deadline_ms,
            } => (job, absolute_deadline_ms),
            AcceptTarget::Join { .. } | AcceptTarget::JoinReady { .. } => {
                return Err(JournalError::JoinRefused(JoinRefusal::SharingMismatch));
            }
        };
        if state
            .jobs
            .iter()
            .any(|entry| entry.job.job == job || entry.interest.interest == command.interest)
        {
            return Err(JournalError::IdentityConflict);
        }
        let reserve = reservation_for(command, self.scope.clone(), 1, true, &self.ids)?;
        if reserve.kind != StageKind::LocalTransform {
            return Err(JournalError::InvalidRecord);
        }
        let entry = Job {
            job: ImageJob {
                job,
                owner: command.identity.owner.clone(),
                sharing: command.sharing,
                intent: command.intent.clone(),
                config: command.config.clone(),
                absolute_deadline_ms: deadline,
                state: JobState::Planning,
                revision: 1,
                resolved_key: None,
                recipe: None,
                created_at_ms: command.now_ms,
                updated_at_ms: command.now_ms,
            },
            receipt: OperationReceipt {
                owner: command.identity.owner.clone(),
                operation: command.identity.operation,
                fingerprint: command.identity.fingerprint,
                job,
                interest: command.interest,
            },
            interest: ImageInterest {
                interest: command.interest,
                owner: command.identity.owner.clone(),
                job,
                operation: command.identity.operation,
                state: InterestState::Attached,
                created_at_ms: command.now_ms,
                withdrawn_at_ms: None,
            },
            stage: StageView {
                owner: command.identity.owner.clone(),
                job,
                stage: reserve.stage,
                kind: reserve.kind,
                state: StageState::Planned,
                claim_generation: 0,
                lease_until_ms: 0,
                local_accounting: true,
                planned_publication: reserve.planned_publication,
                job_revision: 1,
            },
            basis: StageBasis {
                kind: reserve.kind,
                input_revision: reserve.input_revision,
                input_digest: reserve.input_digest,
                config_revision: reserve.config_revision,
                config_digest: reserve.config_digest,
                parent_dispatch: None,
            },
            claim: None,
            dispatch: None,
            checkpoint: None,
            completion: None,
            staged: None,
            result: None,
            sent: false,
            not_before_ms: command.now_ms,
            deferred: false,
            queued_notice: None,
            urgency,
        };
        let view = entry.view();
        state.jobs.push(entry);
        Ok(AcceptOutcome::Created(view))
    }
}
