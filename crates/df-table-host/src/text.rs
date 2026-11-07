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

mod ask;
mod cause;
pub mod codex_text;
mod limits;
mod model;
pub mod responses_text;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use df_ai::text::{
    TextBackend, TextEvent, TextObserver, TextProfile, TextReceipt, TextRequest, TextService,
    TextUsage,
};
use df_ai::text_attempts::{
    AttemptOwner, AttemptScope, Basis, DurableTextService, OperationKey, TextAuthority,
};
use df_ai::text_fixture::MemoryAttemptStore;
use serde::{Deserialize, Serialize};

use self::codex_text::{CodexRoute, CodexTextBackend};
use self::responses_text::{ResponsesRoute, ResponsesTextBackend};
use crate::environment;

pub use self::ask::{Asked, TextAskError};
pub use self::cause::{ServiceReason, TextServiceCause};
pub use self::limits::{MAXIMUM_MODEL_ROUTES, TEXT_DEADLINE};
pub use self::model::{MAXIMUM_MODEL_BYTES, ModelCause, check_model};

/// The environment variable that picks the backend: `responses` or `codex`.
pub const BACKEND_VARIABLE: &str = "DF_TEXT_BACKEND";
/// The environment variables the Responses key is read from, in order.
pub const KEY_VARIABLES: [&str; 2] = ["DF_OPENAI_API_KEY", "OPENAI_API_KEY"];
/// The environment variable that picks the model for character identity.
pub const IDENTITY_MODEL_VARIABLE: &str = "DF_TEXT_IDENTITY_MODEL";
/// The environment variable that picks the model for persona calls only; without it they use
/// [`DEFAULT_IDENTITY_MODEL`] (GPT-6 Luna) on either backend. A request can still name its own
/// model, such as a smarter one.
pub const PERSONA_MODEL_VARIABLE: &str = "DF_TEXT_PERSONA_MODEL";
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
    /// A script of answers, for the host's tests.
    #[cfg(test)]
    Scripted(Arc<df_ai::text_fixture::ScriptedBackend>),
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
            #[cfg(test)]
            TextAdapter::Scripted(backend) => backend.complete(request).await,
        }
    }
}

/// Which backend a runtime is built over, and with what route (none: unconfigured).
#[derive(Clone)]
pub enum BackendChoice {
    Responses(Option<ResponsesRoute>),
    Codex(Option<CodexRoute>),
    /// A script of answers and the model it is asked for, for the host's tests.
    #[cfg(test)]
    Scripted(Arc<df_ai::text_fixture::ScriptedBackend>, Option<String>),
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

    /// This choice over `model` instead of its own: the same backend, key and executable.
    fn with_model(&self, model: &str) -> Self {
        match self {
            Self::Responses(route) => Self::Responses(route.as_ref().map(|route| ResponsesRoute {
                model: model.to_owned(),
                key: route.key.clone(),
            })),
            Self::Codex(route) => Self::Codex(route.as_ref().map(|route| CodexRoute {
                model: Some(model.to_owned()),
                ..route.clone()
            })),
            #[cfg(test)]
            Self::Scripted(script, _) => Self::Scripted(Arc::clone(script), Some(model.to_owned())),
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
            #[cfg(test)]
            Self::Scripted(_, model) => match model {
                Some(model) => format!("scripted:{model}"),
                None => "scripted".to_owned(),
            },
        }
    }

    fn is_configured(&self) -> bool {
        match self {
            Self::Responses(route) => route.is_some(),
            Self::Codex(route) => route.is_some(),
            #[cfg(test)]
            Self::Scripted(..) => true,
        }
    }

    fn kind(&self) -> BackendKind {
        match self {
            Self::Responses(_) => BackendKind::Responses,
            Self::Codex(_) => BackendKind::Codex,
            #[cfg(test)]
            Self::Scripted(..) => BackendKind::Codex,
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
            #[cfg(test)]
            Self::Scripted(_, model) => model.clone(),
        }
    }

    /// The one variable to set when the choice is not configured.
    fn missing_variable(&self) -> &'static str {
        match self {
            Self::Responses(_) => KEY_VARIABLES[1],
            Self::Codex(_) => codex_text::EXECUTABLE_VARIABLE,
            #[cfg(test)]
            Self::Scripted(..) => "",
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
            #[cfg(test)]
            Self::Scripted(..) => String::new(),
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
            #[cfg(test)]
            Self::Scripted(script, _) => TextAdapter::Scripted(script),
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

/// One text service over one model: the durable service, the profile its requests are made under
/// and the recorder of its `TEXT started` lines. `ask` and `submit` are in `ask`.
pub struct TextRoute {
    service: HostTextService,
    profile: TextProfile,
    record: Arc<dyn Fn(String) + Send + Sync>,
}

impl TextRoute {
    /// The profile (backend and model) this route's requests are made under.
    pub fn profile(&self) -> &TextProfile {
        &self.profile
    }
}

/// What every route of a runtime shares: the recorder, the count of calls under way, and the
/// store, authority and owner, so a route for another model is the same service over another
/// adapter.
struct RouteParts {
    record: Arc<dyn Fn(String) + Send + Sync>,
    in_flight: Arc<AtomicUsize>,
    store: Arc<MemoryAttemptStore>,
    authority: Arc<ProcessAuthority>,
    owner: AttemptOwner,
}

impl RouteParts {
    fn route(&self, choice: BackendChoice, profile: TextProfile) -> TextRoute {
        let backend = choice.into_backend(Arc::clone(&self.record), Arc::clone(&self.in_flight));
        let service = DurableTextService::new(
            TextService::new(backend).with_observer(LineObserver {
                record: Arc::clone(&self.record),
            }),
            Arc::clone(&self.store),
            Arc::clone(&self.authority),
            self.owner,
        );
        TextRoute {
            service,
            profile,
            record: Arc::clone(&self.record),
        }
    }
}

/// The composed text surface of one host run: a route over the backend's default model, and one
/// route for each other model a request names, built on its first use.
pub struct TextRuntime {
    default_route: Arc<TextRoute>,
    other_routes: Mutex<HashMap<String, Arc<TextRoute>>>,
    choice: BackendChoice,
    persona_model: Option<String>,
    missing: Option<String>,
    kind: BackendKind,
    identity_model: Option<String>,
    missing_variable: Option<&'static str>,
    parts: RouteParts,
}

/// What a runtime is built over and how it stands, for the services report. It never holds a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextStatus {
    pub backend: BackendKind,
    pub profile: String,
    pub identity_model: Option<String>,
    /// The model persona calls use instead of the default, when `DF_TEXT_PERSONA_MODEL` says one.
    pub persona_model: Option<String>,
    /// The profile persona calls go under when they name no model: the persona model's, or the
    /// default's. None when the persona model is not a valid model name (persona calls refuse).
    pub persona_profile: Option<String>,
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
    /// The text surface over the backend this process's environment chooses, with the persona
    /// model this process's environment names, or GPT-6 Luna when it names none.
    pub fn from_environment(record: Arc<dyn Fn(String) + Send + Sync>) -> Self {
        let persona_model = environment::text(PERSONA_MODEL_VARIABLE)
            .unwrap_or_else(|| DEFAULT_IDENTITY_MODEL.to_owned());
        Self::new(BackendChoice::from_environment(), record).with_persona_model(Some(persona_model))
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
        let parts = RouteParts {
            record,
            in_flight: Arc::new(AtomicUsize::new(0)),
            store: Arc::new(MemoryAttemptStore::default()),
            authority: Arc::new(ProcessAuthority { basis }),
            owner,
        };
        let default_route = Arc::new(parts.route(choice.clone(), profile));
        Self {
            default_route,
            other_routes: Mutex::new(HashMap::new()),
            choice,
            persona_model: None,
            missing,
            kind,
            identity_model,
            missing_variable,
            parts,
        }
    }

    /// This runtime with `model` as the model persona calls use unless they name their own.
    #[must_use]
    pub fn with_persona_model(mut self, model: Option<String>) -> Self {
        self.persona_model = model;
        self
    }

    /// The profile label of requests over `model`, when `model` is a valid model name: the
    /// backend's label with the model in it, which `TextProfile` accepts for any name
    /// [`check_model`] does.
    fn model_profile(&self, model: &str) -> Result<TextProfile, ModelCause> {
        check_model(model)?;
        TextProfile::new(&self.choice.with_model(model).profile_label())
            .map_err(|_| ModelCause::TooLong)
    }

    /// The route over `model`, built the first time it is asked for. A route is the same service
    /// over another model, under its own profile, so its answers never replay for another
    /// model's.
    pub fn route_for_model(&self, model: &str) -> Result<Arc<TextRoute>, ModelCause> {
        let profile = self.model_profile(model)?;
        let mut routes = self
            .other_routes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(route) = routes.get(model) {
            return Ok(Arc::clone(route));
        }
        if routes.len() >= MAXIMUM_MODEL_ROUTES {
            return Err(ModelCause::TooManyModels);
        }
        let route = Arc::new(self.parts.route(self.choice.with_model(model), profile));
        routes.insert(model.to_owned(), Arc::clone(&route));
        Ok(route)
    }

    /// The route a persona call goes through: `requested` model if it names one, else the
    /// persona model, else the default.
    pub fn persona_route(&self, requested: Option<&str>) -> Result<Arc<TextRoute>, ModelCause> {
        match requested.or(self.persona_model.as_deref()) {
            Some(model) => self.route_for_model(model),
            None => Ok(Arc::clone(&self.default_route)),
        }
    }

    /// The backend, its profile and how it stands now.
    pub fn status(&self) -> TextStatus {
        let persona_profile = match self.persona_model.as_deref() {
            Some(model) => self
                .model_profile(model)
                .ok()
                .map(|profile| profile.as_str().to_owned()),
            None => Some(self.default_route.profile.as_str().to_owned()),
        };
        TextStatus {
            backend: self.kind,
            profile: self.default_route.profile.as_str().to_owned(),
            identity_model: self.identity_model.clone(),
            persona_model: self.persona_model.clone(),
            persona_profile,
            configured: self.missing.is_none(),
            missing: self.missing_variable,
            attempts_kept: self.parts.store.len(),
            in_flight: self.parts.in_flight.load(Ordering::Relaxed),
        }
    }

    pub fn service(&self) -> &HostTextService {
        &self.default_route.service
    }

    /// The profile (backend and model) character-identity requests are made under.
    pub fn identity_profile(&self) -> &TextProfile {
        &self.default_route.profile
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
