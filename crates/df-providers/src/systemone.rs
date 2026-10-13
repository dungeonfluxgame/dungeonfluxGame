//! Bounded System One choice calls using the shared native HTTP transport.
//! Wire contract: SchemaFlux `systemone.go` and `decision_wire_contract_test.go`.

use std::{collections::BTreeMap, time::Duration};

use serde_json::{Value, json};

use crate::{HttpRequest, NativeHttpLimits, NativeHttpRefusal, native_http::NativeHttp};

const DEADLINE: Duration = Duration::from_millis(800);

/// A closed failure vocabulary that never includes credentials, URLs or payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemOneError {
    Timeout,
    InvalidAnswer,
    UnknownOption,
    Transport,
}

/// One-attempt Jev choice client. The credential is retained privately and never formatted.
pub struct SystemOneChoiceProvider {
    http: NativeHttp,
    endpoint: String,
    credential: String,
}

impl SystemOneChoiceProvider {
    /// Configures HTTPS, or an explicitly selected literal loopback HTTP test endpoint.
    /// The fixed `/v1/systemone` path is appended to the base URL.
    pub fn new(credential: String, base_url: &str) -> Result<Self, SystemOneError> {
        if credential.is_empty()
            || credential.len() > 8192
            || !credential.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(SystemOneError::Transport);
        }
        let url = reqwest::Url::parse(base_url).map_err(|_| SystemOneError::Transport)?;
        let loopback = url.scheme() == "http"
            && url.host_str().is_some_and(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
            });
        if (url.scheme() != "https" && !loopback)
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(SystemOneError::Transport);
        }
        let http = NativeHttp::new(
            NativeHttpLimits {
                connect_timeout: DEADLINE,
                read_timeout: DEADLINE,
                total_timeout: DEADLINE,
                maximum_request_bytes: 32 * 1024,
                maximum_response_bytes: 16 * 1024,
                maximum_header_bytes: 8 * 1024,
                maximum_chunks: 64,
            },
            loopback,
        )
        .map_err(|_| SystemOneError::Transport)?;
        Ok(Self {
            http,
            endpoint: format!("{}/v1/systemone", base_url.trim_end_matches('/')),
            credential,
        })
    }

    /// Asks one choice question and returns its zero-based option position.
    /// The caller's state is data; instructions and one-sentence criteria are separate.
    /// No retry, redirect or detached task survives the bounded call or caller drop.
    pub async fn choose(
        &self,
        state: &Value,
        instructions: &str,
        criteria: &[String],
    ) -> Result<usize, SystemOneError> {
        if !(2..=255).contains(&criteria.len()) {
            return Err(SystemOneError::InvalidAnswer);
        }
        let options: BTreeMap<String, &String> = criteria
            .iter()
            .enumerate()
            .map(|(index, criterion)| (format!("o{}", index + 1), criterion))
            .collect();
        let body = serde_json::to_vec(&json!({
            "model": "jev-latest",
            "questions": {"q1": {"type": "choice", "instructions": instructions, "criteria": options}},
            "state": state,
        }))
        .map_err(|_| SystemOneError::InvalidAnswer)?;
        let request = HttpRequest::bearer_native(
            &self.endpoint,
            &self.credential,
            "application/json".to_owned(),
            body,
        );
        let response = self
            .http
            .execute(request, DEADLINE, std::future::pending())
            .await
            .map_err(|failure| match failure.reason {
                NativeHttpRefusal::Deadline => SystemOneError::Timeout,
                _ => SystemOneError::Transport,
            })?;
        if response.status != 200 {
            return Err(SystemOneError::Transport);
        }
        let response: Value =
            serde_json::from_slice(&response.body).map_err(|_| SystemOneError::InvalidAnswer)?;
        let answers = response
            .get("answers")
            .and_then(Value::as_object)
            .ok_or(SystemOneError::InvalidAnswer)?;
        let answer = answers.get("q1").ok_or(SystemOneError::InvalidAnswer)?;
        let choice = answer
            .get("choice")
            .and_then(Value::as_str)
            .ok_or(SystemOneError::InvalidAnswer)?;
        if answers.len() != 1 || answer.get("type").and_then(Value::as_str) != Some("choice") {
            return Err(SystemOneError::InvalidAnswer);
        }
        if let Some(confidence) = answer.get("confidence").filter(|value| !value.is_null()) {
            let confidence = confidence.as_f64().ok_or(SystemOneError::InvalidAnswer)?;
            if !(0.0..=1.0).contains(&confidence) {
                return Err(SystemOneError::InvalidAnswer);
            }
        }
        (0..criteria.len())
            .find(|index| choice == format!("o{}", index + 1))
            .ok_or(SystemOneError::UnknownOption)
    }
}
