use df_table::event::FailureCause;

use super::request::ProviderCause;

#[test]
fn every_provider_cause_key_is_its_serde_name_and_reads_back() {
    for cause in ProviderCause::ALL {
        let written = serde_json::to_string(&cause).unwrap();
        assert_eq!(written, format!("\"{}\"", cause.key()));
        assert_eq!(ProviderCause::from_key(cause.key()), Some(cause));
    }
    assert_eq!(ProviderCause::from_key("fault"), None);
}

#[test]
fn a_refusal_and_an_answer_outside_its_rule_are_never_retried() {
    assert!(ProviderCause::Unavailable.is_retryable());
    assert!(ProviderCause::TextInvalid.is_retryable());
    assert!(!ProviderCause::Refused.is_retryable());
    assert!(!ProviderCause::InvalidAnswer.is_retryable());
    assert_eq!(
        ProviderCause::Refused.failure_cause(),
        FailureCause::Refused
    );
}
