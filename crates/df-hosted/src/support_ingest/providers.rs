//! Bounded native HTTPS adapters. No redirects, retries, credential formatting or reply transport.
use std::time::Duration;

use df_support::{
    classification::ModelTransport,
    gmail::{GmailCredentials, GmailMailSource, GmailTransport},
    ingest::{JudgeError, MailSourceError},
};
use serde_json::{Value, json};

use super::{IngestConfig, State, SupportIngest};

pub(crate) type ProductionIngest = SupportIngest<
    GmailMailSource<ReqwestGmailTransport>,
    df_support::classification::JevJudge<ReqwestModelTransport>,
>;

pub(crate) struct ReqwestGmailTransport {
    client: reqwest::Client,
}

pub(crate) struct ReqwestModelTransport {
    client: reqwest::Client,
    judge: Option<ModelRoute>,
    draft: Option<ModelRoute>,
}

struct ModelRoute {
    endpoint: String,
    key: String,
    model: String,
}

fn route(get: &impl Fn(&str) -> Option<String>, prefix: &str) -> Option<ModelRoute> {
    let key = get(&format!("{prefix}_API_KEY"))?;
    let base = get(&format!("{prefix}_BASE_URL"))?;
    let model = get(&format!("{prefix}_MODEL"))?;
    let url = reqwest::Url::parse(&base).ok()?;
    if key.is_empty()
        || model.is_empty()
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(ModelRoute {
        endpoint: base.trim_end_matches('/').to_owned(),
        key,
        model,
    })
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
        df_support::classification::JevJudge::new(ReqwestModelTransport {
            client,
            judge,
            draft,
        }),
        store,
        "mailbox-support".into(),
        address,
        IngestConfig::default(),
        state,
    ))
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

#[async_trait::async_trait]
impl ModelTransport for ReqwestModelTransport {
    fn model_name(&self, is_draft: bool) -> &str {
        let route = if is_draft { &self.draft } else { &self.judge };
        route
            .as_ref()
            .map(|route| route.model.as_str())
            .unwrap_or("unconfigured")
    }
    async fn judge(&self, mut request: Value) -> Result<Value, JudgeError> {
        let route = self.judge.as_ref().ok_or(JudgeError::Unavailable)?;
        request["model"] = json!(route.model);
        let response = self
            .client
            .post(format!("{}/v1/systemone", route.endpoint))
            .timeout(Duration::from_secs(4))
            .bearer_auth(&route.key)
            .json(&request)
            .send()
            .await
            .map_err(|_| JudgeError::Unavailable)?;
        if !response.status().is_success() {
            return Err(JudgeError::Unavailable);
        }
        read_json(response, 64 * 1024)
            .await
            .map_err(|_| JudgeError::Unusable)
    }

    async fn draft(&self, request: Value) -> Result<Value, JudgeError> {
        let route = self.draft.as_ref().ok_or(JudgeError::Unavailable)?;
        let response = self.client.post(format!("{}/v1/responses",route.endpoint))
            .bearer_auth(&route.key).json(&json!({"model":route.model,"store":false,
                "instructions":request["instructions"],"input":json!({"state":request["state"],"classification":request["classification"],
                    "concerns":request["concerns"]}).to_string(),
                "max_output_tokens":4096,"text":{"format":{"type":"json_schema",
                    "name":"support_action_draft","strict":true,"schema":request["schema"]}}}))
            .send().await.map_err(|_| JudgeError::Unavailable)?;
        if !response.status().is_success() {
            return Err(JudgeError::Unavailable);
        }
        let response = read_json(response, 64 * 1024)
            .await
            .map_err(|_| JudgeError::Unusable)?;
        if response.get("status").and_then(Value::as_str) != Some("completed") {
            return Err(JudgeError::Unusable);
        }
        let output = response
            .get("output")
            .and_then(Value::as_array)
            .ok_or(JudgeError::Unusable)?;
        let texts: Vec<_> = output
            .iter()
            .filter_map(|message| message.get("content")?.as_array())
            .flatten()
            .filter(|content| content.get("type").and_then(Value::as_str) == Some("output_text"))
            .filter_map(|content| content.get("text").and_then(Value::as_str))
            .collect();
        let [text] = texts.as_slice() else {
            return Err(JudgeError::Unusable);
        };
        let record: Value = serde_json::from_str(text).map_err(|_| JudgeError::Unusable)?;
        Ok(json!({"record":record,"usage":response.get("usage")}))
    }
}
