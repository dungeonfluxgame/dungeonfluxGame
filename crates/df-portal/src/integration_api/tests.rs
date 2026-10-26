use super::*;
#[test]
fn integration_request_ids_reuse_the_shared_otel_request_correlation(){
    use opentelemetry::{Context,trace::{TraceContextExt,SpanContext,TraceId,SpanId,TraceFlags,TraceState}};
    let context=Context::new().with_remote_span_context(SpanContext::new(TraceId::from_bytes([1;16]),SpanId::from_bytes([2;8]),TraceFlags::SAMPLED,true,TraceState::default()));
    let _scope=context.attach();assert_eq!(request_id(),"01".repeat(16));
}
#[test]
fn integration_cursors_are_account_endpoint_and_expiry_bound() {
    let peppers = Peppers::new(1, [7; 32], None).unwrap();
    let cursor = peppers.cursor(&[1; 16], "campaigns", TokenId([8; 16]), 100);
    assert_eq!(
        peppers.cursor_row(&[1; 16], "campaigns", &cursor, 101),
        Some(TokenId([8; 16]))
    );
    assert_eq!(
        peppers.cursor_row(&[2; 16], "campaigns", &cursor, 101),
        None
    );
    assert_eq!(
        peppers.cursor_row(&[1; 16], "play-sessions", &cursor, 101),
        None
    );
    assert_eq!(
        peppers.cursor_row(&[1; 16], "campaigns", &cursor, 86_400_100),
        None
    );
}
#[test]
fn integration_paging_and_rate_windows_reject_unbounded_input() {
    let peppers = Peppers::new(1, [7; 32], None).unwrap();
    assert!(
        Paging {
            limit: Some(101),
            cursor: None
        }
        .checked(&peppers, &[1; 16], "campaigns", 0, "req")
        .is_err()
    );
    let mut limits = Limits::default();
    for _ in 0..120 {
        assert!(limits.token([1; 16], [1; 16], 0).is_some());
    }
    assert!(!limits.token([1; 16], [1; 16], 0).is_some());
    assert!(limits.token([2; 16], [2; 16], 0).is_some());
    assert!(limits.token([1; 16], [1; 16], 60_000).is_some());
}

#[test]
fn integration_limits_hold_ten_concurrent_requests_and_six_hundred_per_account() {
    let mut limits = Limits::default();
    let mut permits = Vec::new();
    for _ in 0..10 {
        permits.push(limits.token([1; 16], [1; 16], 0).unwrap());
    }
    assert!(limits.token([1; 16], [1; 16], 0).is_none());
    drop(permits);
    assert!(limits.token([1; 16], [1; 16], 0).is_some());
    let mut limits = Limits::default();
    for token in 1..=5 {
        for _ in 0..120 {
            assert!(limits.token([token; 16], [1; 16], 0).is_some());
        }
    }
    assert!(limits.token([6; 16], [1; 16], 0).is_none());
}
