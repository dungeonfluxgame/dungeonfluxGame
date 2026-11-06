//! Account-wide guard for an actual new supplier attempt, independent of image identity.
use super::ports::{ImageCredentials, ImageRuntimeTelemetry, RuntimeError};
use super::quota::{
    QuotaAccount, QuotaAdmission, QuotaAnswer, QuotaError, QuotaPermit, QuotaTransition,
};
use crate::config::QuotaSettings;

/// Uses the exact credential and destination already pinned for the prepared request.
pub(crate) fn admit(
    credentials: &impl ImageCredentials,
    policy: QuotaSettings,
    endpoint: &str,
    secret: &str,
    trusted_identity: Option<&[u8; 32]>,
) -> Result<Option<QuotaAdmission>, RuntimeError> {
    let Some(registry) = credentials.quota_registry() else {
        return if policy.enabled {
            Err(RuntimeError::InvalidConfiguration)
        } else {
            Ok(None)
        };
    };
    let account =
        QuotaAccount::from_credential(endpoint, secret, trusted_identity).map_err(refusal)?;
    registry.admit(&account, policy).map(Some).map_err(refusal)
}

/// Revalidates after execution capacity is owned, before finance or send admission.
pub(crate) fn claim(
    admission: Option<QuotaAdmission>,
) -> Result<Option<QuotaPermit>, RuntimeError> {
    admission
        .map(QuotaAdmission::claim)
        .transpose()
        .map_err(refusal)
}

/// Quota health cannot discard an already observed supplier result or its raw usage.
pub(crate) fn observe(
    permit: Option<QuotaPermit>,
    answer: QuotaAnswer,
    telemetry: &dyn ImageRuntimeTelemetry,
    context: crate::telemetry::EventContext,
) {
    let Some(permit) = permit else {
        return;
    };
    let event = match permit.finish(answer) {
        Ok(Some(QuotaTransition::Opened)) => Some(crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::QuotaOpened,
            context,
        )),
        Ok(Some(QuotaTransition::Recovered)) => Some(crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::QuotaRecovered,
            context,
        )),
        Ok(None) => None,
        Err(_) => Some(
            crate::telemetry::ImageEvent::new(crate::telemetry::EventKind::Failed, context)
                .failed(crate::telemetry::FailureKind::Capacity),
        ),
    };
    if let Some(event) = event {
        telemetry.observe(event);
    }
}

pub(crate) fn raw_answer(status: u16, body: &[u8], useful: bool) -> QuotaAnswer {
    if status == 429 && crate::backends::api::openai::is_quota_exhaustion(body) {
        QuotaAnswer::Exhausted
    } else if (200..300).contains(&status) && useful {
        QuotaAnswer::Succeeded
    } else {
        QuotaAnswer::Inconclusive
    }
}

fn refusal(error: QuotaError) -> RuntimeError {
    match error {
        QuotaError::Cooldown { retry_at_ms } => RuntimeError::QuotaBlocked {
            retry_at_ms: Some(retry_at_ms),
            notice: Some(crate::execution::QuotaDeferralReason::Cooldown),
        },
        QuotaError::Probing => RuntimeError::QuotaBlocked {
            retry_at_ms: None,
            notice: Some(crate::execution::QuotaDeferralReason::Probing),
        },
        QuotaError::AdmissionChanged | QuotaError::RegistryFull => RuntimeError::QuotaBlocked {
            retry_at_ms: None,
            notice: None,
        },
        QuotaError::InvalidIdentity | QuotaError::InvalidPolicy => {
            RuntimeError::InvalidConfiguration
        }
        QuotaError::InvalidTime | QuotaError::StateUnavailable => RuntimeError::WorkerFailed,
    }
}
