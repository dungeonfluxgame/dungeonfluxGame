//! Gmail API mail source. Authorization is the design's internal Workspace OAuth
//! web client with an offline refresh token for the dedicated support mailbox
//! (design section 2: no domain-wide delegation by default). Credential values
//! arrive only through an injected getter keyed by environment NAME; this module
//! never logs, stores or formats them.
use serde_json::Value;

use crate::ingest::{InboundMail, MailPage, MailSource, MailSourceError, bound_body};

/// Environment names for the support mailbox. Values are host supplied; never commit them.
pub const ENV_CLIENT_ID: &str = "DF_SUPPORT_GMAIL_CLIENT_ID";
pub const ENV_CLIENT_SECRET: &str = "DF_SUPPORT_GMAIL_CLIENT_SECRET";
pub const ENV_REFRESH_TOKEN: &str = "DF_SUPPORT_GMAIL_REFRESH_TOKEN";
pub const ENV_MAILBOX_ADDRESS: &str = "DF_SUPPORT_MAILBOX_ADDRESS";

pub fn env_names() -> [&'static str; 4] {
    [
        ENV_CLIENT_ID,
        ENV_CLIENT_SECRET,
        ENV_REFRESH_TOKEN,
        ENV_MAILBOX_ADDRESS,
    ]
}

/// OAuth material resolved from environment names. Deliberately has no Debug impl.
pub struct GmailCredentials {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    pub mailbox_address: String,
}

impl GmailCredentials {
    /// Reads values by NAME through `get`. A missing name is an error naming the variable.
    pub fn from_env_names(get: impl Fn(&str) -> Option<String>) -> Result<Self, MailSourceError> {
        let take = |name: &str| {
            get(name)
                .filter(|value| !value.trim().is_empty())
                .ok_or(MailSourceError::Rejected)
        };
        let mailbox_address = take(ENV_MAILBOX_ADDRESS)?;
        if mailbox_address != "support@dungeonfluxdnd.com" {
            return Err(MailSourceError::Rejected);
        }
        Ok(Self {
            client_id: take(ENV_CLIENT_ID)?,
            client_secret: take(ENV_CLIENT_SECRET)?,
            refresh_token: take(ENV_REFRESH_TOKEN)?,
            mailbox_address,
        })
    }
}

/// Transport seam for the Gmail API and the OAuth token endpoint. Production wires
/// an HTTPS client in the hosted composition; tests use a fake. No other caller exists.
#[async_trait::async_trait]
pub trait GmailTransport: Send + Sync {
    /// Exchanges the refresh token for a short-lived access token (memory only).
    async fn exchange_refresh_token(
        &self,
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
    ) -> Result<String, MailSourceError>;
    /// GETs one Gmail API resource as JSON with the bearer token.
    async fn get_json(&self, path: &str, access_token: &str) -> Result<Value, MailSourceError>;
}

/// Gmail API adapter: lists message ids, then fetches each message once.
/// Read-only scopes only (`gmail.readonly` for ingest); it never sends (no auto-replies).
pub struct GmailMailSource<T: GmailTransport> {
    credentials: GmailCredentials,
    transport: T,
}

impl<T: GmailTransport> GmailMailSource<T> {
    pub fn new(credentials: GmailCredentials, transport: T) -> Self {
        Self {
            credentials,
            transport,
        }
    }

    pub fn mailbox_address(&self) -> &str {
        &self.credentials.mailbox_address
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    history_id: String,
    snapshot: bool,
    page_token: Option<String>,
}

#[async_trait::async_trait]
impl<T: GmailTransport> MailSource for GmailMailSource<T> {
    async fn fetch_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<MailPage, MailSourceError> {
        let limit = limit.clamp(1, crate::ingest::MAXIMUM_BATCH);
        let token = self
            .transport
            .exchange_refresh_token(
                &self.credentials.client_id,
                &self.credentials.client_secret,
                &self.credentials.refresh_token,
            )
            .await?;
        let cursor = match cursor {
            Some(raw) => {
                serde_json::from_str::<Cursor>(raw).map_err(|_| MailSourceError::Malformed)?
            }
            None => {
                let profile = self
                    .transport
                    .get_json("/gmail/v1/users/me/profile", &token)
                    .await?;
                if profile.get("emailAddress").and_then(Value::as_str)
                    != Some(self.mailbox_address())
                {
                    return Err(MailSourceError::Rejected);
                }
                Cursor {
                    history_id: required(&profile, "historyId")?.to_owned(),
                    snapshot: true,
                    page_token: None,
                }
            }
        };
        let mut path = if cursor.snapshot {
            format!("/gmail/v1/users/me/messages?includeSpamTrash=true&maxResults={limit}")
        } else {
            format!(
                "/gmail/v1/users/me/history?historyTypes=messageAdded&startHistoryId={}&maxResults=1",
                encode(&cursor.history_id)
            )
        };
        if let Some(page_token) = &cursor.page_token {
            path.push_str(&format!("&pageToken={}", encode(page_token)));
        }
        let listing = self.transport.get_json(&path, &token).await?;
        let mut ids = std::collections::BTreeSet::new();
        if cursor.snapshot {
            if let Some(entries) = listing.get("messages") {
                for entry in entries.as_array().ok_or(MailSourceError::Malformed)? {
                    ids.insert(required(entry, "id")?.to_owned());
                }
            }
        } else if let Some(histories) = listing.get("history") {
            for history in histories.as_array().ok_or(MailSourceError::Malformed)? {
                if let Some(added) = history.get("messagesAdded") {
                    for entry in added.as_array().ok_or(MailSourceError::Malformed)? {
                        let message = entry.get("message").ok_or(MailSourceError::Malformed)?;
                        ids.insert(required(message, "id")?.to_owned());
                    }
                }
            }
        }
        // Refuse oversized events without advancing; the sync failure remains visible.
        if ids.len() > limit {
            return Err(MailSourceError::Malformed);
        }
        let mut messages = Vec::with_capacity(ids.len());
        for id in ids {
            let raw = self
                .transport
                .get_json(
                    &format!("/gmail/v1/users/me/messages/{}?format=full", encode(&id)),
                    &token,
                )
                .await?;
            messages.push(parse_gmail_message(&raw)?);
        }
        let page_token = match listing.get("nextPageToken") {
            Some(value) => Some(value.as_str().ok_or(MailSourceError::Malformed)?.to_owned()),
            None => None,
        };
        let history_id = if !cursor.snapshot && page_token.is_none() {
            required(&listing, "historyId")?.to_owned()
        } else {
            cursor.history_id
        };
        let next = Cursor {
            history_id,
            snapshot: cursor.snapshot && page_token.is_some(),
            page_token,
        };
        Ok(MailPage {
            messages,
            next_cursor: Some(
                serde_json::to_string(&next).map_err(|_| MailSourceError::Malformed)?,
            ),
        })
    }
}

fn required<'a>(value: &'a Value, field: &str) -> Result<&'a str, MailSourceError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(MailSourceError::Malformed)
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Parses protocol fields without inferring identity. Unsupported body parts remain manual intake.
pub fn parse_gmail_message(raw: &Value) -> Result<InboundMail, MailSourceError> {
    let provider_message_id = required(raw, "id")?.to_owned();
    let provider_thread_id = required(raw, "threadId")?.to_owned();
    let received_at_unix_s = required(raw, "internalDate")?
        .parse::<i64>()
        .map_err(|_| MailSourceError::Malformed)?
        / 1000;
    let payload = raw.get("payload").ok_or(MailSourceError::Malformed)?;
    let header = |name: &str| -> String {
        payload
            .get("headers")
            .and_then(Value::as_array)
            .and_then(|headers| {
                headers.iter().find(|header| {
                    header
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.eq_ignore_ascii_case(name))
                })
            })
            .and_then(|header| header.get("value").and_then(Value::as_str))
            .unwrap_or("")
            .to_owned()
    };
    let from = header("From");
    let (sender_display_name, sender_address) = split_from(&from);
    let mut parts = 0;
    let (body, incomplete) = extract_text(payload, 0, &mut parts);
    let labels: Vec<String> = match raw.get("labelIds") {
        Some(value) => {
            serde_json::from_value(value.clone()).map_err(|_| MailSourceError::Malformed)?
        }
        None => Vec::new(),
    };
    let direction = if labels.iter().any(|label| label == "SENT") {
        crate::contracts::MessageDirection::Outbound
    } else {
        crate::contracts::MessageDirection::Inbound
    };
    Ok(InboundMail {
        provider_message_id,
        provider_thread_id,
        received_at_unix_s,
        intake_incomplete: incomplete
            || body.is_empty()
            || body.len() > crate::ingest::MAXIMUM_BODY_BYTES,
        sender_address,
        sender_display_name,
        recipient_addresses: split_addresses(&header("To")),
        subject: header("Subject"),
        body_text: bound_body(&body),
        labels,
        direction,
    })
}

fn split_from(from: &str) -> (Option<String>, String) {
    if let Some(open) = from.find('<')
        && let Some(close) = from[open..].find('>')
    {
        let display = from[..open].trim().trim_matches('"').to_owned();
        return (
            (!display.is_empty()).then_some(display),
            from[open + 1..open + close].trim().to_owned(),
        );
    }
    (None, from.trim().to_owned())
}

fn split_addresses(value: &str) -> Vec<String> {
    value
        .split(',')
        .take(100)
        .map(|part| split_from(part.trim()).1)
        .filter(|address| !address.is_empty())
        .collect()
}

fn extract_text(payload: &Value, depth: usize, part_count: &mut usize) -> (String, bool) {
    use base64::Engine;
    *part_count += 1;
    if depth > 16 || *part_count > 20 {
        return (String::new(), true);
    }
    if payload
        .get("filename")
        .and_then(Value::as_str)
        .is_some_and(|name| !name.is_empty())
        || payload
            .get("body")
            .and_then(|body| body.get("attachmentId"))
            .is_some()
    {
        return (String::new(), true);
    }
    if payload.get("mimeType").and_then(Value::as_str) == Some("text/plain") {
        let text = payload
            .get("body")
            .and_then(|body| body.get("data"))
            .and_then(Value::as_str)
            .filter(|data| data.len() <= 4 * crate::ingest::MAXIMUM_BODY_BYTES)
            .and_then(|data| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(data.trim_end_matches('='))
                    .ok()
            })
            .and_then(|bytes| String::from_utf8(bytes).ok());
        return match text {
            Some(text) => (text, false),
            None => (String::new(), true),
        };
    }
    let mut output = String::new();
    let mut incomplete = false;
    if let Some(parts) = payload.get("parts").and_then(Value::as_array) {
        if payload.get("mimeType").and_then(Value::as_str) == Some("multipart/alternative")
            && let Some(plain) = parts
                .iter()
                .find(|part| part.get("mimeType").and_then(Value::as_str) == Some("text/plain"))
        {
            return extract_text(plain, depth + 1, part_count);
        }
        for part in parts.iter().take(21) {
            let (text, failed) = extract_text(part, depth + 1, part_count);
            incomplete |= failed;
            if output.len().saturating_add(text.len()) > crate::ingest::MAXIMUM_BODY_BYTES {
                incomplete = true;
                break;
            }
            if !output.is_empty() && !text.is_empty() {
                output.push('\n');
            }
            output.push_str(&text);
        }
        incomplete |= parts.len() > 20;
    } else {
        incomplete = true;
    }
    (output, incomplete)
}

#[cfg(test)]
#[path = "gmail_tests.rs"]
mod tests;
