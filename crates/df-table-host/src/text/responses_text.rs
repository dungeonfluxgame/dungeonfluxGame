//! The host's one raw text egress: a `df-ai` [`TextBackend`] over OpenAI's Responses API.
//!
//! This is a private composition adapter in the sense of the text architecture: it is reached only
//! through the durable text service the host composes (`text::TextRuntime`), owns the endpoint, the
//! bearer header and the wire shapes, and nothing else in the host builds a model request. It sends
//! a request's instructions, its facts as JSON and its pictures in order (each after its label) as
//! one user turn with a strict JSON-schema answer format, and maps the exchange to a
//! [`TextOutcome`] with the same table as `df-hosted`'s support backends. It never retries or
//! resends; credentials and content never reach a log.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use df_ai::text::{
    TextBackend, TextOutcome, TextReceipt, TextRequest, TextUsage, UnavailableReason,
};

use df_providers::{HttpRequest, NativeHttp, NativeHttpRefusal, ProviderLimits};
use serde_json::{Value, json};

use super::limits::MAXIMUM_RESPONSES_OUTPUT_TOKENS;
use super::{facts_text, usage_from};

/// OpenAI's Responses endpoint.
const RESPONSES_ENDPOINT: &str = "https://api.openai.com/v1/responses";

/// The model and key a Responses backend sends with.
#[derive(Clone)]
pub struct ResponsesRoute {
    pub model: String,
    /// Never logged.
    pub key: String,
}

impl std::fmt::Debug for ResponsesRoute {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResponsesRoute")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

/// A Responses backend; without a route every request is `Unavailable(Configuration)`.
pub struct ResponsesTextBackend {
    route: Option<ResponsesRoute>,
}

impl ResponsesTextBackend {
    pub fn new(route: Option<ResponsesRoute>) -> Self {
        Self { route }
    }
}

/// The Responses body of `request`: instructions as the system text, then one user turn with the
/// facts and each picture after its label, and a strict schema for the answer.
pub fn body(request: &TextRequest, model: &str) -> Value {
    let mut content = vec![json!({
        "type": "input_text",
        "text": facts_text(request),
    })];
    for image in &request.images {
        content.push(json!({"type": "input_text", "text": format!("{}:", image.label)}));
        let image_url = format!(
            "data:{};base64,{}",
            image.media_type.mime(),
            STANDARD.encode(&image.bytes)
        );
        content.push(json!({
            "type": "input_image",
            "image_url": image_url,
            "detail": "low",
        }));
    }
    json!({
        "model": model,
        "store": false,
        "instructions": request.instructions,
        "input": [{ "role": "user", "content": content }],
        "max_output_tokens": MAXIMUM_RESPONSES_OUTPUT_TOKENS,
        "text": { "format": {
            "type": "json_schema",
            "name": request.output_name,
            "strict": true,
            "schema": request.output_schema,
        }},
    })
}

fn token_count(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer)?.as_u64()
}

/// The supplier's tokens; unreported categories stay unknown, never zero.
pub fn usage(response: &Value) -> Option<TextUsage> {
    usage_from(
        token_count(response, "/usage/total_tokens"),
        token_count(response, "/usage/input_tokens"),
        token_count(response, "/usage/output_tokens"),
        token_count(response, "/usage/input_tokens_details/cached_tokens"),
        token_count(response, "/usage/output_tokens_details/reasoning_tokens"),
    )
}

/// The completed reply's single output text, parsed as a JSON object.
pub fn answer(response: &Value) -> Option<Value> {
    if response.get("status")?.as_str()? != "completed" {
        return None;
    }
    let texts: Vec<&str> = response
        .get("output")?
        .as_array()?
        .iter()
        .filter_map(|message| message.get("content")?.as_array())
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect();
    let [text] = texts.as_slice() else {
        return None;
    };
    serde_json::from_str::<Value>(text)
        .ok()
        .filter(Value::is_object)
}

/// The exchange's outcome, with the table `df-hosted`'s support backends use: 401 or 403 is an
/// authentication refusal, any other 4xx or 5xx an invalid result, a 2xx body that is not the
/// expected answer invalid, and anything else (never served) a configuration refusal.
pub fn receipt(status: u16, body: &[u8]) -> TextReceipt {
    match status {
        401 | 403 => TextOutcome::Unavailable(UnavailableReason::Authentication).into(),
        200..=299 => match serde_json::from_slice::<Value>(body) {
            Err(_) => TextOutcome::Invalid.into(),
            Ok(response) => TextReceipt {
                outcome: answer(&response).map_or(TextOutcome::Invalid, TextOutcome::Answered),
                usage: usage(&response),
            },
        },
        400..=599 => TextOutcome::Invalid.into(),
        _ => TextOutcome::Unavailable(UnavailableReason::Configuration).into(),
    }
}

fn failure(reason: NativeHttpRefusal) -> TextOutcome {
    match reason {
        NativeHttpRefusal::Deadline => TextOutcome::Deadline,
        NativeHttpRefusal::Cancelled => TextOutcome::Unavailable(UnavailableReason::Interrupted),
        NativeHttpRefusal::InvalidLimits
        | NativeHttpRefusal::InvalidConfiguration
        | NativeHttpRefusal::InvalidEndpoint
        | NativeHttpRefusal::InvalidCredential
        | NativeHttpRefusal::ProviderDisabled
        | NativeHttpRefusal::RequestTooLarge => {
            TextOutcome::Unavailable(UnavailableReason::Configuration)
        }
        // The request may have been served: the result is unknown and is never resent.
        _ => TextOutcome::Unknown,
    }
}

impl TextBackend for ResponsesTextBackend {
    async fn complete(&self, request: &TextRequest) -> TextReceipt {
        let Some(route) = &self.route else {
            return TextOutcome::Unavailable(UnavailableReason::Configuration).into();
        };
        let Ok(bytes) = serde_json::to_vec(&body(request, &route.model)) else {
            return TextOutcome::Unavailable(UnavailableReason::Configuration).into();
        };
        let deadline = request.deadline.min(df_providers::MAXIMUM_EXCHANGE_TIME);
        let Ok(http) = NativeHttp::new(
            ProviderLimits::new(
                deadline,
                df_providers::MAXIMUM_BODY_BYTES,
                df_providers::MAXIMUM_BODY_BYTES,
            )
            .build(),
            false,
        ) else {
            return TextOutcome::Unavailable(UnavailableReason::Configuration).into();
        };
        let call = HttpRequest::bearer_native(
            RESPONSES_ENDPOINT,
            &route.key,
            "application/json".to_owned(),
            bytes,
        );
        match http.execute(call, deadline, std::future::pending()).await {
            Ok(response) => receipt(response.status, &response.body),
            Err(error) => failure(error.reason).into(),
        }
    }
}
