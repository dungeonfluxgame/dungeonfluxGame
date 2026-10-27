//! Text embeddings for the story memory: one bounded `POST /v1/embeddings` per batch through the
//! shared native transport (no redirects, no proxy, no resend; the credential, the texts and the
//! vectors are never logged or placed in errors). The model is the caller's: the story memory
//! stores 1536-dimension vectors, so only a model that can return that width is accepted, and the
//! request always asks for it.

use std::{fmt, net::SocketAddr, time::Duration};

use serde_json::{Value, json};

use crate::native_http::NativeHttp;
use crate::{HttpRequest, NativeHttpLimits, NativeHttpRefusal};

const LIVE_ENDPOINT: &str = "https://api.openai.com/v1/embeddings";
/// The width of every vector this adapter returns.
pub const EMBEDDING_DIMENSIONS: usize = 1536;
/// The default model.
pub const EMBEDDING_MODEL_SMALL: &str = "text-embedding-3-small";
/// The most texts of one call.
pub const EMBEDDING_MAXIMUM_INPUTS: usize = 64;
/// The longest text of one call, in characters (about 2000 tokens, far under the model's cap).
pub const EMBEDDING_MAXIMUM_INPUT_CHARACTERS: usize = 8000;
const MAXIMUM_REQUEST_BYTES: usize = 1024 * 1024;
const MAXIMUM_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// The models that take a `dimensions` argument and so can return 1536 values.
const MODELS: [&str; 2] = ["text-embedding-3-small", "text-embedding-3-large"];

/// Where the adapter sends calls.
pub struct EmbeddingConfig {
    fixture: Option<SocketAddr>,
}

impl EmbeddingConfig {
    pub fn live() -> Self {
        Self { fixture: None }
    }

    /// A literal loopback socket for tests.
    pub fn loopback_fixture(address: SocketAddr) -> Result<Self, EmbeddingError> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(EmbeddingError::InvalidRequest);
        }
        Ok(Self {
            fixture: Some(address),
        })
    }
}

/// Why no vectors came back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmbeddingError {
    /// The request failed local validation and nothing was sent.
    InvalidRequest,
    Unauthorized,
    RateLimited,
    Provider(u16),
    Transport(NativeHttpRefusal),
    /// The answer was not one finite vector of the right width per text.
    Output,
}

impl EmbeddingError {
    /// Whether the request left this machine (and so may have been billed).
    pub const fn was_sent(self) -> bool {
        !matches!(
            self,
            Self::InvalidRequest
                | Self::Transport(
                    NativeHttpRefusal::InvalidEndpoint | NativeHttpRefusal::ProviderDisabled
                )
        )
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Unauthorized => "unauthorized",
            Self::RateLimited => "rate_limited",
            Self::Provider(_) => "provider_error",
            Self::Transport(_) => "transport",
            Self::Output => "bad_output",
        }
    }
}

/// One answered batch.
#[derive(Clone, PartialEq)]
pub struct EmbeddingBatch {
    /// One vector per input, in input order.
    pub vectors: Vec<Vec<f32>>,
    /// The supplier-reported input tokens, when it reported them.
    pub prompt_tokens: Option<u64>,
    /// The model identifier the supplier answered with.
    pub model: String,
}

impl fmt::Debug for EmbeddingBatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingBatch")
            .field("vectors", &self.vectors.len())
            .field("prompt_tokens", &self.prompt_tokens)
            .finish_non_exhaustive()
    }
}

pub struct NativeEmbeddingProvider {
    endpoint: String,
    credential: String,
    model: String,
    http: NativeHttp,
}

impl fmt::Debug for NativeEmbeddingProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeEmbeddingProvider")
            .finish_non_exhaustive()
    }
}

impl NativeEmbeddingProvider {
    /// `total_timeout` bounds each call, including the response body.
    pub fn new(
        config: EmbeddingConfig,
        credential: &str,
        model: &str,
        total_timeout: Duration,
    ) -> Result<Self, EmbeddingError> {
        if credential.is_empty()
            || credential.len() > 8192
            || !credential.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
            || !MODELS.contains(&model)
        {
            return Err(EmbeddingError::InvalidRequest);
        }
        let limits = NativeHttpLimits {
            connect_timeout: Duration::from_secs(5).min(total_timeout),
            read_timeout: total_timeout,
            total_timeout,
            maximum_request_bytes: MAXIMUM_REQUEST_BYTES,
            maximum_response_bytes: MAXIMUM_RESPONSE_BYTES,
            maximum_header_bytes: 16 * 1024,
            maximum_chunks: 8192,
        };
        let http = NativeHttp::new(limits, config.fixture.is_some())
            .map_err(|_| EmbeddingError::InvalidRequest)?;
        Ok(Self {
            endpoint: config.fixture.map_or_else(
                || LIVE_ENDPOINT.to_owned(),
                |address| format!("http://{address}/v1/embeddings"),
            ),
            credential: credential.to_owned(),
            model: model.to_owned(),
            http,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// The vectors of `inputs`, in order. One call, never retried.
    pub async fn embed(
        &self,
        inputs: &[String],
        deadline: Duration,
    ) -> Result<EmbeddingBatch, EmbeddingError> {
        let body = request_body(&self.model, inputs)?;
        let request = HttpRequest::bearer_native(
            &self.endpoint,
            &self.credential,
            "application/json".to_owned(),
            body,
        );
        let response = self
            .http
            .execute(request, deadline, std::future::pending())
            .await
            .map_err(|failure| EmbeddingError::Transport(failure.reason))?;
        match response.status {
            200 => {}
            401 | 403 => return Err(EmbeddingError::Unauthorized),
            429 => return Err(EmbeddingError::RateLimited),
            status => return Err(EmbeddingError::Provider(status)),
        }
        parse_response(&response.body, inputs.len())
    }
}

fn request_body(model: &str, inputs: &[String]) -> Result<Vec<u8>, EmbeddingError> {
    if inputs.is_empty()
        || inputs.len() > EMBEDDING_MAXIMUM_INPUTS
        || inputs.iter().any(|text| {
            text.trim().is_empty() || text.chars().count() > EMBEDDING_MAXIMUM_INPUT_CHARACTERS
        })
    {
        return Err(EmbeddingError::InvalidRequest);
    }
    serde_json::to_vec(&json!({
        "model": model,
        "input": inputs,
        "dimensions": EMBEDDING_DIMENSIONS,
        "encoding_format": "float",
    }))
    .map_err(|_| EmbeddingError::InvalidRequest)
}

fn parse_response(body: &[u8], expected: usize) -> Result<EmbeddingBatch, EmbeddingError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| EmbeddingError::Output)?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .filter(|data| data.len() == expected)
        .ok_or(EmbeddingError::Output)?;
    let mut vectors: Vec<Option<Vec<f32>>> = vec![None; expected];
    for item in data {
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < expected)
            .ok_or(EmbeddingError::Output)?;
        let numbers = item
            .get("embedding")
            .and_then(Value::as_array)
            .filter(|numbers| numbers.len() == EMBEDDING_DIMENSIONS)
            .ok_or(EmbeddingError::Output)?;
        let mut vector = Vec::with_capacity(EMBEDDING_DIMENSIONS);
        for number in numbers {
            let number = number.as_f64().ok_or(EmbeddingError::Output)?;
            if !number.is_finite() {
                return Err(EmbeddingError::Output);
            }
            vector.push(number as f32);
        }
        if vectors[index].replace(vector).is_some() {
            return Err(EmbeddingError::Output);
        }
    }
    Ok(EmbeddingBatch {
        vectors: vectors
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(EmbeddingError::Output)?,
        prompt_tokens: value
            .get("usage")
            .and_then(|usage| {
                usage
                    .get("prompt_tokens")
                    .or_else(|| usage.get("total_tokens"))
            })
            .and_then(Value::as_u64),
        model: value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(64)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_provider_is_not_sent() {
        assert!(!EmbeddingError::Transport(NativeHttpRefusal::ProviderDisabled).was_sent());
        assert!(EmbeddingError::Transport(NativeHttpRefusal::Deadline).was_sent());
    }

    fn answer(indexes: &[usize], width: usize) -> Vec<u8> {
        let data: Vec<Value> = indexes
            .iter()
            .map(|index| json!({"index": index, "embedding": vec![0.25_f64; width]}))
            .collect();
        serde_json::to_vec(&json!({
            "data": data,
            "model": "text-embedding-3-small",
            "usage": {"prompt_tokens": 17, "total_tokens": 17},
        }))
        .unwrap()
    }

    #[test]
    fn the_request_names_the_model_the_width_and_the_texts() {
        let body = request_body(
            EMBEDDING_MODEL_SMALL,
            &["Marek owns the warehouse.".to_owned()],
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["model"], EMBEDDING_MODEL_SMALL);
        assert_eq!(value["dimensions"], 1536);
        assert_eq!(value["input"][0], "Marek owns the warehouse.");
    }

    #[test]
    fn empty_oversized_and_blank_inputs_are_refused_before_anything_is_sent() {
        assert_eq!(request_body("m", &[]), Err(EmbeddingError::InvalidRequest));
        assert_eq!(
            request_body("m", &["  ".to_owned()]),
            Err(EmbeddingError::InvalidRequest)
        );
        assert_eq!(
            request_body("m", &["x".repeat(EMBEDDING_MAXIMUM_INPUT_CHARACTERS + 1)]),
            Err(EmbeddingError::InvalidRequest)
        );
        let many = vec!["x".to_owned(); EMBEDDING_MAXIMUM_INPUTS + 1];
        assert_eq!(
            request_body("m", &many),
            Err(EmbeddingError::InvalidRequest)
        );
        assert!(!EmbeddingError::InvalidRequest.was_sent());
        assert!(EmbeddingError::Provider(500).was_sent());
    }

    #[test]
    fn an_answer_is_put_back_in_input_order_with_its_reported_tokens() {
        let batch = parse_response(&answer(&[1, 0], EMBEDDING_DIMENSIONS), 2).unwrap();
        assert_eq!(batch.vectors.len(), 2);
        assert_eq!(batch.prompt_tokens, Some(17));
        assert_eq!(batch.model, "text-embedding-3-small");
    }

    #[test]
    fn a_short_repeated_or_missing_vector_is_refused() {
        assert_eq!(
            parse_response(&answer(&[0], 3), 1).unwrap_err(),
            EmbeddingError::Output
        );
        assert_eq!(
            parse_response(&answer(&[0, 0], EMBEDDING_DIMENSIONS), 2).unwrap_err(),
            EmbeddingError::Output
        );
        assert_eq!(
            parse_response(&answer(&[0], EMBEDDING_DIMENSIONS), 2).unwrap_err(),
            EmbeddingError::Output
        );
        assert_eq!(
            parse_response(b"not json", 1).unwrap_err(),
            EmbeddingError::Output
        );
    }

    #[test]
    fn only_models_that_can_return_the_stored_width_are_accepted() {
        let config = || EmbeddingConfig::live();
        let second = Duration::from_secs(1);
        assert!(
            NativeEmbeddingProvider::new(config(), "key", "text-embedding-ada-002", second)
                .is_err()
        );
        assert!(NativeEmbeddingProvider::new(config(), "", EMBEDDING_MODEL_SMALL, second).is_err());
        assert!(
            NativeEmbeddingProvider::new(config(), "key", EMBEDDING_MODEL_SMALL, second).is_ok()
        );
    }
}
