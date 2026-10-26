//! Mail-source port, classification draft contract and ingest bounds for the support inbox.
//! Meaning is judged by the model under `classification_schema`; this module only
//! validates the closed record. It never classifies by keyword or string matching.
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::Urgency;

/// Durable ownership token and opaque continuation for one mailbox slice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailboxLease {
    pub fence: i64,
    pub cursor: Option<String>,
}

/// Hard bound on provider messages fetched in one ingest slice (design section 3.1).
pub const MAXIMUM_BATCH: usize = 100;
/// Hard bound in UTF-8 bytes on decoded body text retained for one message (design section 3.2).
pub const MAXIMUM_BODY_BYTES: usize = 128 * 1024;
/// Provisional category-confidence floor; below it the ticket needs human review (design 5).
pub const REVIEW_CONFIDENCE: f64 = 0.8;

/// One inbound message as delivered by a mail source, before queue persistence.
/// `provider_message_id` is the idempotency key; the RFC Message-ID is never used for dedupe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundMail {
    pub provider_message_id: String,
    pub provider_thread_id: String,
    pub received_at_unix_s: i64,
    pub sender_address: String,
    pub sender_display_name: Option<String>,
    pub recipient_addresses: Vec<String>,
    pub subject: String,
    pub body_text: String,
    pub intake_incomplete: bool,
    pub direction: crate::contracts::MessageDirection,
    pub labels: Vec<String>,
}

/// A bounded page from a mail source. `next_cursor` is opaque and only advances
/// after every message in the page is durably represented in the queue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailPage {
    pub messages: Vec<InboundMail>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MailSourceError {
    #[error("mail source is unavailable; the batch may be retried")]
    Unavailable,
    #[error("mail source rejected the request")]
    Rejected,
    #[error("mail source returned malformed data")]
    Malformed,
    #[error("mail history expired; bootstrap the admitted mailbox again")]
    CursorExpired,
    #[error("mail source rate limited the request")]
    RateLimited { retry_after_s: u64 },
}

/// Typed mail-source port. Implementations fetch; they never send or mutate the mailbox.
#[async_trait::async_trait]
pub trait MailSource: Send + Sync {
    async fn fetch_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<MailPage, MailSourceError>;
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum JudgeError {
    #[error("classification judge is unavailable; queue for manual triage")]
    Unavailable,
    #[error("classification judge timed out; queue for manual triage")]
    DeadlineExceeded,
    #[error("classification judge returned unusable output")]
    Unusable,
    #[error("model metering is unavailable")]
    MeterUnavailable,
}

/// Raw-output classification port. The judge returns model text; the ingest worker
/// parses and validates it. Fakes return canned text, including malformed text.
#[async_trait::async_trait]
pub trait ClassificationJudge: Send + Sync {
    async fn judge(&self, subject: &str, body_text: &str) -> Result<String, JudgeError>;

    /// Native workers supply durable platform-meter evidence before any paid dispatch.
    async fn judge_metered(
        &self,
        subject: &str,
        body_text: &str,
        _meter: &dyn crate::ports::Meter,
        _basis: crate::contracts::MeteredCall,
    ) -> Result<String, JudgeError> {
        self.judge(subject, body_text).await
    }
}

/// Coarse triage category for the ingest queue (design section 5, brief-scoped set).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportCategory {
    Billing,
    Bug,
    Legal,
    Account,
    Other,
}

/// LLM-drafted suggested next action. A draft only; operators approve or reject it.
/// No variant executes an effect and none sends a reply.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SuggestedAction {
    RefundReview {
        summary: String,
    },
    BugIssueDraft {
        title: String,
        summary: String,
        redacted_repro: String,
        expected: String,
        observed: String,
        build_ref: Option<String>,
    },
    LegalRoute {
        summary: String,
    },
    AccountRecovery {
        summary: String,
    },
    ReplyDraft {
        body_text: String,
    },
    ManualReview {
        summary: String,
    },
}

/// Closed model output for one ingested message, validated before storage.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClassificationDraft {
    pub category: SupportCategory,
    pub urgency: Urgency,
    pub confidence: f64,
    pub suggested_action: Option<SuggestedAction>,
}

/// Closed failure reasons for model output; no untrusted text is included.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DraftError {
    #[error("model output does not match the schema")]
    Schema,
    #[error("confidence is outside zero through one")]
    Confidence,
    #[error("suggested action text is empty or oversized")]
    ActionText,
}

impl ClassificationDraft {
    pub fn parse(raw: &str) -> Result<Self, DraftError> {
        let value: serde_json::Value = serde_json::from_str(raw).map_err(|_| DraftError::Schema)?;
        if value.get("suggested_action").is_none() {
            return Err(DraftError::Schema);
        }
        let action = value.get("suggested_action");
        if action
            .and_then(|action| action.get("kind"))
            .and_then(serde_json::Value::as_str)
            == Some("bug_issue_draft")
            && action.and_then(|action| action.get("build_ref")).is_none()
        {
            return Err(DraftError::Schema);
        }
        let draft: Self = serde_json::from_str(raw).map_err(|_| DraftError::Schema)?;
        draft.validate()?;
        Ok(draft)
    }

    pub fn validate(&self) -> Result<(), DraftError> {
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            return Err(DraftError::Confidence);
        }
        let texts: Vec<&str> = match &self.suggested_action {
            None => vec![],
            Some(action) => match action {
                SuggestedAction::RefundReview { summary }
                | SuggestedAction::LegalRoute { summary }
                | SuggestedAction::AccountRecovery { summary }
                | SuggestedAction::ManualReview { summary } => vec![summary.as_str()],
                SuggestedAction::BugIssueDraft {
                    title,
                    summary,
                    redacted_repro,
                    expected,
                    observed,
                    build_ref,
                } => {
                    let mut texts = vec![
                        title.as_str(),
                        summary.as_str(),
                        redacted_repro.as_str(),
                        expected.as_str(),
                        observed.as_str(),
                    ];
                    if let Some(build_ref) = build_ref {
                        texts.push(build_ref);
                    }
                    texts
                }
                SuggestedAction::ReplyDraft { body_text } => vec![body_text.as_str()],
            },
        };
        if texts
            .iter()
            .any(|text| text.is_empty() || text.len() > 4000)
        {
            return Err(DraftError::ActionText);
        }
        Ok(())
    }

    /// Below the provisional floor the classification is stored but flagged for review.
    pub fn needs_review(&self) -> bool {
        self.confidence < REVIEW_CONFIDENCE
            || self.category == SupportCategory::Other
            || self.urgency == Urgency::Undetermined
    }
}

/// Exact union: every suggested action carries only its variant's required fields.
pub fn classification_schema() -> serde_json::Value {
    use serde_json::json;
    let text = json!({"type":"string","minLength":1,"maxLength":4000});
    let variants: Vec<_> = [
        ("refund_review", vec!["summary"]),
        (
            "bug_issue_draft",
            vec![
                "title",
                "summary",
                "redacted_repro",
                "expected",
                "observed",
                "build_ref",
            ],
        ),
        ("legal_route", vec!["summary"]),
        ("account_recovery", vec!["summary"]),
        ("reply_draft", vec!["body_text"]),
        ("manual_review", vec!["summary"]),
    ]
    .into_iter()
    .map(|(kind, fields)| {
        let mut properties = serde_json::Map::new();
        properties.insert("kind".into(), json!({"enum":[kind]}));
        let mut required = vec!["kind"];
        for field in fields {
            properties.insert(
                field.into(),
                if field == "build_ref" {
                    json!({"type":["string","null"],"maxLength":4000})
                } else {
                    text.clone()
                },
            );
            required.push(field);
        }
        json!({"type":"object","additionalProperties":false,
            "required":required,"properties":properties})
    })
    .chain(std::iter::once(json!({"type":"null"})))
    .collect();
    json!({"title":"dungeonflux.support-ingest-classification/1","type":"object",
        "additionalProperties":false,"required":["category","urgency","confidence","suggested_action"],
        "properties":{"category":{"enum":["billing","bug","legal","account","other"]},
        "urgency":{"enum":["critical","high","normal","low","undetermined"]},
        "confidence":{"type":"number","minimum":0,"maximum":1},
        "suggested_action":{"anyOf":variants}}})
}

/// Truncates untrusted body text to the retained bound without splitting a char boundary.
pub fn bound_body(text: &str) -> String {
    let mut end = text.len().min(MAXIMUM_BODY_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_draft_rejects_malformed_and_out_of_range_output() {
        assert!(ClassificationDraft::parse("not json").is_err());
        assert!(ClassificationDraft::parse(
            r#"{"category":"billing","urgency":"high","confidence":1.5,"suggested_action":{"kind":"manual_review","summary":"x"}}"#
        )
        .is_err());
        assert!(ClassificationDraft::parse(
            r#"{"category":"billing","urgency":"high","confidence":0.9,"suggested_action":{"kind":"teleport","summary":"x"}}"#
        )
        .is_err());
        let ok = ClassificationDraft::parse(
            r#"{"category":"bug","urgency":"normal","confidence":0.9,"suggested_action":{"kind":"bug_issue_draft","title":"Crash","summary":"It crashes","redacted_repro":"Load","expected":"Loaded","observed":"Crash","build_ref":null}}"#,
        )
        .unwrap();
        assert_eq!(ok.category, SupportCategory::Bug);
        assert!(!ok.needs_review());
    }

    #[test]
    fn retained_body_bounds_count_bytes_without_splitting_unicode() {
        let text = format!("{}{}", "a".repeat(MAXIMUM_BODY_BYTES - 1), '\u{00e9}');
        let bounded = bound_body(&text);
        assert_eq!(bounded.len(), MAXIMUM_BODY_BYTES - 1);
    }

    #[test]
    fn low_confidence_is_flagged_for_review() {
        let draft = ClassificationDraft::parse(
            r#"{"category":"other","urgency":"low","confidence":0.5,"suggested_action":{"kind":"manual_review","summary":"Unsure"}}"#,
        )
        .unwrap();
        assert!(draft.needs_review());
    }
}
