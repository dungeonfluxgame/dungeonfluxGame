//! Pure admission decision over an injected configuration snapshot.
//!
//! The service calls `decide` before reserving spend, launching provider
//! work, or starting local stages. No clock, ambient state, or I/O is read.

use crate::config::{BackendId, GlobalMode, ImageConfig};
use crate::request::{ExecutionMode, Purpose};

/// Everything the decision may inspect besides the frozen config: the
/// resolved route, the request execution mode, which stages the recipe needs,
/// and whether a compatible ready result already exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyInput {
    pub purpose: Purpose,
    pub backend: BackendId,
    pub execution: ExecutionMode,
    pub needs_provider: bool,
    pub needs_local: bool,
    pub cache_hit: bool,
}

impl PolicyInput {
    pub fn new(
        purpose: Purpose,
        backend: BackendId,
        execution: ExecutionMode,
        needs_provider: bool,
        needs_local: bool,
        cache_hit: bool,
    ) -> Self {
        Self {
            purpose,
            backend,
            execution,
            needs_provider,
            needs_local,
            cache_hit,
        }
    }
}

/// Admit new work, reuse a compatible ready result, or refuse with a reason
/// the caller can branch on. Refusal never starts provider or pixel work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyDecision {
    Admit,
    ReuseReady,
    Refuse(Refusal),
}

/// Closed refusal reasons in evaluation order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    ServiceOff,
    PurposeDisabled,
    PreparedMiss,
    ReplayMiss,
    CacheOnlyMiss,
    TransformsDisabled,
    ProviderStageNotPermitted,
    BackendDisabled,
}

/// Decides admission without side effects. A disabled purpose refuses even on
/// a ready hit: existing asset reads are a separate path, while submit for a
/// disabled purpose is always refused. Request-level prepared-only or replay
/// never computes a new stage on a miss, and a disabled required transform
/// is refused before any provider dispatch is considered.
pub fn decide(config: &ImageConfig, input: &PolicyInput) -> PolicyDecision {
    if config.mode == GlobalMode::Off {
        return PolicyDecision::Refuse(Refusal::ServiceOff);
    }
    if !config.purpose(input.purpose).enabled {
        return PolicyDecision::Refuse(Refusal::PurposeDisabled);
    }
    if input.cache_hit {
        return PolicyDecision::ReuseReady;
    }
    match input.execution {
        ExecutionMode::PreparedOnly => {
            return PolicyDecision::Refuse(Refusal::PreparedMiss);
        }
        ExecutionMode::Replay => return PolicyDecision::Refuse(Refusal::ReplayMiss),
        ExecutionMode::Live => {}
    }
    if config.mode == GlobalMode::CacheOnly {
        return PolicyDecision::Refuse(Refusal::CacheOnlyMiss);
    }
    if input.needs_local && !config.transforms.enabled {
        return PolicyDecision::Refuse(Refusal::TransformsDisabled);
    }
    if input.needs_provider && config.mode == GlobalMode::LocalOnly {
        return PolicyDecision::Refuse(Refusal::ProviderStageNotPermitted);
    }
    if input.needs_provider && !config.backend(input.backend).enabled {
        return PolicyDecision::Refuse(Refusal::BackendDisabled);
    }
    PolicyDecision::Admit
}
