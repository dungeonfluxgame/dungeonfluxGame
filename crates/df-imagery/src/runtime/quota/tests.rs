use super::*;
use std::sync::atomic::{AtomicI64, Ordering};

struct Clock(AtomicI64);
impl ServiceClock for Clock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
impl Clock {
    fn set(&self, now: i64) {
        self.0.store(now, Ordering::SeqCst);
    }
}
fn fixture() -> (Arc<Clock>, Arc<QuotaRegistry>, QuotaAccount, QuotaSettings) {
    let clock = Arc::new(Clock(AtomicI64::new(1_000)));
    let registry = Arc::new(QuotaRegistry::new(clock.clone()));
    let account = account("https://api.openai.com/v1/images/edits", "fixture-key");
    let settings = QuotaSettings {
        cooldown_ms: 60,
        ..QuotaSettings::default()
    };
    (clock, registry, account, settings)
}
fn account(endpoint: &str, credential: &str) -> QuotaAccount {
    QuotaAccount::from_credential(endpoint, credential, None).unwrap()
}
fn claim(registry: &QuotaRegistry, account: &QuotaAccount, settings: QuotaSettings) -> QuotaPermit {
    registry.admit(account, settings).unwrap().claim().unwrap()
}
fn open(registry: &QuotaRegistry, account: &QuotaAccount, settings: QuotaSettings) {
    assert_eq!(
        claim(registry, account, settings).finish(QuotaAnswer::Exhausted),
        Ok(Some(QuotaTransition::Opened))
    );
}
fn refusal<T>(result: Result<T, QuotaError>) -> QuotaError {
    match result {
        Ok(_) => panic!("admission unexpectedly succeeded"),
        Err(error) => error,
    }
}

#[test]
fn aliases_and_image_analysis_paths_share_one_actual_credential_account() {
    let (clock, registry, image, settings) = fixture();
    let analysis = account("https://API.OPENAI.COM:443/v1/responses", "fixture-key");
    assert!(image == analysis);
    open(&registry, &image, settings);
    let another_worker = registry.clone();
    assert_eq!(
        refusal(another_worker.admit(&analysis, settings)),
        QuotaError::Cooldown { retry_at_ms: 1_060 }
    );
    clock.set(1_060);
    assert_eq!(
        claim(&another_worker, &analysis, settings).finish(QuotaAnswer::Succeeded),
        Ok(Some(QuotaTransition::Recovered))
    );
    assert!(registry.admit(&image, settings).is_ok());
}

#[test]
fn unrelated_credentials_and_origins_do_not_share_quota() {
    let (_, registry, original, settings) = fixture();
    open(&registry, &original, settings);
    for other in [
        account("https://api.openai.com/v1/images/edits", "dedicated-key"),
        account("https://different.example/v1/images/edits", "fixture-key"),
    ] {
        assert!(other != original);
        assert!(registry.admit(&other, settings).is_ok());
    }
}

#[test]
fn trusted_custody_can_identify_one_account_across_separate_keys() {
    let (_, registry, _, settings) = fixture();
    let first = QuotaAccount::from_credential(
        "https://api.openai.com/v1/images/edits",
        "first-key",
        Some(&[7; 32]),
    )
    .unwrap();
    let second = QuotaAccount::from_credential(
        "https://api.openai.com/v1/responses",
        "second-key",
        Some(&[7; 32]),
    )
    .unwrap();
    let dedicated = QuotaAccount::from_credential(
        "https://api.openai.com/v1/responses",
        "second-key",
        Some(&[8; 32]),
    )
    .unwrap();
    assert!(first == second && first != dedicated);
    open(&registry, &first, settings);
    assert_eq!(
        refusal(registry.admit(&second, settings)),
        QuotaError::Cooldown { retry_at_ms: 1_060 }
    );
    assert!(registry.admit(&dedicated, settings).is_ok());
}

#[test]
fn distinct_new_work_is_refused_without_sending_until_the_full_cooldown() {
    let (clock, registry, account, settings) = fixture();
    let mut supplier_calls = 0;
    let mut dispatch = |answer| {
        let permit = registry.admit(&account, settings)?.claim()?;
        supplier_calls += 1;
        permit.finish(answer)
    };
    assert_eq!(
        dispatch(QuotaAnswer::Exhausted),
        Ok(Some(QuotaTransition::Opened))
    );
    for now in [1_001, 1_030, 1_059] {
        clock.set(now);
        assert_eq!(
            dispatch(QuotaAnswer::Succeeded),
            Err(QuotaError::Cooldown { retry_at_ms: 1_060 })
        );
    }
    clock.set(1_060);
    assert_eq!(
        dispatch(QuotaAnswer::Succeeded),
        Ok(Some(QuotaTransition::Recovered))
    );
    assert_eq!(supplier_calls, 2);
}

#[test]
fn waiting_admission_revalidates_when_another_call_exhausts_quota() {
    let (_, registry, account, settings) = fixture();
    let waiting = registry.admit(&account, settings).unwrap();
    open(&registry, &account, settings);
    assert_eq!(
        refusal(waiting.claim()),
        QuotaError::Cooldown { retry_at_ms: 1_060 }
    );
}

#[test]
fn exactly_one_waiting_call_claims_the_probe_after_cooldown() {
    let (clock, registry, account, settings) = fixture();
    open(&registry, &account, settings);
    clock.set(1_060);
    let first = registry.admit(&account, settings).unwrap();
    let second = registry.admit(&account, settings).unwrap();
    let probe = first.claim().unwrap();
    assert_eq!(refusal(second.claim()), QuotaError::Probing);
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Probing
    );
    assert_eq!(
        probe.finish(QuotaAnswer::Succeeded),
        Ok(Some(QuotaTransition::Recovered))
    );
    assert_eq!(
        claim(&registry, &account, settings).finish(QuotaAnswer::Succeeded),
        Ok(None)
    );
}

#[test]
fn a_safe_rate_limit_retry_keeps_the_same_exclusive_probe_until_actual_success() {
    let (clock, registry, account, settings) = fixture();
    open(&registry, &account, settings);
    clock.set(1_060);
    let probe = claim(&registry, &account, settings);
    // The runtime retains this permit while its existing safe-429 policy retries.
    clock.set(1_100);
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Probing
    );
    assert_eq!(
        probe.finish(QuotaAnswer::Succeeded),
        Ok(Some(QuotaTransition::Recovered))
    );
    assert!(registry.admit(&account, settings).is_ok());
}

#[test]
fn a_probe_exhausting_quota_rearms_without_a_second_open_transition() {
    let (clock, registry, account, settings) = fixture();
    open(&registry, &account, settings);
    clock.set(1_060);
    assert_eq!(
        claim(&registry, &account, settings).finish(QuotaAnswer::Exhausted),
        Ok(None)
    );
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Cooldown { retry_at_ms: 1_120 }
    );
}

#[test]
fn an_inconclusive_probe_waits_a_full_cooldown_without_claiming_recovery() {
    let (clock, registry, account, settings) = fixture();
    open(&registry, &account, settings);
    clock.set(1_060);
    assert_eq!(
        claim(&registry, &account, settings).finish(QuotaAnswer::Inconclusive),
        Ok(None)
    );
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Cooldown { retry_at_ms: 1_120 }
    );
    clock.set(1_120);
    assert!(registry.admit(&account, settings).is_ok());
}

#[test]
fn cancelled_probe_rearms_from_injected_cancellation_time() {
    let (clock, registry, account, settings) = fixture();
    open(&registry, &account, settings);
    clock.set(1_060);
    let probe = claim(&registry, &account, settings);
    clock.set(1_200);
    drop(probe);
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Cooldown { retry_at_ms: 1_260 }
    );
    clock.set(1_260);
    assert!(registry.admit(&account, settings).is_ok());
}

#[test]
fn unrelated_inconclusive_work_never_opens_or_displaces_a_probe() {
    let (clock, registry, account, settings) = fixture();
    assert_eq!(
        claim(&registry, &account, settings).finish(QuotaAnswer::Inconclusive),
        Ok(None)
    );
    let concurrent = claim(&registry, &account, settings);
    open(&registry, &account, settings);
    clock.set(1_060);
    let probe = claim(&registry, &account, settings);
    assert_eq!(concurrent.finish(QuotaAnswer::Inconclusive), Ok(None));
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Probing
    );
    assert_eq!(
        probe.finish(QuotaAnswer::Succeeded),
        Ok(Some(QuotaTransition::Recovered))
    );
}

#[test]
fn dropping_an_old_probe_does_not_overwrite_a_new_quota_episode() {
    let (clock, registry, account, settings) = fixture();
    let concurrent_success = claim(&registry, &account, settings);
    let concurrent_quota = claim(&registry, &account, settings);
    open(&registry, &account, settings);
    clock.set(1_060);
    let old_probe = claim(&registry, &account, settings);
    assert_eq!(
        concurrent_success.finish(QuotaAnswer::Succeeded),
        Ok(Some(QuotaTransition::Recovered))
    );
    assert_eq!(
        concurrent_quota.finish(QuotaAnswer::Exhausted),
        Ok(Some(QuotaTransition::Opened))
    );
    clock.set(1_120);
    let current_probe = claim(&registry, &account, settings);
    drop(old_probe);
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Probing
    );
    current_probe.finish(QuotaAnswer::Succeeded).unwrap();
}

#[test]
fn disabled_developer_policy_bypasses_without_erasing_account_quota() {
    let (_, registry, account, settings) = fixture();
    open(&registry, &account, settings);
    let disabled = QuotaSettings {
        enabled: false,
        ..settings
    };
    assert_eq!(
        claim(&registry, &account, disabled).finish(QuotaAnswer::Succeeded),
        Ok(None)
    );
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Cooldown { retry_at_ms: 1_060 }
    );
}

#[test]
fn a_stale_probe_admission_cannot_claim_after_an_actual_inflight_success_recovers_quota() {
    let (clock, registry, account, settings) = fixture();
    let actual_inflight = claim(&registry, &account, settings);
    open(&registry, &account, settings);
    clock.set(1_060);
    let waiting = registry.admit(&account, settings).unwrap();
    assert_eq!(
        actual_inflight.finish(QuotaAnswer::Succeeded),
        Ok(Some(QuotaTransition::Recovered))
    );
    assert_eq!(refusal(waiting.claim()), QuotaError::AdmissionChanged);
    assert!(registry.admit(&account, settings).is_ok());
}

#[test]
fn a_backwards_clock_cannot_shorten_a_cancelled_probe_cooldown() {
    let (clock, registry, account, settings) = fixture();
    open(&registry, &account, settings);
    clock.set(1_060);
    let probe = claim(&registry, &account, settings);
    clock.set(1);
    drop(probe);
    clock.set(1_060);
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Cooldown { retry_at_ms: 1_120 }
    );
}

#[test]
fn changing_cooldown_settings_does_not_rewrite_a_retained_open_interval() {
    let (clock, registry, account, settings) = fixture();
    open(&registry, &account, settings);
    let faster = QuotaSettings {
        cooldown_ms: 1,
        ..settings
    };
    clock.set(1_059);
    assert_eq!(
        refusal(registry.admit(&account, faster)),
        QuotaError::Cooldown { retry_at_ms: 1_060 }
    );
    clock.set(1_060);
    claim(&registry, &account, faster)
        .finish(QuotaAnswer::Exhausted)
        .unwrap();
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Cooldown { retry_at_ms: 1_061 }
    );
}

#[test]
fn bound_evicts_only_closed_unused_accounts_and_never_forgets_open_quota() {
    let (_, registry, first, settings) = fixture();
    let settings = QuotaSettings {
        maximum_accounts: 1,
        ..settings
    };
    let second = account("https://api.openai.com/v1/images/edits", "second-key");
    let waiting = registry.admit(&first, settings).unwrap();
    assert_eq!(
        refusal(registry.admit(&second, settings)),
        QuotaError::RegistryFull
    );
    drop(waiting);
    assert!(registry.admit(&second, settings).is_ok());
    open(&registry, &second, settings);
    assert_eq!(
        refusal(registry.admit(&first, settings)),
        QuotaError::RegistryFull
    );
    assert_eq!(
        refusal(registry.admit(&second, settings)),
        QuotaError::Cooldown { retry_at_ms: 1_060 }
    );
}

#[test]
fn invalid_identity_time_and_policy_fail_closed_without_leaking_credentials() {
    let (clock, registry, account, settings) = fixture();
    assert_eq!(
        refusal(QuotaAccount::from_credential(
            "https://user:pass@example.com",
            "private-key",
            None
        )),
        QuotaError::InvalidIdentity
    );
    for invalid in [
        QuotaSettings {
            cooldown_ms: 0,
            ..settings
        },
        QuotaSettings {
            maximum_accounts: 0,
            ..settings
        },
    ] {
        assert_eq!(
            refusal(registry.admit(&account, invalid)),
            QuotaError::InvalidPolicy
        );
    }
    clock.set(0);
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::InvalidTime
    );
}

#[test]
fn operator_large_cooldown_cannot_overflow_into_early_admission() {
    let (clock, registry, account, settings) = fixture();
    let settings = QuotaSettings {
        cooldown_ms: u64::MAX,
        ..settings
    };
    open(&registry, &account, settings);
    clock.set(i64::MAX - 1);
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Cooldown {
            retry_at_ms: i64::MAX
        }
    );
}

#[test]
fn developer_summary_is_bounded_read_only_and_contains_no_account_identities() {
    let (clock, registry, account, settings) = fixture();
    assert_eq!(registry.summary().unwrap(), QuotaSummary::default());
    open(&registry, &account, settings);
    let open = QuotaSummary {
        open_accounts: 1,
        ..QuotaSummary::default()
    };
    assert_eq!(registry.summary().unwrap(), open);
    assert_eq!(registry.summary().unwrap(), open);
    assert_eq!(
        refusal(registry.admit(&account, settings)),
        QuotaError::Cooldown { retry_at_ms: 1_060 }
    );
    clock.set(1_060);
    let probe = claim(&registry, &account, settings);
    assert_eq!(
        registry.summary().unwrap(),
        QuotaSummary {
            probing_accounts: 1,
            ..QuotaSummary::default()
        }
    );
    probe.finish(QuotaAnswer::Succeeded).unwrap();
    assert_eq!(
        registry.summary().unwrap(),
        QuotaSummary {
            closed_accounts: 1,
            ..QuotaSummary::default()
        }
    );
}
