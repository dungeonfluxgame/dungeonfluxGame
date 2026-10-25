//! Bounded System One choice calls using the shared native HTTP transport.
//! Wire contract: SchemaFlux `systemone.go` and `decision_wire_contract_test.go`.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use serde_json::{Value, json};

use crate::metering::{
    AnsweredUsage, CallMeter, MeterSlot, MeteredClient, SentCall, settle_refusal, settle_status,
};
use crate::{HttpRequest, NativeHttpLimits, NativeHttpRefusal, native_http::NativeHttp};

// 800 ms and 32 KiB failed every free-roam judgement (larger generated-place context) as a
// transport error, which silenced every stranger (2026-10-24, owner: remove the limits).
const DEADLINE: Duration = Duration::from_secs(4);
/// The model every call asks for.
const MODEL: &str = "jev-latest";

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
    meter: MeterSlot,
}

impl SystemOneChoiceProvider {
    /// Configures HTTPS, or an explicitly selected literal loopback HTTP test endpoint.
    /// The fixed `/v1/systemone` path is appended to the base URL.
    /// A meter is required at construction so aliases cannot create an unmetered client.
    ///
    /// ```compile_fail
    /// use df_providers::SystemOneChoiceProvider as Choice;
    /// let _client = Choice::new("key".to_owned(), "https://example.com");
    /// ```
    pub fn new(
        credential: String,
        base_url: &str,
        meter: Arc<dyn CallMeter>,
    ) -> Result<Self, SystemOneError> {
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
                maximum_request_bytes: 256 * 1024,
                maximum_response_bytes: 64 * 1024,
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
            meter: MeterSlot::new(meter),
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
            "model": MODEL,
            "questions": {"q1": {"type": "choice", "instructions": instructions, "criteria": options}},
            "state": state,
        }))
        .map_err(|_| SystemOneError::InvalidAnswer)?;
        let body_bytes = body.len();
        let request = HttpRequest::bearer_native(
            &self.endpoint,
            &self.credential,
            "application/json".to_owned(),
            body,
        );
        let ticket = self.meter.sent(SentCall {
            client: MeteredClient::Choice,
            model: MODEL,
            text_bytes: body_bytes,
            pictures: 0,
            maximum_output_tokens: 0,
        });
        let response = match self
            .http
            .execute(request, DEADLINE, std::future::pending())
            .await
        {
            Ok(response) => response,
            Err(failure) => {
                settle_refusal(ticket, failure.reason);
                return Err(match failure.reason {
                    NativeHttpRefusal::Deadline => SystemOneError::Timeout,
                    _ => SystemOneError::Transport,
                });
            }
        };
        if response.status != 200 {
            settle_status(ticket, response.status);
            return Err(SystemOneError::Transport);
        }
        let response: Result<Value, _> = serde_json::from_slice(&response.body);
        let usage = response.as_ref().map(reported_usage).unwrap_or_default();
        let choice = response
            .map_err(|_| SystemOneError::InvalidAnswer)
            .and_then(|response| choice_of(&response, criteria));
        // A 200 is a billed call, whatever the answer's shape; an answer that is no choice is
        // billed and of no use.
        if let Some(ticket) = ticket {
            if choice.is_ok() {
                ticket.answered(usage);
            } else {
                ticket.unusable(usage);
            }
        }
        choice
    }
}

/// The zero-based option a Jev answer names, if it names one of `criteria` and nothing else.
fn choice_of(response: &Value, criteria: &[String]) -> Result<usize, SystemOneError> {
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

/// The token counts a Jev answer reports, if it carries a `usage` object (`input_tokens` or
/// `prompt_tokens`, and `output_tokens` or `completion_tokens`).
fn reported_usage(response: &Value) -> AnsweredUsage {
    let usage = response.get("usage");
    let count = |names: [&str; 2]| names.iter().find_map(|name| usage?.get(*name)?.as_u64());
    AnsweredUsage {
        input_tokens: count(["input_tokens", "prompt_tokens"]),
        output_tokens: count(["output_tokens", "completion_tokens"]),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_reply_without_a_usage_object_reports_nothing() {
        assert_eq!(
            reported_usage(&json!({"answers": {}})),
            AnsweredUsage::default()
        );
    }

    #[test]
    fn either_naming_of_the_token_counts_is_read() {
        assert_eq!(
            reported_usage(&json!({"usage": {"input_tokens": 12, "output_tokens": 3}})),
            AnsweredUsage {
                input_tokens: Some(12),
                output_tokens: Some(3)
            }
        );
        assert_eq!(
            reported_usage(&json!({"usage": {"prompt_tokens": 7}})),
            AnsweredUsage {
                input_tokens: Some(7),
                output_tokens: None
            }
        );
    }
}
