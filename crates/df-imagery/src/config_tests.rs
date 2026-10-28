use super::*;
use crate::request::Purpose;

#[test]
fn default_snapshot_is_valid() {
    ImageConfig::defaults().unwrap();
}

#[test]
fn zero_backend_concurrency_is_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.backends[BackendId::OpenAi.index()].max_concurrent_jobs = 0;
    assert_eq!(
        config.validate().unwrap_err(),
        ConfigError::InvalidField {
            field: "backend.max_concurrent_jobs",
            reason: "a zero bound refuses all work",
        }
    );
}

#[test]
fn zero_pending_bytes_are_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.scheduler.max_pending_bytes = 0;
    assert_eq!(
        config.validate().unwrap_err(),
        ConfigError::InvalidField {
            field: "scheduler.max_pending_bytes",
            reason: "a zero bound refuses all work",
        }
    );
}

#[test]
fn zero_resource_bounds_are_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.transforms.max_decoded_bytes = 0;
    assert!(config.validate().is_err());
    let mut config = ImageConfig::defaults().unwrap();
    config.transforms.max_output_bytes = 0;
    assert!(config.validate().is_err());
    let mut config = ImageConfig::defaults().unwrap();
    config.transforms.queue_capacity = 0;
    assert!(config.validate().is_err());
    let mut config = ImageConfig::defaults().unwrap();
    config.scheduler.max_pending_jobs = 0;
    assert!(config.validate().is_err());
}

#[test]
fn larger_than_default_values_are_valid_without_a_code_change() {
    let mut config = ImageConfig::defaults().unwrap();
    config.backends[BackendId::OpenAi.index()].max_concurrent_jobs = 65;
    config.scheduler.foreground_limit = 65;
    config.scheduler.max_pending_bytes = u64::MAX;
    config.transforms.max_decoded_bytes = u64::MAX;
    config.validate().unwrap();
}

#[test]
fn stage_deadline_beyond_job_deadline_is_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.scheduler.stage_deadline_secs = config.scheduler.job_deadline_secs + 1;
    assert_eq!(
        config.validate().unwrap_err(),
        ConfigError::InvalidField {
            field: "scheduler.stage_deadline_secs",
            reason: "a stage deadline cannot outlive the job deadline",
        }
    );
}

#[test]
fn zero_request_bounds_are_refused() {
    let mut config = ImageConfig::defaults().unwrap();
    config.request_limits.max_prompt_chars = 0;
    assert!(config.validate().is_err());
}

#[test]
fn default_request_limits_preserve_the_accepted_prompt_ceiling() {
    let config = ImageConfig::defaults().unwrap();
    assert_eq!(config.request_limits.max_prompt_chars, 32_000);
}

#[test]
fn blank_account_handle_is_refused() {
    assert!(AccountHandle::new("   ").is_err());
}

#[test]
fn account_handle_rejects_overlong_and_special_values() {
    let long: String = std::iter::repeat_n('a', 65).collect();
    assert!(AccountHandle::new(&long).is_err());
    assert!(AccountHandle::new("ops/key:1").is_err());
    assert_eq!(
        AccountHandle::new("clip-01_A").unwrap().as_str(),
        "clip-01_A"
    );
}

#[test]
fn every_purpose_indexes_into_its_own_gate() {
    let config = ImageConfig::defaults().unwrap();
    for index in 0..Purpose::COUNT {
        let purpose = Purpose::from_index(index).unwrap();
        assert_eq!(purpose.index(), index);
        assert!(config.purpose(purpose).enabled);
    }
    assert_eq!(Purpose::from_index(Purpose::COUNT), None);
}
