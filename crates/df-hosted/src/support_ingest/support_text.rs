//! Hosted support's two model calls through the durable text service.
//!
//! The Jev classification is one batched request (one closed question per id) and the action draft
//! is a second, separate request. Each is a [`TextRequest`] submitted to a [`DurableTextService`]
//! under a scoped operation key, so a restart replays the stored answer instead of asking and
//! paying again, and a send whose result was lost stays Unknown and is never resent.
//!
//! The two private backends below are the only raw model egress in df-hosted: credentials, the
//! endpoint, the bearer header, bounded reads and the wire shapes stay here. The feature's own check
//! of an answer (listed labels and confidences for the judge; the draft schema and agreement with
//! Jev's decision for the draft) is the verdict the transport hands to `qualify`.
//!
//! Judge convention. `output_schema` has one required property per question id; each is an object
//! `{"choice": <one of the listed labels>, "confidence": <0 to 1>}`. The confidence rides along
//! because the classification record stores it and the service owns no other place for it.
//! `instructions` is the one instruction text every question shares and `facts` is
//! `{"state": <the email state>, "criteria": {<id>: {<label>: <sentence>}}}`. The backend turns that
//! into the batched `/v1/systemone` body and back into `{<id>: {"choice", "confidence"}}`.
//! Draft convention. `facts` is `{"state", "classification", "concerns"}`, `output_schema` is the
//! support schema and the answer is the drafted record itself.

use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use df_ai::{
    text::{
        LabelError, TextBackend, TextObserver, TextOutcome, TextProfile, TextPurpose, TextReceipt,
        TextRequest, TextService, TextUsage, UnavailableReason,
    },
    text_attempts::{
        Admission, AttemptMeta, AttemptOwner, AttemptScope, AttemptStoreError, Basis,
        DurableTextError, DurableTextService, OperationKey, Qualification, QualifyResult,
        StoredAttempt, Submission, TextAttemptStore, TextAuthority, Verdict,
    },
    text_fixture::MemoryAttemptStore,
};
use df_persistence::{
    postgres_tls::{PostgresTransport, VerifiedTlsConnector, all_hosts_loopback},
    text_attempts::PostgresTextAttempts,
};
use df_support::{
    classification::{JevJudge, ModelTransport},
    contracts::MeteredCall,
    ingest::{ClassificationDraft, ClassificationJudge, JudgeError},
    ports::Meter,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const JUDGE_PROFILE: &str = "support-jev";
const DRAFT_PROFILE: &str = "support-draft";
const JUDGE_OUTPUT: &str = "support_jev_choices";
const DRAFT_OUTPUT: &str = "support_action_draft";
const JUDGE_PURPOSE: TextPurpose = TextPurpose::SupportJudgment;
const DRAFT_PURPOSE: TextPurpose = TextPurpose::SupportDraft;
const JUDGE_DEADLINE: Duration = Duration::from_secs(4);
const DRAFT_DEADLINE: Duration = Duration::from_secs(15);
const MAXIMUM_BODY_BYTES: usize = 64 * 1024;
const MAXIMUM_DRAFT_OUTPUT_TOKENS: u32 = 4096;
const JUDGE_PATH: &str = "/v1/systemone";
const DRAFT_PATH: &str = "/v1/responses";

/// A validated model endpoint, credential and model name.
pub(super) struct ModelRoute {
    pub(super) endpoint: String,
    key: String,
    pub(super) model: String,
}

/// Reads `{prefix}_API_KEY`, `{prefix}_BASE_URL` and `{prefix}_MODEL`. The base must be a plain
/// HTTPS URL: no credentials, query or fragment in it.
pub(super) fn route(get: &impl Fn(&str) -> Option<String>, prefix: &str) -> Option<ModelRoute> {
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

/// What was read of a response body.
#[derive(Debug, Eq, PartialEq)]
enum Body {
    Complete(Vec<u8>),
    /// The supplier sent more than the bound allows.
    TooLarge,
    /// The connection or the deadline ended the body before it was complete.
    Broken,
}

async fn read_body(mut response: reqwest::Response) -> Body {
    let maximum = u64::try_from(MAXIMUM_BODY_BYTES).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > maximum)
    {
        return Body::TooLarge;
    }
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len().saturating_add(chunk.len()) > MAXIMUM_BODY_BYTES {
                    return Body::TooLarge;
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => return Body::Complete(bytes),
            Err(_) => return Body::Broken,
        }
    }
}

/// How a request that produced no response ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SendFailure {
    /// The request could not be built.
    Builder,
    /// The connection was never made, so nothing was sent.
    Connect,
    /// The deadline elapsed while connecting, so nothing was sent.
    ConnectTimeout,
    /// Anything after the connection: the request may have left.
    Other,
}

fn send_failure(error: &reqwest::Error) -> SendFailure {
    if error.is_builder() {
        SendFailure::Builder
    } else if error.is_connect() && error.is_timeout() {
        SendFailure::ConnectTimeout
    } else if error.is_connect() {
        SendFailure::Connect
    } else {
        SendFailure::Other
    }
}

/// Both backends map their exchange to a [`TextOutcome`] with this table. Usage is kept whenever
/// the supplier reported it. Nothing here retries or resends.
///
/// | What happened                                                          | Outcome                     |
/// | ---------------------------------------------------------------------- | --------------------------- |
/// | no route, or a request the backend cannot encode (never left)          | Unavailable(Configuration)  |
/// | request could not be built (never left)                                | Unavailable(Configuration)  |
/// | connection refused or failed before any send                           | Unavailable(Interrupted)    |
/// | deadline elapsed while connecting                                      | Deadline                    |
/// | 401 or 403                                                             | Unavailable(Authentication) |
/// | 1xx or 3xx (redirects are off, so the request was not served)          | Unavailable(Configuration)  |
/// | other 4xx or 5xx, with a complete or oversized body                    | Invalid                     |
/// | other 4xx or 5xx, body cut short                                       | Unknown                     |
/// | 2xx, complete body that is not the expected answer                     | Invalid                     |
/// | 2xx, oversized body                                                    | Invalid                     |
/// | 2xx, body cut short                                                    | Unknown                     |
/// | 2xx, complete body that is the expected answer                         | Answered                    |
/// | timeout or drop after the request left, no response                    | Unknown                     |
fn outcome_of(
    status: u16,
    body: &Body,
    parse: impl Fn(&Value) -> Option<Value>,
    usage: impl Fn(&Value) -> Option<TextUsage>,
) -> TextReceipt {
    match status {
        401 | 403 => TextOutcome::Unavailable(UnavailableReason::Authentication).into(),
        200..=299 => match body {
            Body::Broken => TextOutcome::Unknown.into(),
            Body::TooLarge => TextOutcome::Invalid.into(),
            Body::Complete(bytes) => match serde_json::from_slice::<Value>(bytes) {
                Err(_) => TextOutcome::Invalid.into(),
                Ok(response) => TextReceipt {
                    outcome: parse(&response).map_or(TextOutcome::Invalid, TextOutcome::Answered),
                    usage: usage(&response),
                },
            },
        },
        400..=599 => match body {
            Body::Broken => TextOutcome::Unknown.into(),
            Body::Complete(_) | Body::TooLarge => TextOutcome::Invalid.into(),
        },
        _ => TextOutcome::Unavailable(UnavailableReason::Configuration).into(),
    }
}

fn failure_outcome(failure: SendFailure) -> TextOutcome {
    match failure {
        SendFailure::Builder => TextOutcome::Unavailable(UnavailableReason::Configuration),
        SendFailure::Connect => TextOutcome::Unavailable(UnavailableReason::Interrupted),
        SendFailure::ConnectTimeout => TextOutcome::Deadline,
        SendFailure::Other => TextOutcome::Unknown,
    }
}

async fn exchange(
    client: &reqwest::Client,
    route: &ModelRoute,
    path: &str,
    body: &Value,
    deadline: Duration,
) -> Result<(u16, Body), TextOutcome> {
    let response = client
        .post(format!("{}{path}", route.endpoint))
        .timeout(deadline)
        .bearer_auth(&route.key)
        .json(body)
        .send()
        .await
        .map_err(|error| failure_outcome(send_failure(&error)))?;
    let status = response.status().as_u16();
    Ok((status, read_body(response).await))
}

fn token_count(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer)?.as_u64()
}

/// The supplier's tokens as `TextUsage`. Unreported categories stay unknown. Without a supplier
/// total, the total is the sum of the reported input and output; a sum that does not fit is unknown,
/// never clamped.
fn text_usage(
    total: Option<u64>,
    input: Option<u64>,
    output: Option<u64>,
    cached_input: Option<u64>,
    reasoning_output: Option<u64>,
) -> Option<TextUsage> {
    let total_tokens = match (total, input, output) {
        (Some(total), ..) => total,
        (None, None, None) => return None,
        (None, Some(only), None) | (None, None, Some(only)) => only,
        (None, Some(input), Some(output)) => input.checked_add(output)?,
    };
    Some(TextUsage {
        total_tokens,
        input_tokens: input,
        output_tokens: output,
        cached_input_tokens: cached_input,
        cache_write_tokens: None,
        reasoning_output_tokens: reasoning_output,
    })
}

fn judge_usage(response: &Value) -> Option<TextUsage> {
    text_usage(
        None,
        token_count(response, "/usage/input_tokens"),
        token_count(response, "/usage/output_tokens"),
        None,
        None,
    )
}

fn draft_usage(response: &Value) -> Option<TextUsage> {
    text_usage(
        token_count(response, "/usage/total_tokens"),
        token_count(response, "/usage/input_tokens"),
        token_count(response, "/usage/output_tokens"),
        token_count(response, "/usage/input_tokens_details/cached_tokens"),
        token_count(response, "/usage/output_tokens_details/reasoning_tokens"),
    )
}

/// The question ids and, for each, the sentences describing its labels.
struct JudgeWire<'a> {
    state: &'a Value,
    instructions: &'a str,
    questions: Vec<(&'a str, &'a Value)>,
}

impl<'a> JudgeWire<'a> {
    fn decode(request: &'a TextRequest) -> Option<Self> {
        let ids = request.output_schema.get("properties")?.as_object()?;
        let criteria = request.facts.get("criteria")?.as_object()?;
        let questions = ids
            .keys()
            .map(|id| Some((id.as_str(), criteria.get(id)?)))
            .collect::<Option<Vec<_>>>()?;
        (!questions.is_empty() && criteria.len() == questions.len()).then_some(Self {
            state: request.facts.get("state")?,
            instructions: &request.instructions,
            questions,
        })
    }

    /// The batched `/v1/systemone` body: one closed question per id.
    fn body(&self, model: &str) -> Value {
        let questions: Map<String, Value> = self
            .questions
            .iter()
            .map(|(id, criteria)| {
                (
                    (*id).to_owned(),
                    json!({"type":"choice","instructions":self.instructions,"criteria":criteria}),
                )
            })
            .collect();
        json!({"model":model,"state":self.state,"questions":questions})
    }
}

/// `{<id>: {"choice", "confidence"}}` from the supplier's `answers`, for exactly the asked ids.
fn judge_answers(ids: &[&str], response: &Value) -> Option<Value> {
    let answers = response.get("answers")?.as_object()?;
    if answers.len() != ids.len() {
        return None;
    }
    let mut chosen = Map::new();
    for id in ids {
        let answer = answers.get(*id)?.as_object()?;
        if answer.len() != 3 || answer.get("type")?.as_str()? != "choice" {
            return None;
        }
        chosen.insert(
            (*id).to_owned(),
            json!({"choice":answer.get("choice")?.as_str()?,
                "confidence":answer.get("confidence")?.as_f64()?}),
        );
    }
    Some(Value::Object(chosen))
}

/// The Responses-style reply's single output text, parsed as the drafted record.
fn draft_record(response: &Value) -> Option<Value> {
    if response.get("status")?.as_str()? != "completed" {
        return None;
    }
    let texts: Vec<&str> = response
        .get("output")?
        .as_array()?
        .iter()
        .filter_map(|message| message.get("content")?.as_array())
        .flatten()
        .filter(|content| content.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(|content| content.get("text").and_then(Value::as_str))
        .collect();
    let [text] = texts.as_slice() else {
        return None;
    };
    serde_json::from_str::<Value>(text)
        .ok()
        .filter(Value::is_object)
}

fn draft_body(request: &TextRequest, model: &str) -> Value {
    json!({"model":model,"store":false,
        "instructions":request.instructions,"input":request.facts.to_string(),
        "max_output_tokens":MAXIMUM_DRAFT_OUTPUT_TOKENS,
        "text":{"format":{"type":"json_schema","name":request.output_name,
            "strict":true,"schema":request.output_schema}}})
}

fn judge_receipt(request: &TextRequest, status: u16, body: &Body) -> TextReceipt {
    let ids = question_ids(request);
    outcome_of(
        status,
        body,
        |response| judge_answers(&ids, response),
        judge_usage,
    )
}

fn draft_receipt(status: u16, body: &Body) -> TextReceipt {
    outcome_of(status, body, draft_record, draft_usage)
}

fn question_ids(request: &TextRequest) -> Vec<&str> {
    request
        .output_schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().map(String::as_str).collect())
        .unwrap_or_default()
}

/// The batched Jev classification over `POST {base}/v1/systemone`.
pub(crate) struct SupportJudgeBackend {
    client: reqwest::Client,
    route: Option<ModelRoute>,
}

impl SupportJudgeBackend {
    pub(super) fn new(client: reqwest::Client, route: Option<ModelRoute>) -> Self {
        Self { client, route }
    }
}

impl TextBackend for SupportJudgeBackend {
    async fn complete(&self, request: &TextRequest) -> TextReceipt {
        let (Some(route), Some(wire)) = (&self.route, JudgeWire::decode(request)) else {
            return TextOutcome::Unavailable(UnavailableReason::Configuration).into();
        };
        match exchange(
            &self.client,
            route,
            JUDGE_PATH,
            &wire.body(&route.model),
            request.deadline,
        )
        .await
        {
            Ok((status, body)) => judge_receipt(request, status, &body),
            Err(outcome) => outcome.into(),
        }
    }
}

/// The action draft over `POST {base}/v1/responses`, with `store:false` and a strict schema.
pub(crate) struct SupportDraftBackend {
    client: reqwest::Client,
    route: Option<ModelRoute>,
}

impl SupportDraftBackend {
    pub(super) fn new(client: reqwest::Client, route: Option<ModelRoute>) -> Self {
        Self { client, route }
    }
}

impl TextBackend for SupportDraftBackend {
    async fn complete(&self, request: &TextRequest) -> TextReceipt {
        let Some(route) = &self.route else {
            return TextOutcome::Unavailable(UnavailableReason::Configuration).into();
        };
        match exchange(
            &self.client,
            route,
            DRAFT_PATH,
            &draft_body(request, &route.model),
            request.deadline,
        )
        .await
        {
            Ok((status, body)) => draft_receipt(status, &body),
            Err(outcome) => outcome.into(),
        }
    }
}

/// The request of the batched judge question set, from the wire `JevJudge` builds.
fn judge_request(wire: &Value) -> Option<TextRequest> {
    let questions = wire.get("questions")?.as_object()?;
    let mut instructions: Option<&str> = None;
    let mut properties = Map::new();
    let mut criteria = Map::new();
    for (id, question) in questions {
        if question.get("type")?.as_str()? != "choice" {
            return None;
        }
        let text = question.get("instructions")?.as_str()?;
        if instructions.is_some_and(|shared| shared != text) {
            return None;
        }
        instructions = Some(text);
        let labels = question.get("criteria")?.as_object()?;
        criteria.insert(id.clone(), Value::Object(labels.clone()));
        properties.insert(
            id.clone(),
            json!({"type":"object","additionalProperties":false,
                "required":["choice","confidence"],
                "properties":{"choice":{"type":"string","enum":labels.keys().collect::<Vec<_>>()},
                    "confidence":{"type":"number","minimum":0,"maximum":1}}}),
        );
    }
    Some(TextRequest {
        purpose: JUDGE_PURPOSE,
        profile: TextProfile::new(JUDGE_PROFILE).ok()?,
        instructions: instructions?.to_owned(),
        facts: json!({"state":wire.get("state")?,"criteria":criteria}),
        output_name: JUDGE_OUTPUT.to_owned(),
        output_schema: json!({"type":"object","additionalProperties":false,
            "required":properties.keys().collect::<Vec<_>>(),"properties":properties}),
        deadline: JUDGE_DEADLINE,
    })
}

/// The request of the action draft, from the wire `JevJudge` builds.
fn draft_request(wire: &Value) -> Option<TextRequest> {
    Some(TextRequest {
        purpose: DRAFT_PURPOSE,
        profile: TextProfile::new(DRAFT_PROFILE).ok()?,
        instructions: wire.get("instructions")?.as_str()?.to_owned(),
        facts: json!({"state":wire.get("state")?,"classification":wire.get("classification")?,
            "concerns":wire.get("concerns")?}),
        output_name: DRAFT_OUTPUT.to_owned(),
        output_schema: wire.get("schema")?.clone(),
        deadline: DRAFT_DEADLINE,
    })
}

/// The judge's data guard: every asked question is answered once with one of its listed labels and
/// a confidence from zero through one, and nothing else is present.
fn judge_answer_is_listed(request: &TextRequest, answer: &Value) -> bool {
    let (Some(properties), Some(answer)) = (
        request
            .output_schema
            .get("properties")
            .and_then(Value::as_object),
        answer.as_object(),
    ) else {
        return false;
    };
    answer.len() == properties.len()
        && properties.iter().all(|(id, schema)| {
            let Some(entry) = answer.get(id).and_then(Value::as_object) else {
                return false;
            };
            let listed = |label: &str| {
                schema
                    .pointer("/properties/choice/enum")
                    .and_then(Value::as_array)
                    .is_some_and(|labels| labels.iter().any(|item| item.as_str() == Some(label)))
            };
            entry.len() == 2
                && entry
                    .get("choice")
                    .and_then(Value::as_str)
                    .is_some_and(listed)
                && entry
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .is_some_and(|confidence| (0.0..=1.0).contains(&confidence))
        })
}

/// The draft's data guard: the record satisfies the support schema, and it agrees with the
/// decision it was drafted for, because the generative model cannot change Jev's decision.
fn draft_is_consistent(request: &TextRequest, record: &Value) -> bool {
    let Ok(draft) = ClassificationDraft::parse(&record.to_string()) else {
        return false;
    };
    request
        .facts
        .get("classification")
        .and_then(|decision| serde_json::from_value::<ClassificationDraft>(decision.clone()).ok())
        .is_some_and(|decision| {
            draft.category == decision.category
                && draft.urgency == decision.urgency
                && draft.confidence == decision.confidence
        })
}

fn usage_json(usage: Option<TextUsage>) -> Option<Value> {
    usage
        .map(|usage| json!({"input_tokens":usage.input_tokens,"output_tokens":usage.output_tokens}))
}

/// The value `JevJudge` expects from the judge transport: `answers` and any reported usage.
fn judge_value(answer: &Value, usage: Option<TextUsage>) -> Value {
    let answers: Map<String, Value> = answer
        .as_object()
        .map(|answers| {
            answers
                .iter()
                .map(|(id, entry)| {
                    (
                        id.clone(),
                        json!({"type":"choice","choice":entry.get("choice"),
                            "confidence":entry.get("confidence")}),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    json!({"answers":answers,"usage":usage_json(usage)})
}

/// The value `JevJudge` expects from the draft transport: the record and any reported usage.
fn draft_value(record: Value, usage: Option<TextUsage>) -> Value {
    json!({"record":record,"usage":usage_json(usage)})
}

/// One run's owner. It holds one basis for the whole run (a digest of the mailbox label and a
/// generation counter) and lets go of it when the run ends, so an answer that lands afterwards is
/// recorded as stale instead of being reused.
pub(crate) struct MailboxAuthority {
    basis: Mutex<Option<Basis>>,
}

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

impl MailboxAuthority {
    pub(super) fn start(mailbox: &str) -> Self {
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        let mut hasher = Sha256::new();
        hasher.update(b"support-mailbox/1");
        hasher.update(mailbox.len().to_be_bytes());
        hasher.update(mailbox.as_bytes());
        hasher.update(generation.to_be_bytes());
        Self {
            basis: Mutex::new(Some(hasher.finalize().into())),
        }
    }

    pub(super) fn release(&self) {
        *self.basis.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

impl TextAuthority for MailboxAuthority {
    fn current_basis(&self, _: &OperationKey) -> impl Future<Output = Option<Basis>> + Send {
        let basis = *self.basis.lock().unwrap_or_else(PoisonError::into_inner);
        std::future::ready(basis)
    }
}

/// One owner token per process start, from the process id and the start time.
fn process_owner() -> Option<AttemptOwner> {
    let mut bytes = [0_u8; 16];
    bytes[..4].copy_from_slice(&std::process::id().to_be_bytes());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    bytes[4..].copy_from_slice(&nanos.to_be_bytes()[4..]);
    AttemptOwner::new(bytes)
}

/// PostgreSQL attempts that connect on first use, so building the worker needs no await.
pub(crate) struct DeferredPostgres {
    config: tokio_postgres::Config,
    connector: Option<VerifiedTlsConnector>,
    store: tokio::sync::Mutex<Option<Arc<PostgresTextAttempts>>>,
}

impl DeferredPostgres {
    async fn connected(&self) -> Result<Arc<PostgresTextAttempts>, AttemptStoreError> {
        let mut slot = self.store.lock().await;
        if let Some(store) = slot.as_ref() {
            return Ok(Arc::clone(store));
        }
        let store = match &self.connector {
            Some(connector) => {
                PostgresTextAttempts::connect_tls(&self.config, connector.clone()).await?
            }
            None => PostgresTextAttempts::connect(&self.config).await?,
        };
        let store = Arc::new(store);
        *slot = Some(Arc::clone(&store));
        Ok(store)
    }
}

/// The attempt store a worker has: the hosted database, or memory when the database cannot be
/// reached safely (still one send per operation per process, but not across a restart).
pub(crate) enum SupportAttemptStore {
    Postgres(Box<DeferredPostgres>),
    Memory {
        store: MemoryAttemptStore,
        reason: &'static str,
    },
}

impl SupportAttemptStore {
    /// The store for this database and transport. TLS is verified against the configured CA;
    /// plaintext is accepted only to loopback, as everywhere else in the hosted composition.
    pub(super) fn choose(config: &tokio_postgres::Config, transport: &PostgresTransport) -> Self {
        let memory = |reason| Self::Memory {
            store: MemoryAttemptStore::default(),
            reason,
        };
        let connector = match transport {
            PostgresTransport::VerifiedTls { ca_file } => {
                match VerifiedTlsConnector::from_ca_file(ca_file) {
                    Ok(connector) => Some(connector),
                    Err(_) => return memory("ca_unreadable"),
                }
            }
            PostgresTransport::PlaintextLoopbackOnly if all_hosts_loopback(config) => None,
            PostgresTransport::PlaintextLoopbackOnly => return memory("plaintext_to_remote_host"),
        };
        Self::Postgres(Box::new(DeferredPostgres {
            config: config.clone(),
            connector,
            store: tokio::sync::Mutex::new(None),
        }))
    }

    /// The label the start-up line prints; never a connection string.
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Postgres(_) => "postgres",
            Self::Memory { .. } => "memory",
        }
    }

    pub(super) fn fallback_reason(&self) -> Option<&'static str> {
        match self {
            Self::Postgres(_) => None,
            Self::Memory { reason, .. } => Some(reason),
        }
    }
}

macro_rules! delegate {
    ($self:ident, $method:ident($($argument:expr),*)) => {
        match $self {
            SupportAttemptStore::Postgres(deferred) => {
                deferred.connected().await?.$method($($argument),*).await
            }
            SupportAttemptStore::Memory { store, .. } => store.$method($($argument),*).await,
        }
    };
}

impl TextAttemptStore for SupportAttemptStore {
    async fn lookup(&self, key: &OperationKey) -> Result<Option<StoredAttempt>, AttemptStoreError> {
        delegate!(self, lookup(key))
    }

    async fn admit(
        &self,
        key: &OperationKey,
        meta: &AttemptMeta<'_>,
    ) -> Result<Admission, AttemptStoreError> {
        delegate!(self, admit(key, meta))
    }

    async fn begin_dispatch(
        &self,
        key: &OperationKey,
        owner: AttemptOwner,
        basis: Basis,
    ) -> Result<bool, AttemptStoreError> {
        delegate!(self, begin_dispatch(key, owner, basis))
    }

    async fn record_candidate(
        &self,
        key: &OperationKey,
        answer: &Value,
        usage: Option<TextUsage>,
    ) -> Result<bool, AttemptStoreError> {
        delegate!(self, record_candidate(key, answer, usage))
    }

    async fn settle(
        &self,
        key: &OperationKey,
        outcome: &TextOutcome,
        usage: Option<TextUsage>,
    ) -> Result<bool, AttemptStoreError> {
        delegate!(self, settle(key, outcome, usage))
    }

    async fn qualify(
        &self,
        key: &OperationKey,
        verdict: Verdict,
        basis_now: Option<Basis>,
    ) -> Result<QualifyResult, AttemptStoreError> {
        delegate!(self, qualify(key, verdict, basis_now))
    }

    async fn invalidate(&self, key: &OperationKey) -> Result<bool, AttemptStoreError> {
        delegate!(self, invalidate(key))
    }

    async fn mark_unknown(&self, key: &OperationKey) -> Result<bool, AttemptStoreError> {
        delegate!(self, mark_unknown(key))
    }

    async fn cancel(&self, key: &OperationKey) -> Result<bool, AttemptStoreError> {
        delegate!(self, cancel(key))
    }
}

type Service<B, S, O> = DurableTextService<B, Arc<S>, Arc<MailboxAuthority>, O>;

/// Why the worker's text composition could not be built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetupError {
    Mailbox(LabelError),
    Owner,
}

/// Everything one worker's text composition is made of.
pub(crate) struct Parts<J, D, S, O> {
    pub(crate) mailbox: String,
    pub(crate) judge: J,
    pub(crate) draft: D,
    pub(crate) store: Arc<S>,
    pub(crate) observer: O,
    pub(crate) judge_model: Option<String>,
    pub(crate) draft_model: Option<String>,
}

struct Composition<J, D, S, O> {
    judge: Service<J, S, O>,
    draft: Service<D, S, O>,
    scope: AttemptScope,
    authority: Arc<MailboxAuthority>,
    judge_model: Option<String>,
    draft_model: Option<String>,
}

/// The existing `ModelTransport`, served by the durable text service. A transport made with
/// [`Self::for_job`] names its operations after that classification job, so a restart replays
/// them; without a job the operation is named by the request's own digest.
pub(crate) struct SupportTextTransport<J, D, S, O> {
    composition: Arc<Composition<J, D, S, O>>,
    job: Option<String>,
}

impl<J, D, S, O> Clone for SupportTextTransport<J, D, S, O> {
    fn clone(&self) -> Self {
        Self {
            composition: Arc::clone(&self.composition),
            job: self.job.clone(),
        }
    }
}

impl<J, D, S, O> SupportTextTransport<J, D, S, O>
where
    J: TextBackend,
    D: TextBackend,
    S: TextAttemptStore,
    O: TextObserver + Clone,
{
    pub(crate) fn new(parts: Parts<J, D, S, O>) -> Result<Self, SetupError> {
        let Parts {
            mailbox,
            judge,
            draft,
            store,
            observer,
            judge_model,
            draft_model,
        } = parts;
        let scope =
            AttemptScope::new(&format!("support:{mailbox}")).map_err(SetupError::Mailbox)?;
        let owner = process_owner().ok_or(SetupError::Owner)?;
        let authority = Arc::new(MailboxAuthority::start(&mailbox));
        let judge = durable(judge, &store, &observer, &authority, owner);
        let draft = durable(draft, &store, &observer, &authority, owner);
        Ok(Self {
            composition: Arc::new(Composition {
                judge,
                draft,
                scope,
                authority,
                judge_model,
                draft_model,
            }),
            job: None,
        })
    }

    /// The same composition, naming its operations after one classification job.
    pub(crate) fn for_job(&self, job: &str) -> Self {
        Self {
            composition: Arc::clone(&self.composition),
            job: Some(job.to_owned()),
        }
    }

    fn key(&self, stage: &str, request: &TextRequest) -> Result<OperationKey, JudgeError> {
        let operation = match &self.job {
            Some(job) => format!("{stage}-{job}"),
            None => {
                let digest: String = df_ai::text_attempts::fingerprint(request)
                    .iter()
                    .take(16)
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                format!("{stage}-h{digest}")
            }
        };
        OperationKey::new(&self.composition.scope, &operation).map_err(|_| JudgeError::Unusable)
    }
}

fn durable<B, S, O>(
    backend: B,
    store: &Arc<S>,
    observer: &O,
    authority: &Arc<MailboxAuthority>,
    owner: AttemptOwner,
) -> Service<B, S, O>
where
    B: TextBackend,
    S: TextAttemptStore,
    O: TextObserver + Clone,
{
    DurableTextService::new(
        TextService::new(backend).with_observer(observer.clone()),
        Arc::clone(store),
        Arc::clone(authority),
        owner,
    )
}

/// The answer the feature accepted, with the usage the supplier reported for it.
struct Accepted {
    answer: Value,
    usage: Option<TextUsage>,
}

/// Submits one request, applies the feature's check to the answer and qualifies it exactly once.
/// Every path that is not an accepted answer maps to the error the old transport gave for it.
async fn call<B, S, O>(
    service: &Service<B, S, O>,
    key: &OperationKey,
    request: &TextRequest,
    accept: &(dyn Fn(&Value) -> bool + Sync),
) -> Result<Accepted, JudgeError>
where
    B: TextBackend,
    S: TextAttemptStore,
    O: TextObserver,
{
    match service.submit(key, request).await {
        Ok(Submission::Candidate { answer, usage, .. }) => {
            if !accept(&answer) {
                // A failed verdict is reported by the service; the call is unusable either way.
                let _ = service.qualify(key, request, Verdict::Rejected).await;
                return Err(JudgeError::Unusable);
            }
            match service.qualify(key, request, Verdict::Accepted).await {
                Ok(Qualification::Qualified) => Ok(Accepted { answer, usage }),
                Ok(Qualification::Rejected) => Err(JudgeError::Unusable),
                Ok(Qualification::NotACandidate | Qualification::Stale) | Err(_) => {
                    Err(JudgeError::Unavailable)
                }
            }
        }
        Ok(Submission::Settled {
            outcome: TextOutcome::Answered(answer),
            usage,
            ..
        }) => {
            if accept(&answer) {
                Ok(Accepted { answer, usage })
            } else {
                Err(JudgeError::Unusable)
            }
        }
        Ok(Submission::Settled {
            outcome: TextOutcome::Deadline,
            ..
        }) => Err(JudgeError::DeadlineExceeded),
        Ok(Submission::Settled {
            outcome: TextOutcome::Invalid,
            ..
        })
        | Ok(Submission::Rejected { .. })
        | Err(DurableTextError::Refused(_) | DurableTextError::Conflict) => {
            Err(JudgeError::Unusable)
        }
        // Unknown spend stays Unknown: it is neither retried nor guessed into a classification.
        Ok(
            Submission::Settled { .. }
            | Submission::InFlight
            | Submission::Unknown
            | Submission::Cancelled,
        )
        | Err(
            DurableTextError::Store(_)
            | DurableTextError::Unauthorized
            | DurableTextError::UnknownOperation,
        ) => Err(JudgeError::Unavailable),
    }
}

#[async_trait::async_trait]
impl<J, D, S, O> ModelTransport for SupportTextTransport<J, D, S, O>
where
    J: TextBackend,
    D: TextBackend,
    S: TextAttemptStore,
    O: TextObserver + Clone,
{
    fn model_name(&self, is_draft: bool) -> &str {
        let model = if is_draft {
            &self.composition.draft_model
        } else {
            &self.composition.judge_model
        };
        model.as_deref().unwrap_or("unconfigured")
    }

    async fn judge(&self, request: Value) -> Result<Value, JudgeError> {
        if self.composition.judge_model.is_none() {
            return Err(JudgeError::Unavailable);
        }
        let request = judge_request(&request).ok_or(JudgeError::Unusable)?;
        let key = self.key("jev", &request)?;
        let accepted = call(&self.composition.judge, &key, &request, &|answer| {
            judge_answer_is_listed(&request, answer)
        })
        .await?;
        Ok(judge_value(&accepted.answer, accepted.usage))
    }

    async fn draft(&self, request: Value) -> Result<Value, JudgeError> {
        if self.composition.draft_model.is_none() {
            return Err(JudgeError::Unavailable);
        }
        let request = draft_request(&request).ok_or(JudgeError::Unusable)?;
        let key = self.key("draft", &request)?;
        let accepted = call(&self.composition.draft, &key, &request, &|record| {
            draft_is_consistent(&request, record)
        })
        .await?;
        Ok(draft_value(accepted.answer, accepted.usage))
    }
}

/// The ingest worker's judge: Jev over the text service, with each message's calls named after its
/// classification job. The authority is released when the judge is dropped, which is when the
/// worker ends.
pub(crate) struct SupportJudge<J, D, S, O> {
    transport: SupportTextTransport<J, D, S, O>,
}

impl<J, D, S, O> SupportJudge<J, D, S, O> {
    pub(crate) fn new(transport: SupportTextTransport<J, D, S, O>) -> Self {
        Self { transport }
    }
}

impl<J, D, S, O> Drop for SupportJudge<J, D, S, O> {
    fn drop(&mut self) {
        self.transport.composition.authority.release();
    }
}

#[async_trait::async_trait]
impl<J, D, S, O> ClassificationJudge for SupportJudge<J, D, S, O>
where
    J: TextBackend,
    D: TextBackend,
    S: TextAttemptStore,
    O: TextObserver + Clone,
{
    async fn judge(&self, subject: &str, body_text: &str) -> Result<String, JudgeError> {
        JevJudge::new(self.transport.clone())
            .judge(subject, body_text)
            .await
    }

    async fn judge_metered(
        &self,
        subject: &str,
        body_text: &str,
        meter: &dyn Meter,
        basis: MeteredCall,
    ) -> Result<String, JudgeError> {
        JevJudge::new(self.transport.for_job(basis.job_id.as_str()))
            .judge_metered(subject, body_text, meter, basis)
            .await
    }
}

/// The production composition: reqwest backends, the hosted store and the hosted log observer.
pub(crate) type ProductionJudge = SupportJudge<
    SupportJudgeBackend,
    SupportDraftBackend,
    SupportAttemptStore,
    Arc<Option<Arc<df_ai::text_telemetry::TextTelemetry>>>,
>;

#[cfg(test)]
#[path = "support_text_tests.rs"]
mod tests;
