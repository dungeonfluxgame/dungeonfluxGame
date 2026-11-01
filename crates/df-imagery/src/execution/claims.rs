//! Pure claim, sequence, capacity and recovery decisions for execution.
//!
//! These functions read only their arguments: no clock, no storage, no
//! provider. The persistence port applies them inside its transaction next
//! to the row locks that make them durable.

/// True while a worker may still use the claim: valid instants precede it.
pub fn lease_is_live(lease_until_ms: i64, now_ms: i64) -> bool {
    now_ms < lease_until_ms
}

/// True when the presented token and generation own the stored claim.
pub fn claim_owns(
    presented_token: &[u8; 16],
    presented_generation: u64,
    stored_token: &[u8; 16],
    stored_generation: u64,
) -> bool {
    presented_token == stored_token && presented_generation == stored_generation
}

/// Next dispatch sequence after the highest one seen. The stage row lock
/// serializes prepares, so two workers never mint one sequence twice.
pub fn next_sequence(max_seen: Option<i32>) -> i32 {
    max_seen.map_or(0, |seen| seen.saturating_add(1))
}

/// True when the durable capacity row still has a free slot.
pub fn capacity_admits(in_flight: i64, bound: i64) -> bool {
    bound > 0 && in_flight >= 0 && in_flight < bound
}

/// True when the durable pending queue still fits one more job with
/// `job_bytes` retained intent/config bytes. Pending is tracked separately
/// from active concurrency; both gates must admit before new work reserves.
pub fn pending_admits(
    pending_jobs: i64,
    pending_bytes: i64,
    max_pending_jobs: i64,
    max_pending_bytes: i64,
    job_bytes: i64,
) -> bool {
    max_pending_jobs > 0
        && max_pending_bytes > 0
        && job_bytes >= 0
        && pending_jobs >= 0
        && pending_bytes >= 0
        && pending_jobs < max_pending_jobs
        && pending_bytes.saturating_add(job_bytes) <= max_pending_bytes
}

/// True when a result for this dispatch may still finalize it: only the
/// cursor's current dispatch completes; anything else is stale.
pub fn sender_may_complete(current: &[u8; 16], target: &[u8; 16]) -> bool {
    current == target
}

/// True for dispatch states that retire the durable capacity reserve.
/// Unknown keeps its liability and its reserve for reconciliation.
pub fn completion_releases_capacity(state: super::DispatchState) -> bool {
    matches!(
        state,
        super::DispatchState::Declined
            | super::DispatchState::Succeeded
            | super::DispatchState::Failed
            | super::DispatchState::Released
    )
}
