use std::sync::{Arc, Mutex};
use std::time::Duration;

use df_ai::text::{
    TextImage, TextImageType, TextOutcome, TextProfile, TextPurpose, TextRequest, UnavailableReason,
};
use df_ai::text_attempts::Submission;
use serde_json::json;

use super::responses_text::{body, receipt};
use super::{BackendChoice, TextRuntime, usage_from};

fn request() -> TextRequest {
    TextRequest {
        purpose: TextPurpose::CharacterIdentity,
        profile: TextProfile::new("responses:gpt-6-luna").unwrap(),
        instructions: "Match the people.".to_owned(),
        facts: json!({"characters": [{"name": "innkeeper"}], "candidates": 1}),
        output_name: "character_matches".to_owned(),
        output_schema: json!({"type": "object"}),
        deadline: Duration::from_secs(30),
        images: vec![TextImage {
            label: "Candidate 1".to_owned(),
            media_type: TextImageType::Jpeg,
            bytes: vec![1, 2, 3],
        }],
    }
}

#[test]
fn the_body_is_one_user_turn_with_facts_then_each_picture_after_its_label() {
    let sent = body(&request(), "gpt-6-luna");
    assert_eq!(sent["model"], "gpt-6-luna");
    assert_eq!(sent["store"], false);
    assert_eq!(sent["instructions"], "Match the people.");
    assert_eq!(sent["text"]["format"]["strict"], true);
    assert_eq!(sent["text"]["format"]["name"], "character_matches");
    let content = sent["input"][0]["content"].as_array().unwrap();
    assert!(
        content[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Facts (JSON): ")
    );
    assert_eq!(content[1]["text"], "Candidate 1:");
    assert_eq!(content[2]["type"], "input_image");
    assert_eq!(content[2]["image_url"], "data:image/jpeg;base64,AQID");
}

#[test]
fn the_outcome_table_matches_the_support_backends() {
    assert_eq!(
        receipt(401, b"{}").outcome,
        TextOutcome::Unavailable(UnavailableReason::Authentication)
    );
    assert_eq!(receipt(500, b"{}").outcome, TextOutcome::Invalid);
    assert_eq!(receipt(200, b"not json").outcome, TextOutcome::Invalid);
    assert_eq!(
        receipt(302, b"").outcome,
        TextOutcome::Unavailable(UnavailableReason::Configuration)
    );
    let completed = json!({
        "status": "completed",
        "output": [{"content": [{"type": "output_text", "text": "{\"assignments\":[]}"}]}],
        "usage": {"input_tokens": 900, "output_tokens": 40, "total_tokens": 940,
            "output_tokens_details": {"reasoning_tokens": 0}},
    });
    let answered = receipt(200, completed.to_string().as_bytes());
    assert_eq!(
        answered.outcome,
        TextOutcome::Answered(json!({"assignments": []}))
    );
    let usage = answered.usage.unwrap();
    assert_eq!((usage.total_tokens, usage.input_tokens), (940, Some(900)));
    assert_eq!(usage.cached_input_tokens, None);
    let unfinished = json!({"status": "incomplete", "output": []});
    assert_eq!(
        receipt(200, unfinished.to_string().as_bytes()).outcome,
        TextOutcome::Invalid
    );
}

#[test]
fn an_unconfigured_choice_names_the_variables_the_host_reads() {
    let silent = || Arc::new(|_line: String| {}) as Arc<dyn Fn(String) + Send + Sync>;
    let responses = TextRuntime::new(BackendChoice::Responses(None), silent());
    let codex = TextRuntime::new(BackendChoice::Codex(None), silent());

    assert_eq!(
        responses.missing(),
        Some(
            "no OpenAI key in the host's environment: \
             set OPENAI_API_KEY (or DF_OPENAI_API_KEY), or DF_TEXT_BACKEND=codex"
        )
    );
    assert_eq!(
        codex.missing(),
        Some(
            "no Codex executable: set DF_TEXT_CODEX_EXECUTABLE to the signed-in codex.exe, \
             or put codex on the path"
        )
    );
}

#[tokio::test]
async fn an_unconfigured_runtime_settles_unavailable_without_a_send_and_logs_one_line() {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&lines);
    let runtime = TextRuntime::new(
        BackendChoice::Responses(None),
        Arc::new(move |line| sink.lock().unwrap().push(line)),
    );
    assert!(
        runtime
            .missing()
            .is_some_and(|text| text.contains("OPENAI_API_KEY"))
    );
    assert_eq!(runtime.identity_profile().as_str(), "responses:gpt-6-luna");
    let key = TextRuntime::key("locate", "identify:test").unwrap();
    let submission = runtime.submit(&key, &request()).await.unwrap();
    assert!(matches!(
        submission,
        Submission::Settled {
            outcome: TextOutcome::Unavailable(UnavailableReason::Configuration),
            ..
        }
    ));
    let lines = lines.lock().unwrap();
    assert!(lines.iter().any(|line| line.starts_with("TEXT ")
        && line.contains("purpose=character_identity")
        && line.contains("scope=locate")));
    assert!(lines.iter().all(|line| !line.contains("Match the people")));
    assert!(
        lines.contains(
            &concat!(
                "TEXT started purpose=character_identity profile=responses:gpt-6-luna ",
                "scope=locate operation=identify:test deadline_ms=30000"
            )
            .to_owned()
        )
    );
    let settled = lines
        .iter()
        .find(|line| line.starts_with("TEXT unavailable "))
        .expect("the settled event is one line");
    assert!(settled.contains(" duration_ms="), "{settled}");
    assert!(!settled.contains("elapsed_ms"), "{settled}");
    assert_eq!(runtime.status().attempts_kept, 1);
    assert_eq!(runtime.status().in_flight, 0);
}

#[test]
fn every_backend_reads_usage_the_same_way() {
    let usage = usage_from(None, Some(10), Some(5), Some(2), None).unwrap();
    assert_eq!(
        (usage.total_tokens, usage.cached_input_tokens),
        (15, Some(2))
    );
    assert_eq!(
        usage_from(Some(99), Some(10), None, None, None)
            .unwrap()
            .total_tokens,
        99
    );
    assert_eq!(usage_from(None, None, None, None, None), None);
    assert_eq!(usage_from(None, Some(u64::MAX), Some(1), None, None), None);
}

#[test]
fn the_codex_prompt_carries_the_facts_and_the_order_of_the_pictures() {
    let text = super::codex_text::prompt(&request());
    assert!(text.starts_with("Match the people."));
    assert!(text.contains("Facts (JSON): {"));
    assert!(text.contains("The attached images are, in order:\n1. Candidate 1\n"));
    assert!(text.ends_with("Reply with only the JSON object."));
}

#[test]
fn a_codex_run_maps_to_one_outcome() {
    use super::codex_text::{answer, receipt};
    let events = concat!(
        r#"{"type":"turn.completed","usage":{"input_tokens":1200,"#,
        r#""cached_input_tokens":300,"output_tokens":60}}"#
    );
    let answered = receipt(true, Some("{\"assignments\":[]}"), events, "");
    assert_eq!(
        answered.outcome,
        TextOutcome::Answered(json!({"assignments": []}))
    );
    let usage = answered.usage.unwrap();
    assert_eq!(
        (usage.total_tokens, usage.cached_input_tokens),
        (1260, Some(300))
    );
    assert_eq!(answer("```json\n{\"a\":1}\n```"), Some(json!({"a": 1})));
    assert_eq!(
        receipt(true, Some("not json"), "", "").outcome,
        TextOutcome::Invalid
    );
    assert_eq!(
        receipt(false, None, "", "Error: Not logged in").outcome,
        TextOutcome::Unavailable(UnavailableReason::Authentication)
    );
    assert_eq!(
        receipt(false, None, "", "boom").outcome,
        TextOutcome::Unknown
    );
}

#[tokio::test]
async fn a_codex_backend_with_no_executable_is_unconfigured_and_sends_nothing() {
    use df_ai::text::TextBackend;
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorder = Arc::clone(&lines);
    let record: Arc<dyn Fn(String) + Send + Sync> =
        Arc::new(move |line| recorder.lock().unwrap().push(line));
    let backend = super::codex_text::CodexTextBackend::new(None, Arc::clone(&record));
    assert_eq!(
        backend.complete(&request()).await.outcome,
        TextOutcome::Unavailable(UnavailableReason::Configuration)
    );
    let work_root = tempfile::tempdir().unwrap();
    let route = super::codex_text::CodexRoute {
        executable: std::path::PathBuf::from("Z:/no/such/codex.exe"),
        work_root: work_root.path().to_path_buf(),
        model: None,
    };
    let missing = super::codex_text::CodexTextBackend::new(Some(route), record);
    assert_eq!(
        missing.complete(&request()).await.outcome,
        TextOutcome::Unavailable(UnavailableReason::Configuration)
    );
    // The spawn failure is recorded by its closed error kind, never a Debug form, and the
    // attempt's folder is removed.
    let lines = lines.lock().unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("TEXT codex_step step=spawn outcome=failed duration_ms=")
            && lines[0].ends_with(" error_kind=not_found"),
        "{lines:?}"
    );
    assert!(!lines[0].contains("NotFound"), "{lines:?}");
    assert_eq!(std::fs::read_dir(work_root.path()).unwrap().count(), 0);
}

#[test]
fn a_codex_step_line_holds_closed_keys_and_milliseconds_only() {
    use super::codex_text::{RunStep, StepCause, step_line};
    assert_eq!(
        step_line(RunStep::Wait, 1200, None),
        "TEXT codex_step step=wait outcome=ok duration_ms=1200"
    );
    assert_eq!(
        step_line(RunStep::Read, 3, Some(StepCause::PermissionDenied)),
        "TEXT codex_step step=read outcome=failed duration_ms=3 error_kind=permission_denied"
    );
    let kinds = [
        (std::io::ErrorKind::NotFound, "not_found"),
        (std::io::ErrorKind::PermissionDenied, "permission_denied"),
        (std::io::ErrorKind::TimedOut, "timed_out"),
        (std::io::ErrorKind::Interrupted, "interrupted"),
        (std::io::ErrorKind::StorageFull, "storage_full"),
        (std::io::ErrorKind::InvalidData, "invalid_data"),
        (std::io::ErrorKind::AddrInUse, "other"),
    ];
    for (kind, key) in kinds {
        assert_eq!(StepCause::from_kind(kind).key(), key);
    }
}

#[test]
fn building_a_runtime_writes_its_backend_profile_and_the_variable_it_misses() {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&lines);
    let record: Arc<dyn Fn(String) + Send + Sync> =
        Arc::new(move |line| sink.lock().unwrap().push(line));
    TextRuntime::new(BackendChoice::Responses(None), Arc::clone(&record));
    TextRuntime::new(BackendChoice::Codex(None), Arc::clone(&record));
    let route = super::responses_text::ResponsesRoute {
        model: "gpt-6-luna".to_owned(),
        key: "sk-test-never-logged".to_owned(),
    };
    TextRuntime::new(BackendChoice::Responses(Some(route)), record);

    assert_eq!(
        lines.lock().unwrap().as_slice(),
        [
            concat!(
                "TEXT runtime backend=responses profile=responses:gpt-6-luna configured=false ",
                "missing=OPENAI_API_KEY"
            ),
            "TEXT runtime backend=codex profile=codex configured=false \
             missing=DF_TEXT_CODEX_EXECUTABLE",
            "TEXT runtime backend=responses profile=responses:gpt-6-luna configured=true missing=-",
        ]
    );
}

#[test]
fn the_status_names_the_backend_and_never_its_key() {
    let silent: Arc<dyn Fn(String) + Send + Sync> = Arc::new(|_line: String| {});
    let route = super::responses_text::ResponsesRoute {
        model: "gpt-6-luna".to_owned(),
        key: "sk-test-never-shown".to_owned(),
    };
    let configured = TextRuntime::new(BackendChoice::Responses(Some(route)), Arc::clone(&silent));
    let status = configured.status();
    assert_eq!(status.backend, super::BackendKind::Responses);
    assert_eq!(status.identity_model.as_deref(), Some("gpt-6-luna"));
    assert_eq!((status.configured, status.missing), (true, None));
    assert_eq!((status.attempts_kept, status.in_flight), (0, 0));
    assert!(!format!("{status:?}").contains("sk-test"));

    let codex = TextRuntime::new(BackendChoice::Codex(None), silent).status();
    assert_eq!(codex.backend.key(), "codex");
    assert_eq!(codex.identity_model, None);
    assert_eq!(codex.missing, Some("DF_TEXT_CODEX_EXECUTABLE"));
}

#[test]
fn every_backend_kind_key_is_its_serde_name() {
    for kind in super::BackendKind::ALL {
        let serde_name = serde_json::to_value(kind).unwrap();
        assert_eq!(serde_name, json!(kind.key()));
        assert_eq!(super::BackendKind::from_key(kind.key()), Some(kind));
    }
}

#[tokio::test]
async fn a_codex_attempt_folder_that_cannot_be_removed_is_reported_not_swallowed() {
    use super::codex_text::{RunStep, finish, prepare};
    let work_root = tempfile::tempdir().unwrap();
    let attempt = prepare(work_root.path(), "{}", &[]).unwrap();
    let folder = std::fs::read_dir(work_root.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::remove_dir_all(&folder).unwrap();

    let finished = finish(attempt).await;

    assert_eq!(finished.message, None);
    let failures: Vec<_> = finished
        .failures
        .iter()
        .map(|failure| (failure.step, failure.error.kind()))
        .collect();
    assert_eq!(failures, [(RunStep::Cleanup, std::io::ErrorKind::NotFound)]);
}

/// Keeps the attempt `folder` from being removed while its files stay readable, until dropped.
#[cfg(windows)]
fn hold_folder(folder: &std::path::Path) -> impl Sized {
    use std::os::windows::fs::OpenOptionsExt;
    // Share reading and writing but not deletion; backup semantics opens a directory.
    const SHARE_READ_AND_WRITE: u32 = 0x1 | 0x2;
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ_AND_WRITE)
        .custom_flags(BACKUP_SEMANTICS)
        .open(folder)
        .expect("the folder opens")
}

/// Keeps the attempt `folder` from being removed while its files stay readable, until dropped.
#[cfg(unix)]
fn hold_folder(folder: &std::path::Path) -> impl Sized {
    use std::os::unix::fs::PermissionsExt;
    struct Held(std::path::PathBuf);
    impl Drop for Held {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }
    let parent = folder
        .parent()
        .expect("the folder has a parent")
        .to_path_buf();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555))
        .expect("the parent becomes read-only");
    Held(parent)
}

#[tokio::test]
async fn a_failed_codex_cleanup_is_reported_and_still_returns_the_runs_message() {
    use super::codex_text::{LAST_MESSAGE_FILE, RunStep, finish, prepare};
    let work_root = tempfile::tempdir().unwrap();
    let attempt = prepare(work_root.path(), "{}", &[]).unwrap();
    let folder = std::fs::read_dir(work_root.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(folder.join(LAST_MESSAGE_FILE), "{\"matches\":[]}").unwrap();
    let held = hold_folder(&folder);

    let finished = finish(attempt).await;
    drop(held);

    assert_eq!(finished.message.as_deref(), Some("{\"matches\":[]}"));
    let steps: Vec<_> = finished
        .failures
        .iter()
        .map(|failure| failure.step)
        .collect();
    assert_eq!(steps, [RunStep::Cleanup]);
    let _ = std::fs::remove_dir_all(&folder);
}

fn event(usage: Option<df_ai::text::TextUsage>, attempt: u32) -> df_ai::text::TextEvent {
    df_ai::text::TextEvent {
        purpose: TextPurpose::CharacterIdentity,
        profile: TextProfile::new("responses:gpt-6-luna").unwrap(),
        scope: Some("locate".to_owned()),
        operation: Some("identify:0123".to_owned()),
        output_name: "character_matches".to_owned(),
        kind: df_ai::text::TextEventKind::Answered,
        elapsed: Duration::from_millis(1500),
        usage,
        attempt,
    }
}

#[test]
fn an_event_line_carries_its_attempt_and_the_tokens_its_call_reported() {
    let usage = usage_from(None, Some(900), Some(40), Some(300), None);
    assert_eq!(
        super::event_line(&event(usage, 2)),
        concat!(
            "TEXT answered purpose=character_identity profile=responses:gpt-6-luna ",
            "output=character_matches duration_ms=1500 attempt=2 tokens_in=900 tokens_out=40 ",
            "tokens_cached=300 scope=locate operation=identify:0123"
        )
    );
}

#[test]
fn an_event_line_without_usage_writes_dashes_never_zero_tokens() {
    let line = super::event_line(&event(None, 1));
    assert!(
        line.contains(" attempt=1 tokens_in=- tokens_out=- tokens_cached=- "),
        "{line}"
    );
    assert!(!line.contains("elapsed_ms"), "{line}");
}

#[test]
fn the_responses_exchange_is_named_for_its_provider_line() {
    let label = super::responses_text::EXCHANGE_OPERATION;
    assert!(
        !label.is_empty()
            && label
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_')),
        "{label}"
    );
}
