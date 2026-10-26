use super::*;
use serde_json::json;

#[test]
fn parses_a_gmail_message_and_decodes_its_plain_text_body() {
    let raw = json!({"id":"msg-1","threadId":"thr-1","internalDate":"1700000000000",
        "payload":{"mimeType":"multipart/alternative","headers":[
            {"name":"from","value":"Ada <ada@example.invalid>"},
            {"name":"To","value":"support@dungeonfluxdnd.com"},
            {"name":"Subject","value":"Refund help"}],
            "parts":[{"mimeType":"text/plain","body":{"data":"SGVsbG8"}}]}});
    let mail = parse_gmail_message(&raw).unwrap();
    assert_eq!(mail.provider_message_id, "msg-1");
    assert_eq!(mail.sender_address, "ada@example.invalid");
    assert_eq!(mail.sender_display_name.as_deref(), Some("Ada"));
    assert_eq!(mail.body_text, "Hello");
    assert_eq!(mail.received_at_unix_s, 1_700_000_000);
    assert!(!mail.intake_incomplete);
}

#[test]
fn missing_credentials_are_refused_by_name_only() {
    assert!(GmailCredentials::from_env_names(|_| None).is_err());
}

#[test]
fn a_different_mailbox_is_refused() {
    assert!(
        GmailCredentials::from_env_names(|name| Some(if name == ENV_MAILBOX_ADDRESS {
            "other@example.invalid".into()
        } else {
            "synthetic".into()
        }))
        .is_err()
    );
}

#[test]
fn unsupported_or_malformed_body_is_queued_as_incomplete() {
    let raw = json!({"id":"1","threadId":"1","internalDate":"1700000000000",
        "payload":{"mimeType":"text/html","body":{"data":"PGI-SGk8L2I-"}}});
    assert!(parse_gmail_message(&raw).unwrap().intake_incomplete);
    let bad = json!({"id":"1","threadId":"1","internalDate":"invalid","payload":{}});
    assert_eq!(parse_gmail_message(&bad), Err(MailSourceError::Malformed));
}

struct Script {
    replies: std::sync::Mutex<std::collections::VecDeque<Value>>,
    paths: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl GmailTransport for Script {
    async fn exchange_refresh_token(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<String, MailSourceError> {
        Ok("synthetic-access".into())
    }
    async fn get_json(&self, path: &str, _: &str) -> Result<Value, MailSourceError> {
        self.paths.lock().unwrap().push(path.into());
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(MailSourceError::Unavailable)
    }
}

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    use std::task::{Context, Poll, Waker};
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("scripted transport never waits"),
    }
}

fn resource(id: &str) -> Value {
    json!({"id":id,"threadId":"thread","internalDate":"1700000000000",
        "payload":{"mimeType":"text/plain","body":{"data":"SGVsbG8"}}})
}

#[test]
fn bootstrap_replays_from_the_original_history_boundary_and_keeps_spam_in_scope() {
    let credentials = GmailCredentials::from_env_names(|name| {
        Some(if name == ENV_MAILBOX_ADDRESS {
            "support@dungeonfluxdnd.com".into()
        } else {
            "synthetic".into()
        })
    })
    .unwrap();
    let source = GmailMailSource::new(credentials, Script {
        replies:std::sync::Mutex::new(vec![
            json!({"emailAddress":"support@dungeonfluxdnd.com","historyId":"100"}),
            json!({"messages":[{"id":"m/1"}],"nextPageToken":"page&2"}),resource("m/1"),
            json!({"messages":[{"id":"m2"}]}),resource("m2"),
            json!({"history":[{"messagesAdded":[{"message":{"id":"m3"}},{"message":{"id":"m3"}}]}],
                "historyId":"103"}),resource("m3"),
        ].into()),paths:std::sync::Mutex::new(Vec::new()),
    });
    let first = run(source.fetch_page(None, 100)).unwrap();
    let second = run(source.fetch_page(first.next_cursor.as_deref(), 100)).unwrap();
    let third = run(source.fetch_page(second.next_cursor.as_deref(), 100)).unwrap();
    assert_eq!(third.messages.len(), 1);
    let paths = source.transport.paths.lock().unwrap();
    assert!(paths[1].contains("includeSpamTrash=true"));
    assert!(paths[2].contains("m%2F1"));
    assert!(paths[3].contains("pageToken=page%262"));
    assert!(paths[5].contains("startHistoryId=100"));
    let cursor: Value = serde_json::from_str(third.next_cursor.as_deref().unwrap()).unwrap();
    assert_eq!(cursor["history_id"], "103");
    assert_eq!(cursor["snapshot"], false);
}

#[test]
fn a_plain_alternative_is_preferred_to_html_without_fetching_external_content() {
    let raw = json!({"id":"1","threadId":"1","internalDate":"1700000000000",
        "payload":{"mimeType":"multipart/alternative","parts":[
            {"mimeType":"text/html","body":{"data":"PGI-SGk8L2I-"}},
            {"mimeType":"text/plain","body":{"data":"SGVsbG8"}}]}});
    let mail = parse_gmail_message(&raw).unwrap();
    assert_eq!(mail.body_text, "Hello");
    assert!(!mail.intake_incomplete);
}

#[test]
fn text_attachments_remain_incomplete_intake_and_are_never_supplied_as_message_text() {
    let raw = json!({"id":"1","threadId":"1","internalDate":"1700000000000",
        "payload":{"mimeType":"multipart/mixed","parts":[
            {"mimeType":"text/plain","body":{"data":"SGVsbG8"}},
            {"mimeType":"text/plain","filename":"evidence.txt","body":{"data":"U0VDUkVU"}}]}});
    let mail = parse_gmail_message(&raw).unwrap();
    assert_eq!(mail.body_text, "Hello");
    assert!(mail.intake_incomplete);
    assert!(!mail.body_text.contains("SECRET"));
}
