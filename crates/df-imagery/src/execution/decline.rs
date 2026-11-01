//! Protected submit-rejection evidence permits a new admission without releasing old liability.
use super::{
    DispatchId, DispatchState, DispatchView, ExecutionError, ExecutionScope, FinanceTuple,
    ObservationReceipt, StageBasis, StageClaim, StageKind,
};
use crate::ImageJob;
use crate::backends::api::{ApiFailure, DeclineContract};
use crate::runtime::RuntimeUsage;
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitDeclineContract {
    OpenAiImages429V1,
    FalQueue429V1,
}
impl SubmitDeclineContract {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAiImages429V1 => "openai-images-429-v1",
            Self::FalQueue429V1 => "fal-queue-429-v1",
        }
    }
    pub fn parse_label(value: &str) -> Option<Self> {
        match value {
            "openai-images-429-v1" => Some(Self::OpenAiImages429V1),
            "fal-queue-429-v1" => Some(Self::FalQueue429V1),
            _ => None,
        }
    }
}
#[derive(Clone, Eq, PartialEq)]
pub struct DeclineRoute {
    pub provider: String,
    pub model: String,
    pub cause: String,
    pub detail: String,
}
impl fmt::Debug for DeclineRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeclineRoute { .. }")
    }
}
/// Retained retry policy is decoded before locks and bound to the immutable job configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeclineRetryPolicy {
    pub max_attempts: u32,
    pub retry_deadline_ms: i64,
    pub backoff_ms: i64,
    pub config_snapshot_revision: u32,
}
#[derive(Clone, Eq, PartialEq)]
pub struct DeclineReceipt {
    pub scope: ExecutionScope,
    pub basis: StageBasis,
    pub finance: Option<FinanceTuple>,
    pub observation_version: u32,
    pub receipt: ObservationReceipt,
    pub contract: SubmitDeclineContract,
    pub status: u16,
    pub body_sha256: [u8; 32],
    pub body_bytes: u64,
    pub raw_sha256: [u8; 32],
    pub raw_bytes: u64,
    pub route: DeclineRoute,
    pub retry_not_before_ms: i64,
    pub transport_request_id_sha256: Option<[u8; 32]>,
    pub retry_after_values: Vec<Vec<u8>>,
    pub retry: DeclineRetryPolicy,
}
impl fmt::Debug for DeclineReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeclineReceipt")
            .field("contract", &self.contract)
            .field("observation_version", &self.observation_version)
            .finish_non_exhaustive()
    }
}
/// Only the native worker can construct recordable evidence from an actual protocol failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclineObservation {
    inner: DeclineReceipt,
}
impl DeclineObservation {
    pub fn as_receipt(&self) -> &DeclineReceipt {
        &self.inner
    }
    pub(crate) fn qualify(
        job: &ImageJob,
        dispatch: &DispatchView,
        basis: StageBasis,
        failure: &ApiFailure,
        usage: &RuntimeUsage,
    ) -> Result<Self, ExecutionError> {
        let contract = match failure
            .retryable_submit_decline()
            .ok_or(ExecutionError::InvalidRecord)?
        {
            DeclineContract::OpenAiImages429V1 => SubmitDeclineContract::OpenAiImages429V1,
            DeclineContract::FalQueue429V1 => SubmitDeclineContract::FalQueue429V1,
            DeclineContract::BflImages429V1 => return Err(ExecutionError::InvalidRecord),
        };
        if dispatch.owner != job.owner
            || dispatch.job != job.job
            || usage.owner != job.owner
            || usage.job != job.job
            || usage.stage != dispatch.stage
            || usage.dispatch != dispatch.dispatch
            || basis.kind == StageKind::LocalTransform
            || !matches!(dispatch.state, DispatchState::Sent | DispatchState::Unknown)
            || usage.evidence != crate::cost::UsageEvidence::Unknown
            || usage.usable
            || usage.units != usage.route.maxima
            || usage.revision.0 == 0
        {
            return Err(ExecutionError::InvalidRecord);
        }
        let observed_at_ms =
            i64::try_from(usage.at_ms).map_err(|_| ExecutionError::InvalidRecord)?;
        let config = crate::intent_codec::decode_config(
            job.config.bytes(),
            crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| ExecutionError::InvalidRecord)?;
        if basis.config_digest != <[u8; 32]>::from(Sha256::digest(job.config.bytes())) {
            return Err(ExecutionError::BasisConflict);
        }
        let max_attempts = config.retry.max_attempts;
        let backoff_secs = config.retry.backoff_secs;
        let retry_deadline_secs = config.effective_retry_deadline_secs();
        let retry_deadline_ms = i64::try_from(
            retry_deadline_secs
                .checked_mul(1000)
                .ok_or(ExecutionError::InvalidRecord)?,
        )
        .map_err(|_| ExecutionError::InvalidRecord)?;
        let backoff_ms = i64::try_from(
            backoff_secs
                .checked_mul(1000)
                .ok_or(ExecutionError::InvalidRecord)?,
        )
        .map_err(|_| ExecutionError::InvalidRecord)?;
        let response = failure
            .response
            .as_ref()
            .ok_or(ExecutionError::InvalidRecord)?;
        let instant = std::time::UNIX_EPOCH
            .checked_add(std::time::Duration::from_millis(
                u64::try_from(observed_at_ms).map_err(|_| ExecutionError::InvalidRecord)?,
            ))
            .ok_or(ExecutionError::InvalidRecord)?;
        let delay = response
            .retry_after
            .minimum_delay(instant)
            .ok_or(ExecutionError::InvalidRecord)?;
        let minimum =
            i64::try_from(delay.as_millis()).map_err(|_| ExecutionError::InvalidRecord)?;
        let retry_not_before_ms = observed_at_ms
            .checked_add(minimum.max(backoff_ms))
            .ok_or(ExecutionError::InvalidRecord)?;
        let raw = usage
            .raw_output
            .as_deref()
            .ok_or(ExecutionError::ObservationMissing)?;
        let value = df_providers::parse_unique_json(
            raw,
            df_providers::TextSchemaLimits {
                maximum_schema_bytes: raw.len(),
                maximum_depth: 32,
                maximum_nodes: raw.len(),
            },
        )
        .map_err(|_| ExecutionError::InvalidRecord)?;
        let expected = serde_json::json!({"schema_revision":2,"kind":"image-api-failure","exchange":"submit","submission":"rejected_submit","decline_contract":contract.label(),"status":response.status,"body":response.body,"transport_request_id":response.transport_request_id.as_ref().map(|id|id.as_str()),"retry_after_values":response.retry_after.raw_values()});
        if value != expected
            || raw.is_empty()
            || observed_at_ms <= 0
            || max_attempts == 0
            || retry_deadline_ms <= 0
        {
            return Err(ExecutionError::InvalidRecord);
        }
        Ok(Self {
            inner: DeclineReceipt {
                scope: ExecutionScope {
                    owner: job.owner.clone(),
                    job: job.job,
                    stage: dispatch.stage,
                    dispatch: dispatch.dispatch,
                },
                basis,
                finance: dispatch.finance,
                observation_version: 1,
                receipt: ObservationReceipt {
                    usage_revision: u32::try_from(usage.revision.0)
                        .map_err(|_| ExecutionError::InvalidRecord)?,
                    observed_at_ms,
                },
                contract,
                status: response.status,
                body_sha256: Sha256::digest(&response.body).into(),
                body_bytes: u64::try_from(response.body.len())
                    .map_err(|_| ExecutionError::InvalidRecord)?,
                raw_sha256: Sha256::digest(raw).into(),
                raw_bytes: u64::try_from(raw.len()).map_err(|_| ExecutionError::InvalidRecord)?,
                route: DeclineRoute {
                    provider: usage.route.provider.clone(),
                    model: usage.route.model.clone(),
                    cause: usage.route.cause.clone(),
                    detail: usage.route.detail.clone(),
                },
                retry_not_before_ms,
                transport_request_id_sha256: response
                    .transport_request_id
                    .as_ref()
                    .map(|id| Sha256::digest(id.as_str().as_bytes()).into()),
                retry_after_values: response.retry_after.raw_values().to_vec(),
                retry: DeclineRetryPolicy {
                    max_attempts,
                    retry_deadline_ms,
                    backoff_ms,
                    config_snapshot_revision: job.config.revision(),
                },
            },
        })
    }
}
/// Requeue is technical authority only; a fresh claim and independent finance admission precede send.
#[derive(Clone, Eq, PartialEq)]
pub struct DeclineRetryCommand {
    pub claim: StageClaim,
    pub dispatch: DispatchId,
    pub now_ms: i64,
}
impl fmt::Debug for DeclineRetryCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeclineRetryCommand { .. }")
    }
}
