use super::*;
use serde_json::Value;

fn completed_params(items_json: &str) -> Value {
    let mut text = String::from(
        "{\"threadId\":\"thread-1\",\"turn\":{\"id\":\"turn-1\",\"status\":\"completed\",\"items\":[",
    );
    text.push_str(items_json);
    text.push_str("]}}");
    serde_json::from_str(&text).unwrap()
}

#[test]
fn file_id_wins_over_an_empty_result() {
    let view = extract_turn_completed(&completed_params(
        r#"{"id":"item-1","type":"imageGeneration","status":"completed","fileId":"file-abc","result":""}"#,
    ))
    .unwrap();
    assert_eq!(view.items.len(), 1);
    assert_eq!(view.items[0].file_id.as_deref(), Some("file-abc"));
    assert!(!view.items[0].has_result);
}

#[test]
fn saved_path_only_item_parses() {
    let view = extract_turn_completed(&completed_params(
        r#"{"id":"item-2","type":"imageGeneration","status":"completed","savedPath":"/tmp/out/img.png"}"#,
    ))
    .unwrap();
    assert_eq!(
        view.items[0].saved_path.as_deref(),
        Some("/tmp/out/img.png")
    );
    assert_eq!(view.items[0].file_id, None);
}

#[test]
fn non_empty_result_is_presence_only() {
    let view = extract_turn_completed(&completed_params(
        r#"{"id":"item-3","type":"imageGeneration","status":"completed","result":"aGVsbG8="}"#,
    ))
    .unwrap();
    assert!(view.items[0].has_result);
    assert_eq!(view.items[0].result_len, 8);
}

#[test]
fn usage_limit_failure_is_typed() {
    let view = extract_turn_completed(&completed_params(
        r#"{"id":"item-4","type":"imageGeneration","status":"failed","failure":{"type":"usageLimitExceeded","limitId":"img-7"}}"#,
    ))
    .unwrap();
    assert_eq!(
        view.items[0].failure,
        Some(ItemFailure::UsageLimited {
            limit_id: Some("img-7".to_owned()),
        })
    );
}

#[test]
fn unconstrained_status_passes_through() {
    let view = extract_turn_completed(&completed_params(
        r#"{"id":"item-5","type":"imageGeneration","status":"partially_ready_later"}"#,
    ))
    .unwrap();
    assert_eq!(view.items[0].status, "partially_ready_later");
}

#[test]
fn missing_item_id_is_a_protocol_error() {
    let error = extract_turn_completed(&completed_params(
        r#"{"type":"imageGeneration","status":"completed"}"#,
    ))
    .unwrap_err();
    assert_eq!(error, ProtocolError::MissingField { field: "item.id" });
}

#[test]
fn thread_id_extracts_from_the_evidenced_shape() {
    let result: Value = serde_json::from_str(r#"{"thread":{"id":"thread-9"}}"#).unwrap();
    assert_eq!(extract_thread_id(&result).unwrap(), "thread-9");
    let missing: Value = serde_json::from_str(r#"{"thread":{}}"#).unwrap();
    assert_eq!(
        extract_thread_id(&missing).unwrap_err(),
        ProtocolError::MissingThreadId
    );
}

#[test]
fn malformed_lines_and_shapes_are_rejected() {
    assert_eq!(
        parse_line(b"not json").unwrap_err(),
        ProtocolError::InvalidJson
    );
    assert_eq!(
        parse_line(b"[1,2]").unwrap_err(),
        ProtocolError::NotAnObject
    );
    assert_eq!(
        parse_line(br#"{"jsonrpc":"2.0"}"#).unwrap_err(),
        ProtocolError::MissingField { field: "id" }
    );
}

#[test]
fn thread_start_params_carry_ephemeral_never_and_read_only_sandbox() {
    let params = thread_start_params("/tmp/scratch", Some("agent-x"));
    assert_eq!(params["cwd"], Value::from("/tmp/scratch"));
    assert_eq!(params["ephemeral"], Value::Bool(true));
    assert_eq!(params["approvalPolicy"], Value::from("never"));
    assert_eq!(params["sandbox"], Value::from("read-only"));
    assert_eq!(params["model"], Value::from("agent-x"));
}

#[test]
fn turn_start_keeps_text_first_then_ordered_images() {
    let params = turn_start_params(
        "thread-1",
        "a calm lake",
        &["/tmp/in/a.png".to_owned(), "/tmp/in/b.png".to_owned()],
    );
    let input = params["input"].as_array().unwrap();
    assert_eq!(input.len(), 3);
    assert_eq!(input[0]["type"], Value::from("text"));
    assert_eq!(input[1]["path"], Value::from("/tmp/in/a.png"));
    assert_eq!(input[2]["path"], Value::from("/tmp/in/b.png"));
}

#[test]
fn initialized_notification_carries_no_request_id() {
    let bytes = encode_notification(INITIALIZED_METHOD, serde_json::json!({}), 4096).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["method"], Value::from(INITIALIZED_METHOD));
    assert!(
        value.get("id").is_none(),
        "the client initialized notification carries no request id"
    );
    assert_eq!(bytes.last(), Some(&b'\n'));
    assert!(encode_notification(INITIALIZED_METHOD, serde_json::json!({}), 16).is_err());
}

#[test]
fn request_encoding_respects_the_message_bound() {
    let error = encode_request(1, TURN_START_METHOD, Value::Null, 16).unwrap_err();
    assert!(matches!(error, ProtocolError::MessageTooLong { .. }));
    let bytes = encode_request(1, TURN_START_METHOD, Value::Null, 4096).unwrap();
    assert_eq!(bytes.last(), Some(&b'\n'));
}

#[test]
fn input_observations_must_match_staged_order() {
    let observed = Some(vec![ObservedInput {
        path: "/tmp/in/a.png".to_owned(),
        sha256_hex: "AB".to_owned(),
    }]);
    assert!(match_inputs(observed.as_deref(), &[("/tmp/in/a.png", "ab")]).is_ok());
    assert_eq!(
        match_inputs(observed.as_deref(), &[("/tmp/in/a.png", "cd")]).unwrap_err(),
        ProtocolError::InputMismatch
    );
    assert_eq!(
        match_inputs(observed.as_deref(), &[]).unwrap_err(),
        ProtocolError::InputMismatch
    );
    assert!(match_inputs(None, &[("/tmp/in/a.png", "ab")]).is_ok());
}

#[test]
fn usage_is_unknown_when_absent_and_observed_when_present() {
    let bare: Value = serde_json::from_str(r#"{"id":"t","status":"completed"}"#).unwrap();
    assert_eq!(observe_usage(&bare), ObservedUsage::unknown());
    let with_usage: Value = serde_json::from_str(
        r#"{"id":"t","status":"completed","usage":{"input_tokens":5,"outputTokens":3}}"#,
    )
    .unwrap();
    let usage = observe_usage(&with_usage);
    assert_eq!(usage.input_tokens(), Some(5));
    assert_eq!(usage.output_tokens(), Some(3));
}

#[test]
fn initialize_result_accepts_objects_without_version_pins() {
    let ok: Value = serde_json::from_str(r#"{"codexHome":"/tmp"}"#).unwrap();
    assert!(extract_initialize_result(&ok).is_ok());
    let bad: Value = serde_json::from_str(r#"[]"#).unwrap();
    assert!(extract_initialize_result(&bad).is_err());
}

#[test]
fn turn_ids_bind_from_response_and_started_event() {
    let response: Value = serde_json::from_str(r#"{"turn":{"id":"turn-7"}}"#).unwrap();
    assert_eq!(extract_turn_id_from_start(&response).unwrap(), "turn-7");
    let flat: Value = serde_json::from_str(r#"{"id":"turn-8"}"#).unwrap();
    assert_eq!(extract_turn_id_from_start(&flat).unwrap(), "turn-8");
    let started: Value =
        serde_json::from_str(r#"{"threadId":"thread-1","turn":{"id":"turn-7"}}"#).unwrap();
    assert_eq!(
        extract_turn_started(&started).unwrap(),
        ("thread-1".to_owned(), "turn-7".to_owned())
    );
}

#[test]
fn token_usage_last_and_total_remain_separate_without_summing_twice() {
    let params: Value = serde_json::from_str(
        r#"{"threadId":"thread-1","turnId":"turn-1","tokenUsage":{"last":{"inputTokens":5,"outputTokens":3,"cachedInputTokens":1,"reasoningOutputTokens":2,"totalTokens":8},"total":{"inputTokens":100,"outputTokens":90,"cachedInputTokens":20,"reasoningOutputTokens":30,"totalTokens":190}}}"#,
    )
    .unwrap();
    let update = extract_token_usage_update(&params).unwrap();
    assert_eq!(update.usage.input_tokens(), Some(5));
    assert_eq!(update.usage.total_tokens(), Some(8));
    assert_eq!(update.usage.output_tokens(), Some(3));
    assert_eq!(update.usage.cumulative_usage().input_tokens, Some(100));
    assert_eq!(update.usage.cumulative_usage().output_tokens, Some(90));
    assert_eq!(update.usage.cumulative_usage().total_tokens, Some(190));
    let mut merged = ObservedUsage::unknown();
    merged.merge(update.usage);
    merged.merge(update.usage);
    assert_eq!(merged.input_tokens(), Some(5));
}

#[test]
fn repeated_reordered_and_missing_breakdowns_keep_whole_counter_snapshots() {
    let mut usage = ObservedUsage::unknown();
    let update = |last: Value, total: Value| {
        extract_token_usage_update(&serde_json::json!({
            "threadId":"thread-1", "turnId":"turn-1", "tokenUsage":{"last":last,"total":total}
        }))
        .expect("snapshot")
        .usage
    };
    let first = update(
        serde_json::json!({"inputTokens":5,"outputTokens":20,"totalTokens":25}),
        serde_json::json!({"inputTokens":100,"outputTokens":10,"totalTokens":110}),
    );
    let second = update(
        serde_json::json!({"inputTokens":30,"outputTokens":1,"totalTokens":31}),
        serde_json::json!({"inputTokens":80,"outputTokens":40,"totalTokens":120}),
    );
    usage.merge(first);
    usage.merge(second);
    usage.merge(second);
    usage.merge(first);
    assert_eq!(usage.input_tokens(), Some(30));
    assert_eq!(usage.output_tokens(), Some(1));
    assert_eq!(usage.cumulative_usage().input_tokens, Some(80));
    assert_eq!(usage.cumulative_usage().output_tokens, Some(40));
    usage.merge(update(
        Value::Null,
        serde_json::json!({"inputTokens":90,"totalTokens":130}),
    ));
    assert_eq!(usage.input_tokens(), Some(30));
    assert_eq!(usage.output_tokens(), Some(1));
    assert_eq!(usage.cumulative_usage().input_tokens, Some(90));
    assert_eq!(usage.cumulative_usage().output_tokens, None);
}

#[test]
fn image_instruction_keeps_operation_contract_and_user_content() {
    let text = image_instruction("generate", 8, 4, "png", "either", "/out", "a lake");
    assert!(text.contains("generate"));
    assert!(text.contains("8 by 4"));
    assert!(text.contains("png"));
    assert!(text.contains("/out"));
    assert!(text.contains("a lake"));
}

#[test]
fn wire_debug_hides_private_result_and_params() {
    let marker = "SECRET-MARKER-9f2c";
    let params = serde_json::json!({"tokenUsage": {"last": {"inputTokens": 1}}, "note": marker});
    let notification = Inbound::Notification {
        method: "thread/tokenUsageUpdated".to_owned(),
        params,
    };
    assert!(!format!("{notification:?}").contains(marker));
    let response = ResponseView {
        id: RpcId::Number(0),
        result: Some(serde_json::json!({ "secret": marker })),
        error: None,
    };
    assert!(!format!("{response:?}").contains(marker));
}
