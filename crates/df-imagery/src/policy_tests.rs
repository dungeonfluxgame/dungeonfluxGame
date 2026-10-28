use super::*;
use crate::config::{BackendId, GlobalMode, ImageConfig};
use crate::request::{ExecutionMode, Purpose};

fn live_config() -> ImageConfig {
    ImageConfig::defaults().unwrap()
}

fn input(
    execution: ExecutionMode,
    needs_provider: bool,
    needs_local: bool,
    cache_hit: bool,
) -> PolicyInput {
    PolicyInput::new(
        Purpose::ScenePlate,
        BackendId::OpenAi,
        execution,
        needs_provider,
        needs_local,
        cache_hit,
    )
}

#[test]
fn off_mode_refuses_even_on_a_ready_hit() {
    let mut config = live_config();
    config.mode = GlobalMode::Off;
    let probe = input(ExecutionMode::Live, true, false, true);
    assert_eq!(
        decide(&config, &probe),
        PolicyDecision::Refuse(Refusal::ServiceOff)
    );
}

#[test]
fn disabled_purpose_is_refused_even_on_a_ready_hit() {
    let mut config = live_config();
    config.purposes[Purpose::ScenePlate.index()].enabled = false;
    let probe = input(ExecutionMode::Live, true, false, true);
    assert_eq!(
        decide(&config, &probe),
        PolicyDecision::Refuse(Refusal::PurposeDisabled)
    );
}

#[test]
fn cache_only_reuses_a_ready_hit_without_new_work() {
    let mut config = live_config();
    config.mode = GlobalMode::CacheOnly;
    let probe = input(ExecutionMode::Live, true, true, true);
    assert_eq!(decide(&config, &probe), PolicyDecision::ReuseReady);
}

#[test]
fn cache_only_miss_refuses_without_provider_work() {
    let mut config = live_config();
    config.mode = GlobalMode::CacheOnly;
    let probe = input(ExecutionMode::Live, true, false, false);
    assert_eq!(
        decide(&config, &probe),
        PolicyDecision::Refuse(Refusal::CacheOnlyMiss)
    );
}

#[test]
fn replay_miss_never_computes_a_new_stage() {
    let config = live_config();
    let probe = input(ExecutionMode::Replay, true, true, false);
    assert_eq!(
        decide(&config, &probe),
        PolicyDecision::Refuse(Refusal::ReplayMiss)
    );
}

#[test]
fn replay_hit_reuses_the_ready_result() {
    let config = live_config();
    let probe = input(ExecutionMode::Replay, true, true, true);
    assert_eq!(decide(&config, &probe), PolicyDecision::ReuseReady);
}

#[test]
fn prepared_only_miss_never_starts_local_work() {
    let config = live_config();
    let probe = input(ExecutionMode::PreparedOnly, false, true, false);
    assert_eq!(
        decide(&config, &probe),
        PolicyDecision::Refuse(Refusal::PreparedMiss)
    );
}

#[test]
fn local_only_allows_local_transforms_without_provider_dispatch() {
    let mut config = live_config();
    config.mode = GlobalMode::LocalOnly;
    let probe = input(ExecutionMode::Live, false, true, false);
    assert_eq!(decide(&config, &probe), PolicyDecision::Admit);
}

#[test]
fn local_only_refuses_provider_stages() {
    let mut config = live_config();
    config.mode = GlobalMode::LocalOnly;
    let probe = input(ExecutionMode::Live, true, false, false);
    assert_eq!(
        decide(&config, &probe),
        PolicyDecision::Refuse(Refusal::ProviderStageNotPermitted)
    );
}

#[test]
fn disabled_required_transform_is_refused_before_provider_dispatch() {
    let mut config = live_config();
    config.transforms.enabled = false;
    let probe = input(ExecutionMode::Live, true, true, false);
    assert_eq!(
        decide(&config, &probe),
        PolicyDecision::Refuse(Refusal::TransformsDisabled)
    );
}

#[test]
fn disabled_purpose_refuses_new_work() {
    let mut config = live_config();
    config.purposes[Purpose::ScenePlate.index()].enabled = false;
    let probe = input(ExecutionMode::Live, false, true, false);
    assert_eq!(
        decide(&config, &probe),
        PolicyDecision::Refuse(Refusal::PurposeDisabled)
    );
}

#[test]
fn disabled_backend_refuses_provider_work_but_not_local_recipes() {
    let mut config = live_config();
    config.backends[BackendId::OpenAi.index()].enabled = false;
    let provider = input(ExecutionMode::Live, true, false, false);
    assert_eq!(
        decide(&config, &provider),
        PolicyDecision::Refuse(Refusal::BackendDisabled)
    );
    let local = input(ExecutionMode::Live, false, true, false);
    assert_eq!(decide(&config, &local), PolicyDecision::Admit);
}

#[test]
fn live_miss_with_all_gates_open_admits() {
    let config = live_config();
    let probe = input(ExecutionMode::Live, true, true, false);
    assert_eq!(decide(&config, &probe), PolicyDecision::Admit);
}
