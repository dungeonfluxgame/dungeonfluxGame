//! Retained supplier usage and its projection into the cost book, for every stage the worker
//! runs. A revision is recorded once per distinct observation; projection always settles the
//! newest retained revision and reports a not-yet-durable book entry as pending.

use super::{ports::*, worker::NativeImageRuntime};
use crate::cost::{ImageCostProjection, UsageEvidence, UsageReport, UsageRevision};
use crate::execution::*;
use crate::{ImageJournal, ServiceClock};

impl<J, E, S, P, F, C, I, K, B, T, Q, U> NativeImageRuntime<J, E, S, P, F, C, I, K, B, T, Q, U>
where
    J: ImageJournal,
    E: ImageExecutionJournal,
    S: ImageSources,
    P: ImagePublisher,
    F: ImageDispatchAdmission,
    C: ServiceClock,
    I: RuntimeIds,
    K: ImageCredentials,
    B: ImageCostProjection,
    T: ImageRuntimeTelemetry,
    Q: ImageRuntimeCostPolicy,
    U: ImageUsageJournal,
{
    pub(super) async fn retain_usage(
        &self,
        context: &PublicationContext,
        route: &crate::cost::CostRoute,
        evidence: UsageEvidence,
        units: &[crate::cost::CostUnits],
        raw_output: Option<&[u8]>,
        usable: bool,
    ) -> Result<RuntimeUsage, RuntimeError> {
        let units = if evidence == UsageEvidence::Unknown {
            route.maxima.clone()
        } else {
            units.to_vec()
        };
        let previous = self
            .usage
            .latest_usage(&context.job.owner, context.dispatch)
            .await?;
        if let Some(previous) = &previous
            && previous.route == *route
            && previous.evidence == evidence
            && previous.units == units
            && previous.usable == usable
            && previous.raw_output.as_deref() == raw_output
        {
            return Ok(previous.clone());
        }
        let revision = previous
            .map_or(Some(1), |usage| usage.revision.0.checked_add(1))
            .ok_or(RuntimeError::InvalidRecord)?;
        let usage = RuntimeUsage {
            owner: context.job.owner.clone(),
            job: context.job.job,
            stage: context.stage.stage,
            dispatch: context.dispatch,
            revision: UsageRevision(revision),
            route: route.clone(),
            evidence,
            units,
            usable,
            at_ms: u64::try_from(self.clock.now_ms()).map_err(|_| RuntimeError::InvalidRecord)?,
            raw_output: raw_output.map(<[u8]>::to_vec),
        };
        self.usage.record_usage(&usage).await?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::UsageRecorded,
            self.event_context(&context.job, &context.stage, Some(context.dispatch)),
        );
        event.measurements.usage_revision = Some(usage.revision);
        event.measurements.usage_evidence = Some(usage.evidence);
        event.measurements.raw_checkpointed = Some(usage.raw_output.is_some());
        self.telemetry.observe(event);
        Ok(usage)
    }

    pub(super) async fn project_usage(&self, usage: &RuntimeUsage) -> Result<(), RuntimeError> {
        self.project_usage_observed(usage, true).await
    }

    /// A projection that is not yet durable is finished by the cost log's own acknowledgement, so
    /// it must not abort a pipeline that still has to queue its next stage.
    pub(super) async fn project_usage_or_pending(
        &self,
        usage: &RuntimeUsage,
    ) -> Result<(), RuntimeError> {
        match self.project_usage(usage).await {
            Ok(()) | Err(RuntimeError::CostProjectionPending) => Ok(()),
            Err(error) => Err(error),
        }
    }
    pub(super) async fn project_usage_observed(
        &self,
        usage: &RuntimeUsage,
        report_pending: bool,
    ) -> Result<(), RuntimeError> {
        let latest = self
            .usage
            .latest_usage(&usage.owner, usage.dispatch)
            .await?;
        let selected = latest
            .as_ref()
            .filter(|latest| latest.revision.0 > usage.revision.0)
            .unwrap_or(usage);
        if selected.owner != usage.owner
            || selected.job != usage.job
            || selected.stage != usage.stage
            || selected.route != usage.route
        {
            return Err(RuntimeError::InvalidRecord);
        }
        let usage = selected;
        let result = self.project_usage_inner(usage).await;
        if let Err(error) = &result {
            if *error == RuntimeError::CostProjectionPending && !report_pending {
                return result;
            }
            let context = crate::telemetry::EventContext {
                job: Some(usage.job),
                stage: Some(usage.stage),
                attempt: Some(usage.dispatch),
                gate_revision: Some(self.settings.active_config_revision),
                ..Default::default()
            };
            let pending = *error == RuntimeError::CostProjectionPending;
            let mut event = crate::telemetry::ImageEvent::new(
                if pending {
                    crate::telemetry::EventKind::UsageProjectionPending
                } else {
                    crate::telemetry::EventKind::UsageProjectionFailed
                },
                context,
            );
            if !pending {
                event.failure = Some(crate::telemetry::FailureKind::UsageProjection);
            }
            event.measurements.usage_revision = Some(usage.revision);
            event.measurements.usage_evidence = Some(usage.evidence);
            self.telemetry.observe(event);
        }
        result
    }

    async fn project_usage_inner(&self, usage: &RuntimeUsage) -> Result<(), RuntimeError> {
        let call_id = crate::cost::pinned_call_id(usage.dispatch.as_bytes());
        self.costs
            .start_pinned(&call_id, &usage.route)
            .await
            .map_err(RuntimeError::Cost)?;
        let report = UsageReport {
            revision: usage.revision,
            evidence: usage.evidence,
            units: &usage.units,
            usable: usage.usable,
            at_ms: usage.at_ms,
        };
        match self.costs.correct(&call_id, &report).await {
            Ok(_) => {}
            Err(crate::cost::CostError::StaleRevision { stored, .. }) => {
                return self.acknowledge_newer_projection(usage, stored).await;
            }
            Err(error) => return Err(RuntimeError::Cost(error)),
        }
        if !self
            .costs
            .is_durable(&call_id, Some(usage.revision))
            .await
            .map_err(RuntimeError::Cost)?
        {
            return Err(RuntimeError::CostProjectionPending);
        }
        self.usage
            .acknowledge_projection(&usage.owner, usage.dispatch, usage.revision)
            .await?;
        let mut event = crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::UsageProjected,
            crate::telemetry::EventContext {
                job: Some(usage.job),
                stage: Some(usage.stage),
                attempt: Some(usage.dispatch),
                gate_revision: Some(self.settings.active_config_revision),
                ..Default::default()
            },
        );
        event.measurements.usage_revision = Some(usage.revision);
        event.measurements.usage_evidence = Some(usage.evidence);
        self.telemetry.observe(event);
        Ok(())
    }

    async fn acknowledge_newer_projection(
        &self,
        older: &RuntimeUsage,
        revision: UsageRevision,
    ) -> Result<(), RuntimeError> {
        let scope = ExecutionScope {
            owner: older.owner.clone(),
            job: older.job,
            stage: older.stage,
            dispatch: older.dispatch,
        };
        let current = self
            .usage
            .read_usage_revision(&scope, revision)
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
        if current.revision.0 <= older.revision.0 || current.route != older.route {
            return Err(RuntimeError::InvalidRecord);
        }
        let receipt = self
            .costs
            .read_receipt(&crate::cost::pinned_call_id(older.dispatch.as_bytes()))
            .await
            .map_err(RuntimeError::Cost)?
            .ok_or(RuntimeError::StorageUnavailable)?;
        if !receipt.durable {
            return Err(RuntimeError::CostProjectionPending);
        }
        let max_word_bytes = [
            &current.route.provider,
            &current.route.model,
            &current.route.cause,
            &current.route.detail,
        ]
        .into_iter()
        .map(|word| word.len())
        .chain(
            current
                .route
                .maxima
                .iter()
                .chain(&current.units)
                .map(|unit| unit.category.len()),
        )
        .max()
        .ok_or(RuntimeError::InvalidRecord)?;
        let limits = super::UsageProjectionLimits {
            max_attempts: 1,
            max_observations: 1,
            max_unit_lines: current.route.maxima.len().max(current.units.len()),
            max_word_bytes,
        };
        super::usage_projection::ActualCostReceipt::verify(receipt, &current, limits)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        self.usage
            .acknowledge_projection(&current.owner, current.dispatch, current.revision)
            .await
    }
}
