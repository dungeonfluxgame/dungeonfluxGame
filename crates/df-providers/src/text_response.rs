//! Native complete-JSON OpenAI Responses profile, observed 2026-10-11.
//! Wire source: <https://developers.openai.com/api/docs/guides/structured-outputs>
//! and <https://developers.openai.com/api/reference/cli/resources/responses/methods/create>.
//! Output and supplier token counters are candidates, never invoice or publication authority.

use std::{
    fmt,
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

use df_model::checkpoint::ExecutionMode;
use df_provider_api::{
    ProviderEvent, ProviderRequestId as EventRequestId, RequestBasis, RequestError,
    RequestIdentity, RequestOwnerState, TextEvent, TextEventKind, TextRequest, TextSemantics,
};
use df_types::{Usage, UsageUnit};
use serde_json::Value;

use crate::native_http::NativeHttp;
use crate::text_schema::parse_unique_json;
use crate::{
    HttpRequest, NativeHttpLimits, NativeHttpRefusal, ProviderFailureClass, ProviderRequestId,
    TextOutputSchema, TextSchemaError,
};

const RESPONSES_ENDPOINT: &str = "https://api.openai.com/v1/responses";

/// Explicit caller-selected model and checked schema; there is no default model.
/// The native owner must qualify the model's availability, exact returned model
/// identifier, rights and spend before dispatch. This value supplies none of them.
#[derive(Clone, Eq, PartialEq)]
pub struct TextProviderConfig {
    model: String,
    schema: TextOutputSchema,
    maximum_output_tokens: u32,
    reasoning_effort: Option<ReasoningEffort>,
}

/// How much hidden reasoning the model may spend before it writes. Reasoning tokens count
/// against `max_output_tokens`, so a short creative answer should ask for little of it.
/// The request carries the setting only when the caller chose one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    fn wire_name(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

impl TextProviderConfig {
    pub fn new(
        model: &str,
        schema: TextOutputSchema,
        maximum_output_tokens: u32,
    ) -> Result<Self, TextResponseError> {
        if model.is_empty()
            || model.len() > 128
            || !model.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
            || maximum_output_tokens == 0
        {
            return Err(TextResponseError::InvalidConfiguration);
        }
        Ok(Self {
            model: model.to_owned(),
            schema,
            maximum_output_tokens,
            reasoning_effort: None,
        })
    }

    /// Requests a reasoning effort; without this call the request omits the setting and the
    /// supplier's default applies.
    #[must_use]
    pub fn with_reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort = Some(effort);
        self
    }
}

impl fmt::Debug for TextProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextProviderConfig")
            .finish_non_exhaustive()
    }
}

/// Safe failure facts. Retained usage is supplier-reported, not verified billing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextResponseError {
    UnregisteredRoute,
    InvalidConfiguration,
    InvalidCredential,
    InvalidInput,
    ModeDoesNotDispatch,
    SchemaConfigurationMismatch,
    Owner(RequestError),
    Transport(NativeHttpRefusal),
    Supplier(ProviderFailureClass),
    Contract {
        reason: TextResponseContract,
        usage: Option<Box<TextTokenUsage>>,
    },
    UsageExceeded(Box<TextTokenUsage>),
    /// Captured IDs survive failure for same-attempt lookup, never as retry or
    /// billing authority. Their own Debug implementation redacts supplier data.
    Observed {
        failure: Box<TextResponseError>,
        provider_request_id: Option<ProviderRequestId>,
        transport_request_id: Option<ProviderRequestId>,
        usage: Option<Box<TextTokenUsage>>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextResponseContract {
    Json,
    Usage,
    Identity,
    Status,
    Schema,
    Output,
    Refusal,
}

impl TextResponseError {
    pub fn reason(&self) -> &Self {
        match self {
            Self::Observed { failure, .. } => failure.reason(),
            _ => self,
        }
    }
    pub fn provider_request_id(&self) -> Option<&ProviderRequestId> {
        match self {
            Self::Observed {
                provider_request_id,
                ..
            } => provider_request_id.as_ref(),
            _ => None,
        }
    }
    pub fn transport_request_id(&self) -> Option<&ProviderRequestId> {
        match self {
            Self::Observed {
                transport_request_id,
                ..
            } => transport_request_id.as_ref(),
            _ => None,
        }
    }
    pub fn supplier_usage(&self) -> Option<TextTokenUsage> {
        match self {
            Self::Observed { usage, .. } | Self::Contract { usage, .. } => {
                usage.as_deref().copied()
            }
            Self::UsageExceeded(usage) => Some(**usage),
            _ => None,
        }
    }
    pub fn failure_class(&self) -> ProviderFailureClass {
        match self {
            Self::Owner(RequestError::Cancelled) => ProviderFailureClass::Cancelled,
            Self::Owner(RequestError::DeadlineExceeded) => ProviderFailureClass::Deadline,
            Self::Owner(_)
            | Self::Contract { .. }
            | Self::UsageExceeded(_)
            | Self::SchemaConfigurationMismatch => ProviderFailureClass::Contract,
            Self::Transport(failure) => failure.failure_class(),
            Self::Supplier(failure) => *failure,
            Self::Observed { failure, .. } => failure.failure_class(),
            _ => ProviderFailureClass::InvalidRequest,
        }
    }
}

/// Exact final supplier counters. Absence of cache-write metadata remains None.
/// These counters retain overruns and reasoning/cache categories without settling money.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextTokenUsage {
    pub input_tokens: u128,
    pub output_tokens: u128,
    pub total_tokens: u128,
    pub cached_input_tokens: u128,
    pub cache_write_tokens: Option<u128>,
    pub reasoning_output_tokens: u128,
}

/// Validated complete response provenance. Construction is private to the actual
/// decoder. It is not verified billing, rights, or permission to publish content.
#[derive(Clone)]
pub struct TextResponseObservation {
    pub(crate) identity: RequestIdentity,
    pub(crate) provider_request_id: ProviderRequestId,
    usage: TextTokenUsage,
}

impl TextResponseObservation {
    pub fn identity(&self) -> RequestIdentity {
        self.identity
    }
    pub fn provider_request_id(&self) -> &ProviderRequestId {
        &self.provider_request_id
    }
    pub fn supplier_usage(&self) -> TextTokenUsage {
        self.usage
    }
}

impl fmt::Debug for TextResponseObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextResponseObservation")
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}

/// Schema-qualified JSON after final owner validation. Access remains explicit;
/// Debug never exposes the candidate to private-data diagnostics.
pub struct ValidatedTextOutput(Value);

impl ValidatedTextOutput {
    pub fn json(&self) -> &Value {
        &self.0
    }
}

impl fmt::Debug for ValidatedTextOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ValidatedTextOutput([REDACTED])")
    }
}

/// Real native adapter. One submit future owns one request and never resends it.
pub struct NativeTextProvider {
    config: TextProviderConfig,
    credential: String,
    endpoint: String,
    http: NativeHttp,
}

impl fmt::Debug for NativeTextProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTextProvider")
            .finish_non_exhaustive()
    }
}

type TextBasis<Context> = RequestBasis<TextSemantics<Context, TextOutputSchema>>;

/// Complete supplier observation held until the same original request passes
/// current owner validation. Borrowing prevents substitution of another request.
pub struct PreparedTextResponse<'a, Context> {
    request: &'a TextRequest<Context, TextOutputSchema>,
    provider_request_id: ProviderRequestId,
    transport_request_id: Option<ProviderRequestId>,
    usage: TextTokenUsage,
    output: ValidatedTextOutput,
}

impl<Context> fmt::Debug for PreparedTextResponse<'_, Context> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedTextResponse")
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}

impl<Context: Eq> PreparedTextResponse<'_, Context> {
    pub fn observation(&self) -> TextResponseObservation {
        TextResponseObservation {
            identity: self.request.checked().binding().identity,
            provider_request_id: self.provider_request_id.clone(),
            usage: self.usage,
        }
    }
    pub fn supplier_usage(&self) -> TextTokenUsage {
        self.usage
    }
    pub fn provider_request_id(&self) -> &ProviderRequestId {
        &self.provider_request_id
    }
    pub fn transport_request_id(&self) -> Option<&str> {
        self.transport_request_id
            .as_ref()
            .map(ProviderRequestId::as_str)
    }

    /// The caller supplies a fresh owner observation at consumption. Candidate
    /// precedes Completed only after exact current basis/mode/schema validation.
    /// Replaying these events cannot authorize publication, billing or a resend.
    /// Both events describe the same final counter; they are not additive usage.
    pub fn complete(
        self,
        owner: RequestOwnerState<'_, TextBasis<Context>>,
    ) -> Result<[TextEvent<ProviderRequestId, ValidatedTextOutput>; 2], TextResponseError> {
        self.request
            .checked()
            .validate_current(owner)
            .map_err(|error| TextResponseError::Observed {
                failure: Box::new(TextResponseError::Owner(error)),
                provider_request_id: Some(self.provider_request_id.clone()),
                transport_request_id: self.transport_request_id.clone(),
                usage: Some(Box::new(self.usage)),
            })?;
        let usage = Usage::new(self.usage.total_tokens, UsageUnit::Token);
        let identity = self.request.checked().binding().identity;
        Ok([
            ProviderEvent {
                provider_request_id: EventRequestId(self.provider_request_id.clone()),
                usage,
                kind: TextEventKind::Candidate(self.output),
            },
            ProviderEvent {
                provider_request_id: EventRequestId(self.provider_request_id),
                usage,
                kind: TextEventKind::Completed { identity },
            },
        ])
    }
}

impl NativeTextProvider {
    pub(crate) fn new(
        config: TextProviderConfig,
        credential: &str,
        limits: NativeHttpLimits,
    ) -> Result<Self, TextResponseError> {
        Self::construct(
            config,
            credential,
            limits,
            RESPONSES_ENDPOINT.to_owned(),
            false,
        )
    }

    /// Scoped faithful HTTP fixture construction. Only a captured literal
    /// loopback IP/port is accepted; path is fixed, no DNS or production fallback.
    /// This deliberately separate constructor never rewrites a production factory.
    pub fn for_loopback_fixture(mut self, address: SocketAddr) -> Result<Self, TextResponseError> {
        if !matches!(address.ip(), IpAddr::V4(ip) if ip == Ipv4Addr::LOCALHOST)
            && !matches!(address.ip(), IpAddr::V6(ip) if ip == Ipv6Addr::LOCALHOST)
            || address.port() == 0
        {
            return Err(TextResponseError::Transport(
                NativeHttpRefusal::InvalidEndpoint,
            ));
        }
        self.http =
            NativeHttp::new(self.http.limits(), true).map_err(TextResponseError::Transport)?;
        self.endpoint = format!("http://{address}/v1/responses");
        Ok(self)
    }

    fn construct(
        config: TextProviderConfig,
        credential: &str,
        limits: NativeHttpLimits,
        endpoint: String,
        fixture: bool,
    ) -> Result<Self, TextResponseError> {
        if credential.is_empty()
            || credential.len() > 8192
            || !credential.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(TextResponseError::InvalidCredential);
        }
        let http = NativeHttp::new(limits, fixture).map_err(TextResponseError::Transport)?;
        Ok(Self {
            config,
            credential: credential.to_owned(),
            endpoint,
            http,
        })
    }

    /// Sends exactly once in Live mode, consumes bounded HTTP chunks through EOF,
    /// and withholds all raw text until JSON/schema/status/usage checks succeed.
    /// Cancellation must belong to this admitted job; cancelling a receipt wait
    /// is not cancellation of the accepted provider job or evidence of unused spend.
    pub async fn submit<'a, Context: Eq>(
        &self,
        request: &'a TextRequest<Context, TextOutputSchema>,
        owner: RequestOwnerState<'_, TextBasis<Context>>,
        cancellation: impl Future<Output = ()>,
    ) -> Result<PreparedTextResponse<'a, Context>, TextResponseError> {
        let elapsed = owner.elapsed;
        request
            .checked()
            .validate_current(owner)
            .map_err(TextResponseError::Owner)?;
        let binding = request.checked().binding();
        if binding.mode != ExecutionMode::Live {
            return Err(TextResponseError::ModeDoesNotDispatch);
        }
        if binding.semantic_basis.semantic.output_schema != self.config.schema {
            return Err(TextResponseError::SchemaConfigurationMismatch);
        }
        let limits = request.checked().limits();
        if limits.max_usage().unit() != UsageUnit::Token
            || u128::from(self.config.maximum_output_tokens) > limits.max_tokens()
        {
            return Err(TextResponseError::InvalidConfiguration);
        }
        let input = std::str::from_utf8(request.checked().payload())
            .map_err(|_| TextResponseError::InvalidInput)?;
        if input.trim().is_empty() {
            return Err(TextResponseError::InvalidInput);
        }
        let maximum_body_bytes = self.http.limits().maximum_request_bytes;
        if input.len() > maximum_body_bytes {
            return Err(TextResponseError::Transport(
                NativeHttpRefusal::RequestTooLarge,
            ));
        }
        let metadata = request_metadata(binding.identity);
        let mut body = BoundedRequestBody {
            bytes: Vec::new(),
            maximum_bytes: maximum_body_bytes,
        };
        let mut wire_body = serde_json::json!({
            "model":self.config.model,"input":input,"stream":false,"store":false,
            "max_output_tokens":self.config.maximum_output_tokens,
            "text":{"format":self.config.schema.format()},"metadata":metadata,
        });
        if let (Some(effort), Some(fields)) =
            (self.config.reasoning_effort, wire_body.as_object_mut())
        {
            fields.insert(
                "reasoning".to_owned(),
                serde_json::json!({"effort":effort.wire_name()}),
            );
        }
        serde_json::to_writer(&mut body, &wire_body)
            .map_err(|_| TextResponseError::Transport(NativeHttpRefusal::RequestTooLarge))?;
        let mapped = HttpRequest::responses(&self.endpoint, &self.credential, body.bytes);
        let response = self
            .http
            .execute(
                mapped,
                binding.deadline.saturating_sub(elapsed),
                cancellation,
            )
            .await
            .map_err(|failure| {
                observed(
                    TextResponseError::Transport(failure.reason),
                    None,
                    failure.transport_request_id,
                )
            })?;
        let json =
            parse_unique_json(&response.body, self.config.schema.limits()).map_err(|_| {
                observed(
                    contract(TextResponseContract::Json, None),
                    None,
                    response.transport_request_id.clone(),
                )
            })?;
        let captured_response_id = json
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| {
                id.starts_with("resp_")
                    && id.len() <= 256
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            })
            .and_then(|id| ProviderRequestId::new(id).ok());
        let failure = |reason| {
            observed(
                reason,
                captured_response_id.clone(),
                response.transport_request_id.clone(),
            )
        };
        if response.status != 200 {
            return Err(failure(TextResponseError::Supplier(
                classify_supplier_status(response.status, &json),
            )));
        }
        let usage = parse_usage(&json)
            .ok_or_else(|| failure(contract(TextResponseContract::Usage, None)))?;
        if usage.total_tokens > limits.max_tokens()
            || usage.total_tokens > limits.max_usage().quantity()
            || usage.output_tokens > u128::from(self.config.maximum_output_tokens)
        {
            return Err(failure(TextResponseError::UsageExceeded(Box::new(usage))));
        }
        if json.get("object").and_then(Value::as_str) != Some("response")
            || json.get("model").and_then(Value::as_str) != Some(self.config.model.as_str())
            || json.get("metadata") != Some(&metadata)
        {
            return Err(failure(contract(
                TextResponseContract::Identity,
                Some(usage),
            )));
        }
        let id = json
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| {
                id.starts_with("resp_")
                    && id.len() <= 256
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            })
            .ok_or_else(|| failure(contract(TextResponseContract::Identity, Some(usage))))?;
        let provider_request_id = ProviderRequestId::new(id)
            .map_err(|_| failure(contract(TextResponseContract::Identity, Some(usage))))?;
        if json.get("status").and_then(Value::as_str) != Some("completed")
            || json.get("error").is_some_and(|value| !value.is_null())
            || json
                .get("incomplete_details")
                .is_some_and(|value| !value.is_null())
        {
            return Err(failure(contract(TextResponseContract::Status, Some(usage))));
        }
        if json
            .get("text")
            .and_then(|text| text.get("format"))
            .map(without_null_description)
            != Some(self.config.schema.format())
        {
            return Err(failure(contract(TextResponseContract::Schema, Some(usage))));
        }
        let output = parse_output(&json, &self.config.schema)
            .map_err(|reason| failure(contract(reason, Some(usage))))?;
        Ok(PreparedTextResponse {
            request,
            provider_request_id,
            transport_request_id: response.transport_request_id,
            usage,
            output: ValidatedTextOutput(output),
        })
    }
}

/// The live API echoes the requested format with an extra `"description": null`; any other
/// difference from the format that was sent still refuses the response.
fn without_null_description(format: &Value) -> Value {
    let mut format = format.clone();
    if let Some(object) = format.as_object_mut()
        && object.get("description").is_some_and(Value::is_null)
    {
        object.remove("description");
    }
    format
}

fn observed(
    failure: TextResponseError,
    provider_request_id: Option<ProviderRequestId>,
    transport_request_id: Option<ProviderRequestId>,
) -> TextResponseError {
    let usage = failure.supplier_usage();
    TextResponseError::Observed {
        failure: Box::new(failure),
        provider_request_id,
        transport_request_id,
        usage: usage.map(Box::new),
    }
}

struct BoundedRequestBody {
    bytes: Vec<u8>,
    maximum_bytes: usize,
}

impl std::io::Write for BoundedRequestBody {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("request body bound"))?;
        if length > self.maximum_bytes {
            return Err(std::io::Error::other("request body bound"));
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| std::io::Error::other("request body capacity"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn contract(reason: TextResponseContract, usage: Option<TextTokenUsage>) -> TextResponseError {
    TextResponseError::Contract {
        reason,
        usage: usage.map(Box::new),
    }
}

fn request_metadata(identity: RequestIdentity) -> Value {
    serde_json::json!({"df_operation":hex(identity.operation.as_bytes()),"df_job":hex(identity.job.as_bytes()),"df_generation":identity.generation.to_string()})
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(char::from(DIGITS[usize::from(byte >> 4)]));
        value.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    value
}

fn parse_usage(json: &Value) -> Option<TextTokenUsage> {
    let usage = json.get("usage")?.as_object()?;
    if usage.keys().any(|key| {
        ![
            "input_tokens",
            "output_tokens",
            "total_tokens",
            "input_tokens_details",
            "output_tokens_details",
        ]
        .contains(&key.as_str())
    }) {
        return None;
    }
    let input = usage.get("input_tokens_details")?.as_object()?;
    let output = usage.get("output_tokens_details")?.as_object()?;
    if input
        .keys()
        .any(|key| !["cached_tokens", "cache_write_tokens"].contains(&key.as_str()))
        || output.keys().any(|key| key != "reasoning_tokens")
    {
        return None;
    }
    let number = |value: &Value| value.as_u64().map(u128::from);
    let input_tokens = number(usage.get("input_tokens")?)?;
    let output_tokens = number(usage.get("output_tokens")?)?;
    let total_tokens = number(usage.get("total_tokens")?)?;
    let cached_input_tokens = number(input.get("cached_tokens")?)?;
    let cache_write_tokens = match input.get("cache_write_tokens") {
        Some(value) => Some(number(value)?),
        None => None,
    };
    let reasoning_output_tokens = number(output.get("reasoning_tokens")?)?;
    if input_tokens.checked_add(output_tokens)? != total_tokens
        || cached_input_tokens.checked_add(cache_write_tokens.unwrap_or(0))? > input_tokens
        || reasoning_output_tokens > output_tokens
    {
        return None;
    }
    Some(TextTokenUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        cached_input_tokens,
        cache_write_tokens,
        reasoning_output_tokens,
    })
}

fn parse_output(json: &Value, schema: &TextOutputSchema) -> Result<Value, TextResponseContract> {
    let items = json
        .get("output")
        .and_then(Value::as_array)
        .ok_or(TextResponseContract::Output)?;
    let mut text = None;
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("message")
                if item.get("role").and_then(Value::as_str) == Some("assistant")
                    && item.get("status").and_then(Value::as_str) == Some("completed") =>
            {
                let content = item
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or(TextResponseContract::Output)?;
                for part in content {
                    match part.get("type").and_then(Value::as_str) {
                        Some("refusal") => return Err(TextResponseContract::Refusal),
                        Some("output_text") if text.is_none() => {
                            if part.get("annotations").is_some_and(|value| {
                                value.as_array().is_none_or(|values| !values.is_empty())
                            }) {
                                return Err(TextResponseContract::Output);
                            }
                            text = part.get("text").and_then(Value::as_str);
                            if text.is_none() {
                                return Err(TextResponseContract::Output);
                            }
                        }
                        _ => return Err(TextResponseContract::Output),
                    }
                }
            }
            // Reasoning is not published. Only the documented empty summary
            // profile is accepted; tools and other output families are refused.
            Some("reasoning")
                if item
                    .get("summary")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                    && item
                        .get("status")
                        .is_none_or(|status| status.as_str() == Some("completed")) => {}
            _ => return Err(TextResponseContract::Output),
        }
    }
    let output = parse_unique_json(
        text.ok_or(TextResponseContract::Output)?.as_bytes(),
        schema.limits(),
    )
    .map_err(|_| TextResponseContract::Json)?;
    schema
        .validate_output(&output)
        .map_err(|error| match error {
            TextSchemaError::OutputMismatch => TextResponseContract::Schema,
            _ => TextResponseContract::Json,
        })?;
    Ok(output)
}

fn classify_supplier_status(status: u16, json: &Value) -> ProviderFailureClass {
    let code = json
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str);
    match status {
        400 | 422 => ProviderFailureClass::InvalidRequest,
        401 | 403 => ProviderFailureClass::Denied,
        429 if code == Some("rate_limit_exceeded") => ProviderFailureClass::Capacity,
        429 if matches!(
            code,
            Some("insufficient_quota" | "billing_hard_limit_reached")
        ) =>
        {
            ProviderFailureClass::Denied
        }
        503 => ProviderFailureClass::Unavailable,
        _ => ProviderFailureClass::Unknown,
    }
}
