//! Bounded native HTTPS adapters. No redirects, retries, credential formatting or reply transport.
use std::{sync::Arc, time::Duration};

use df_support::{
    gmail::{GmailCredentials, GmailMailSource, GmailTransport},
    ingest::MailSourceError,
};
use serde_json::Value;

use super::{IngestConfig, State, SupportIngest};

// The model code lives in its own private module so the raw text egress has one exact home.
#[path = "support_text.rs"]
mod support_text;

use support_text::{
    Parts, ProductionJudge, SupportAttemptStore, SupportDraftBackend, SupportJudge,
    SupportJudgeBackend, SupportTextTransport, route,
};

pub(crate) type ProductionIngest =
    SupportIngest<GmailMailSource<ReqwestGmailTransport>, ProductionJudge>;

const MAILBOX_ID: &str = "mailbox-support";

pub(crate) struct ReqwestGmailTransport {
    client: reqwest::Client,
}

pub(crate) fn from_env(
    store: std::sync::Arc<df_persistence::support_store::SupportStore>,
) -> Option<ProductionIngest> {
    let get = |name: &str| std::env::var(name).ok();
    let credentials = GmailCredentials::from_env_names(get).ok()?;
    // Historical import and sending private mail text to model suppliers require owner setup.
    if get("DF_SUPPORT_IMPORT_ALL").as_deref() != Some("true") {
        return None;
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    let models_approved = get("DF_SUPPORT_MODELS_APPROVED").as_deref() == Some("true");
    let (judge, draft) = if models_approved {
        (
            route(&get, "DF_SUPPORT_JEV"),
            route(&get, "DF_SUPPORT_DRAFT"),
        )
    } else {
        (None, None)
    };
    let address = credentials.mailbox_address.clone();
    let config = crate::config::Config::from_env().ok()?;
    let attempts = Arc::new(SupportAttemptStore::choose(
        &config.database,
        &config.transport,
    ));
    announce_attempt_store(&attempts);
    let (judge_model, draft_model) = (
        judge.as_ref().map(|route| route.model.clone()),
        draft.as_ref().map(|route| route.model.clone()),
    );
    let text = SupportTextTransport::new(Parts {
        mailbox: MAILBOX_ID.to_owned(),
        judge: SupportJudgeBackend::new(client.clone(), judge),
        draft: SupportDraftBackend::new(client.clone(), draft),
        store: attempts,
        observer: Arc::new(
            crate::telemetry::diagnostic_sink()
                .map(|sink| Arc::new(df_ai::text_telemetry::TextTelemetry::new(sink))),
        ),
        judge_model,
        draft_model,
    });
    let Ok(text) = text else {
        df_observe::record_audit_line(
            "hosted.support_text_setup_failed",
            "HOSTED support_text setup_failed".to_owned(),
        );
        return None;
    };
    let state = State::Connect(Box::new(crate::database::Connector {
        config: config.database,
        transport: config.transport,
    }));
    Some(SupportIngest::new(
        GmailMailSource::new(
            credentials,
            ReqwestGmailTransport {
                client: client.clone(),
            },
        ),
        SupportJudge::new(text),
        store,
        MAILBOX_ID.into(),
        address,
        IngestConfig::default(),
        state,
    ))
}

/// Says which attempt store this run has. The event name carries the answer because hosted audit
/// payloads are redacted; no connection string is ever printed.
fn announce_attempt_store(store: &SupportAttemptStore) {
    match store.fallback_reason() {
        None => df_observe::record_audit_line(
            "hosted.support_text_postgres",
            format!("HOSTED support_text store={} observer=none", store.label()),
        ),
        Some(reason) => df_observe::record_audit_line(
            "hosted.support_text_memory",
            format!(
                "HOSTED support_text store={} reason={reason} observer=none",
                store.label()
            ),
        ),
    }
}

async fn read_json(mut response: reqwest::Response, maximum_bytes: usize) -> Result<Value, ()> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err(());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len().saturating_add(chunk.len()) > maximum_bytes {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}

fn mail_status(response: &reqwest::Response, history: bool) -> Result<(), MailSourceError> {
    match response.status().as_u16() {
        200..=299 => Ok(()),
        401 | 403 | 400 => Err(MailSourceError::Rejected),
        404 if history => Err(MailSourceError::CursorExpired),
        429 => Err(MailSourceError::RateLimited {
            retry_after_s: response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok())
                .unwrap_or(300),
        }),
        _ => Err(MailSourceError::Unavailable),
    }
}

#[async_trait::async_trait]
impl GmailTransport for ReqwestGmailTransport {
    async fn exchange_refresh_token(
        &self,
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
    ) -> Result<String, MailSourceError> {
        let response = self
            .client
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|_| MailSourceError::Unavailable)?;
        mail_status(&response, false)?;
        read_json(response, 16 * 1024)
            .await
            .map_err(|_| MailSourceError::Malformed)?
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .map(str::to_owned)
            .ok_or(MailSourceError::Malformed)
    }

    async fn get_json(&self, path: &str, access_token: &str) -> Result<Value, MailSourceError> {
        if !path.starts_with("/gmail/v1/users/me/") {
            return Err(MailSourceError::Rejected);
        }
        let response = self
            .client
            .get(format!("https://gmail.googleapis.com{path}"))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| MailSourceError::Unavailable)?;
        mail_status(&response, path.starts_with("/gmail/v1/users/me/history?"))?;
        read_json(response, 35 * 1024 * 1024)
            .await
            .map_err(|_| MailSourceError::Malformed)
    }
}
