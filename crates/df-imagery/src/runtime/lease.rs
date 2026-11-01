//! Stage leases and provider call budgets.
//!
//! A claim lease covers the stage budget plus a fixed margin for pre-send work, so a provider
//! call that uses its whole budget is not cut off by the lease that protects it. Both values
//! stop at the job's absolute deadline. A call cut off at its budget is classified as unknown
//! by the caller, never silently retried.

use std::time::Duration;

use super::ports::RuntimeError;
use crate::ImageConfig;

/// Admission, prepare and the send mark must finish inside this margin before the call starts.
pub(super) const PRE_SEND_MARGIN_MS: i64 = 30_000;

fn stage_budget_ms(config: &ImageConfig) -> Result<i64, RuntimeError> {
    i64::try_from(config.scheduler.stage_deadline_secs)
        .map_err(|_| RuntimeError::InvalidConfiguration)?
        .checked_mul(1000)
        .ok_or(RuntimeError::InvalidConfiguration)
}

/// Lease for a new claim: the stage budget plus the pre-send margin, capped by the job deadline.
pub(super) fn claim_lease_ms(
    config: &ImageConfig,
    absolute_deadline_ms: i64,
    now_ms: i64,
) -> Result<i64, RuntimeError> {
    let remaining = absolute_deadline_ms
        .checked_sub(now_ms)
        .ok_or(RuntimeError::Expired)?;
    let lease = stage_budget_ms(config)?
        .checked_add(PRE_SEND_MARGIN_MS)
        .ok_or(RuntimeError::InvalidConfiguration)?
        .min(remaining);
    if lease <= 0 {
        return Err(RuntimeError::Expired);
    }
    Ok(lease)
}

/// Provider call budget for a lease with `remaining_lease_ms` left: the stage budget, never more
/// than the lease still holds.
pub(super) fn call_budget(
    config: &ImageConfig,
    remaining_lease_ms: i64,
) -> Result<Duration, RuntimeError> {
    let budget = stage_budget_ms(config)?.min(remaining_lease_ms);
    if budget <= 0 {
        return Err(RuntimeError::Expired);
    }
    Ok(Duration::from_millis(
        u64::try_from(budget).map_err(|_| RuntimeError::Expired)?,
    ))
}

/// Provider call budget at send time, measured against the claimed lease's expiry.
pub(super) fn provider_call_timeout(
    config: &ImageConfig,
    lease_until_ms: i64,
    now_ms: i64,
) -> Result<Duration, RuntimeError> {
    let remaining = lease_until_ms
        .checked_sub(now_ms)
        .filter(|remaining| *remaining > 0)
        .ok_or(RuntimeError::Expired)?;
    call_budget(config, remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_covers_the_whole_stage_budget_plus_margin() {
        let config = ImageConfig::defaults().unwrap();
        let lease = claim_lease_ms(&config, i64::MAX / 4, 0).unwrap();
        let budget = i64::try_from(config.scheduler.stage_deadline_secs).unwrap() * 1000;
        assert_eq!(lease, budget + PRE_SEND_MARGIN_MS);
        assert_eq!(
            call_budget(&config, lease).unwrap(),
            Duration::from_secs(config.scheduler.stage_deadline_secs)
        );
    }

    #[test]
    fn call_budget_never_outlives_the_lease_or_the_job() {
        let config = ImageConfig::defaults().unwrap();
        assert_eq!(
            call_budget(&config, 5_000).unwrap(),
            Duration::from_millis(5_000)
        );
        assert!(matches!(
            call_budget(&config, 0),
            Err(RuntimeError::Expired)
        ));
        assert_eq!(claim_lease_ms(&config, 10_000, 0).unwrap(), 10_000);
        assert!(matches!(
            claim_lease_ms(&config, 10_000, 10_000),
            Err(RuntimeError::Expired)
        ));
    }
}
