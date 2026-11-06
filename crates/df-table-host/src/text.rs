//! The host's composition of the text surface: one `df-ai` durable text service, its store, its
//! authority and its observer, over one of the host's private backends.
//!
//! A host feature that needs a model builds a `df_ai::text::TextRequest` (purpose, profile,
//! instructions, facts, schema, pictures) and submits it here under a scoped operation key; it
//! checks the candidate with its own rule and qualifies it. Nothing in the host calls a model any
//! other way. The store is in memory (the dev host has no database): one send per operation per
//! process, and a qualified answer replays without asking again; a rejected answer is never
//! regenerated under its key. Each event becomes one `TEXT` audit line of labels, ids and times,
//! never prompts, pictures or answers. Building the runtime writes `TEXT runtime` (the backend, its
//! profile and the variable to set when it is not configured), and each submission writes
//! `TEXT started` before the service decides whether to send.
//!
//! Layout: this module holds what every backend shares (the backend choice, environment reading,
//! the facts text and supplier usage); `responses_text` (OpenAI's Responses API) and `codex_text`
//! (the signed-in Codex CLI) hold only their own wire shapes. `DF_TEXT_BACKEND` picks one:
//! `responses` or `codex`; without it, Responses when an OpenAI key is set, else Codex.

pub mod codex_text;
mod limits;
pub mod responses_text;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use df_ai::text::{
    TextBackend, TextEvent, TextObserver, TextProfile, TextReceipt, TextRequest, TextService,
    TextUsage,
};
use df_ai::text_attempts::{
    AttemptOwner, AttemptScope, Basis, DurableTextError, DurableTextService, OperationKey,
    Submission, TextAuthority,
};
use df_ai::text_fixture::MemoryAttemptStore;
use serde::{Deserialize, Serialize};

use self::codex_text::{CodexRoute, CodexTextBackend};
use self::responses_text::{ResponsesRoute, ResponsesTextBackend};
use crate::environment;

pub use self::limits::IDENTITY_DEADLINE;

/// The environment variable that picks the backend: `responses` or `codex`.
pub const BACKEND_VARIABLE: &str = "DF_TEXT_BACKEND";
/// The environment variables the Responses key is read from, in order.
pub const KEY_VARIABLES: [&str; 2] = ["DF_OPENAI_API_KEY", "OPENAI_API_KEY"];
/// The environment variable that picks the model for character identity.
pub const IDENTITY_MODEL_VARIABLE: &str = "DF_TEXT_IDENTITY_MODEL";
/// The cheap vision model the project prices (`content/pricing.json`): GPT-6 Luna. Responses uses
/// it unless `IDENTITY_MODEL_VARIABLE` says otherwise; Codex uses its own default unless told.
pub const DEFAULT_IDENTITY_MODEL: &str = "gpt-6-luna";

/// The request's facts as the one line every backend sends them in.
pub(crate) fn facts_text(request: &TextRequest) -> String {
    format!("Facts (JSON): {}", request.facts)
}

/// A supplier's token counts as `TextUsage`. Unreported categories stay unknown, never zero;
/// without a supplier total the total is input plus output, and a sum that does not fit is
/// unknown rather than clamped.
pub(crate) fn usage_from(
    total: Option<u64>,
    input: Option<u64>,
    output: Option<u64>,
    cached_input: Option<u64>,
    reasoning_output: Option<u64>,
) -> Option<TextUsage> {
    let total_tokens = match (total, input, output) {
        (Some(total), ..) => total,
        (None, Some(input), Some(output)) => input.checked_add(output)?,
        (None, Some(only), None) | (None, None, Some(only)) => only,
        (None, None, None) => return None,
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

/// Which of the host's adapters a runtime is built over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Responses,
    Codex,
}

impl BackendKind {
    pub const ALL: [Self; 2] = [Self::Responses, Self::Codex];

    pub fn key(self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::Codex => "codex",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// The adapter a host backend sends through: exactly one, chosen at start.
enum TextAdapter {
    Responses(ResponsesTextBackend),
    Codex(CodexTextBackend),
}

/// The host's backend: one adapter, and the count of its calls under way.
pub struct HostTextBackend {
    adapter: TextAdapter,
    in_flight: Arc<AtomicUsize>,
}

/// Counts one backend call while it lasts; dropping it (the call ended or was dropped) uncounts
/// it.
struct InFlight<'a>(&'a AtomicUsize);

impl<'a> InFlight<'a> {
    fn enter(count: &'a AtomicUsize) -> Self {
        // Relaxed: the count orders no other data; a status read only shows it.
        count.fetch_add(1, Ordering::Relaxed);
        Self(count)
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

impl TextBackend for HostTextBackend {
    async fn complete(&self, request: &TextRequest) -> TextReceipt {
        let _counted = InFlight::enter(&self.in_flight);
        match &self.adapter {
            TextAdapter::Responses(backend) => backend.complete(request).await,
            TextAdapter::Codex(backend) => backend.complete(request).await,
        }
    }
}

/// Which backend a runtime is built over, and with what route (none: unconfigured).
pub enum BackendChoice {
    Responses(Option<ResponsesRoute>),
    Codex(Option<CodexRoute>),
}

impl BackendChoice {
    /// The choice this process's environment gives (see the module documentation).
    pub fn from_environment() -> Self {
        let key = environment::first_text(&KEY_VARIABLES);
        let model = environment::text(IDENTITY_MODEL_VARIABLE);
        let codex = || Self::Codex(CodexRoute::from_environment(model.clone()));
        let responses = |key: Option<String>| {
            Self::Responses(key.map(|key| {
                ResponsesRoute {
                    model: model
                        .clone()
                        .unwrap_or_else(|| DEFAULT_IDENTITY_MODEL.to_owned()),
                    key,
                }
            }))
        };
        match environment::text(BACKEND_VARIABLE).as_deref() {
            Some("codex") => codex(),
            Some("responses") => responses(key),
            _ if key.is_some() => responses(key),
            _ => codex(),
        }
    }

    /// The profile label requests go under: backend and model, so answers of different models
    /// never replay for one another.
    fn profile_label(&self) -> String {
        match self {
            Self::Responses(route) => format!(
                "responses:{}",
                route
                    .as_ref()
                    .map_or(DEFAULT_IDENTITY_MODEL, |route| route.model.as_str())
            ),
            Self::Codex(route) => match route.as_ref().and_then(|route| route.model.as_deref()) {
                Some(model) => format!("codex:{model}"),
                None => "codex".to_owned(),
            },
        }
    }

    fn is_configured(&self) -> bool {
        match self {
            Self::Responses(route) => route.is_some(),
            Self::Codex(route) => route.is_some(),
        }
    }

    fn kind(&self) -> BackendKind {
        match self {
            Self::Responses(_) => BackendKind::Responses,
            Self::Codex(_) => BackendKind::Codex,
        }
    }

    /// The model identity questions name: the route's, Responses' default, or none (Codex's own).
    fn identity_model(&self) -> Option<String> {
        match self {
            Self::Responses(route) => Some(
                route
                    .as_ref()
                    .map_or(DEFAULT_IDENTITY_MODEL, |route| route.model.as_str())
                    .to_owned(),
            ),
            Self::Codex(route) => route.as_ref().and_then(|route| route.model.clone()),
        }
    }

    /// The one variable to set when the choice is not configured.
    fn missing_variable(&self) -> &'static str {
        match self {
            Self::Responses(_) => KEY_VARIABLES[1],
            Self::Codex(_) => codex_text::EXECUTABLE_VARIABLE,
        }
    }

    /// What to set when the choice is not configured, naming the variables this module reads.
    fn missing(&self) -> String {
        match self {
            Self::Responses(_) => {
                let [named_key, plain_key] = KEY_VARIABLES;
                format!(
                    "no OpenAI key in the host's environment: \
                     set {plain_key} (or {named_key}), or {BACKEND_VARIABLE}=codex"
                )
            }
            Self::Codex(_) => format!(
                "no Codex executable: set {} to the signed-in codex.exe, or put codex on the path",
                codex_text::EXECUTABLE_VARIABLE
            ),
        }
    }

    fn into_backend(
        self,
        record: Arc<dyn Fn(String) + Send + Sync>,
        in_flight: Arc<AtomicUsize>,
    ) -> HostTextBackend {
        let adapter = match self {
            Self::Responses(route) => TextAdapter::Responses(ResponsesTextBackend::new(route)),
            Self::Codex(route) => TextAdapter::Codex(CodexTextBackend::new(route, record)),
        };
        HostTextBackend { adapter, in_flight }
    }
}

/// One basis per run of the host: the owner holds every operation it starts until it stops.
pub struct ProcessAuthority {
    basis: Basis,
}

impl TextAuthority for ProcessAuthority {
    fn current_basis(&self, _: &OperationKey) -> impl Future<Output = Option<Basis>> + Send {
        std::future::ready(Some(self.basis))
    }
}

/// Turns each text event into one audit line.
pub struct LineObserver {
    record: Arc<dyn Fn(String) + Send + Sync>,
}

/// A token count as a line writes it: the number, or `-` when the supplier did not report it.
fn tokens(count: Option<u64>) -> String {
    count.map_or_else(|| "-".to_owned(), |count| count.to_string())
}

/// The line of one text event: its kind, labels, time, which submit it is and the tokens its call
/// reported, then the operation and the closed reason when it has them.
pub(crate) fn event_line(event: &TextEvent) -> String {
    let usage = event.usage.as_ref();
    let mut line = format!(
        concat!(
            "TEXT {} purpose={} profile={} output={} duration_ms={} attempt={} tokens_in={} ",
            "tokens_out={} tokens_cached={}"
        ),
        event.kind.key(),
        event.purpose.key(),
        event.profile.as_str(),
        event.output_name,
        event.elapsed.as_millis(),
        event.attempt,
        tokens(usage.and_then(|usage| usage.input_tokens)),
        tokens(usage.and_then(|usage| usage.output_tokens)),
        tokens(usage.and_then(|usage| usage.cached_input_tokens))
    );
    if let (Some(scope), Some(operation)) = (&event.scope, &event.operation) {
        line.push_str(&format!(" scope={scope} operation={operation}"));
    }
    if let Some(reason) = event.kind.reason() {
        line.push_str(&format!(" reason={reason}"));
    }
    line
}

impl TextObserver for LineObserver {
    fn observe(&self, event: &TextEvent) {
        (self.record)(event_line(event));
    }
}

/// The durable text service the host's features submit to.
pub type HostTextService = DurableTextService<
    HostTextBackend,
    Arc<MemoryAttemptStore>,
    Arc<ProcessAuthority>,
    LineObserver,
>;

/// The composed text surface of one host run.
pub struct TextRuntime {
    service: HostTextService,
    identity_profile: TextProfile,
    missing: Option<String>,
    kind: BackendKind,
    identity_model: Option<String>,
    missing_variable: Option<&'static str>,
    store: Arc<MemoryAttemptStore>,
    in_flight: Arc<AtomicUsize>,
    record: Arc<dyn Fn(String) + Send + Sync>,
}

/// What a runtime is built over and how it stands, for the services report. It never holds a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextStatus {
    pub backend: BackendKind,
    pub profile: String,
    pub identity_model: Option<String>,
    pub configured: bool,
    /// The variable to set when the backend is not configured.
    pub missing: Option<&'static str>,
    /// Attempts the in-memory store keeps (one per operation asked this run).
    pub attempts_kept: usize,
    /// Backend calls under way.
    pub in_flight: usize,
}

fn owner_token() -> AttemptOwner {
    let mut bytes = [0_u8; 16];
    bytes[..4].copy_from_slice(&std::process::id().to_be_bytes());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |elapsed| elapsed.as_nanos());
    bytes[4..].copy_from_slice(&nanos.to_be_bytes()[4..]);
    bytes[15] |= 1;
    AttemptOwner::new(bytes).unwrap_or_else(|| unreachable!("the last byte is never zero"))
}

impl TextRuntime {
    /// The text surface over the backend this process's environment chooses.
    pub fn from_environment(record: Arc<dyn Fn(String) + Send + Sync>) -> Self {
        Self::new(BackendChoice::from_environment(), record)
    }

    /// The text surface over `choice`.
    pub fn new(choice: BackendChoice, record: Arc<dyn Fn(String) + Send + Sync>) -> Self {
        let profile = TextProfile::new(&choice.profile_label())
            .or_else(|_| TextProfile::new("host"))
            .unwrap_or_else(|_| unreachable!("a short plain label is a valid profile"));
        let configured = choice.is_configured();
        let missing = (!configured).then(|| choice.missing());
        let missing_variable = (!configured).then(|| choice.missing_variable());
        let (kind, identity_model) = (choice.kind(), choice.identity_model());
        record(format!(
            "TEXT runtime backend={} profile={} configured={configured} missing={}",
            kind.key(),
            profile.as_str(),
            missing_variable.unwrap_or("-")
        ));
        let owner = owner_token();
        let mut basis = [1_u8; 32];
        basis[..16].copy_from_slice(owner.as_bytes());
        let (store, in_flight) = (
            Arc::new(MemoryAttemptStore::default()),
            Arc::new(AtomicUsize::new(0)),
        );
        let backend = choice.into_backend(Arc::clone(&record), Arc::clone(&in_flight));
        let service = DurableTextService::new(
            TextService::new(backend).with_observer(LineObserver {
                record: Arc::clone(&record),
            }),
            Arc::clone(&store),
            Arc::new(ProcessAuthority { basis }),
            owner,
        );
        Self {
            service,
            identity_profile: profile,
            missing,
            kind,
            identity_model,
            missing_variable,
            store,
            in_flight,
            record,
        }
    }

    /// Submits `request` under `key` to the service, after one `TEXT started` line naming the
    /// operation and its deadline. Every host feature submits through here.
    pub async fn submit(
        &self,
        key: &OperationKey,
        request: &TextRequest,
    ) -> Result<Submission, DurableTextError> {
        (self.record)(format!(
            "TEXT started purpose={} profile={} scope={} operation={} deadline_ms={}",
            request.purpose.key(),
            request.profile.as_str(),
            key.scope().as_str(),
            key.operation(),
            request.deadline.as_millis()
        ));
        self.service.submit(key, request).await
    }

    /// The backend, its profile and how it stands now.
    pub fn status(&self) -> TextStatus {
        TextStatus {
            backend: self.kind,
            profile: self.identity_profile.as_str().to_owned(),
            identity_model: self.identity_model.clone(),
            configured: self.missing.is_none(),
            missing: self.missing_variable,
            attempts_kept: self.store.len(),
            in_flight: self.in_flight.load(Ordering::Relaxed),
        }
    }

    pub fn service(&self) -> &HostTextService {
        &self.service
    }

    /// The profile (backend and model) character-identity requests are made under.
    pub fn identity_profile(&self) -> &TextProfile {
        &self.identity_profile
    }

    /// What to set before asking, when the chosen backend is not configured.
    pub fn missing(&self) -> Option<&str> {
        self.missing.as_deref()
    }

    /// The operation key of a host feature's request: `scope` and an operation id.
    pub fn key(scope: &str, operation: &str) -> Option<OperationKey> {
        let scope = AttemptScope::new(scope).ok()?;
        OperationKey::new(&scope, operation).ok()
    }
}

#[cfg(test)]
mod text_tests;
