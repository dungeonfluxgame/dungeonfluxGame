//! The host's composition of the text surface: one `df-ai` durable text service, its store, its
//! authority and its observer, over one of the host's private backends.
//!
//! A host feature that needs a model builds a `df_ai::text::TextRequest` (purpose, profile,
//! instructions, facts, schema, pictures) and submits it here under a scoped operation key; it
//! checks the candidate with its own rule and qualifies it. Nothing in the host calls a model any
//! other way. The store is in memory (the dev host has no database): one send per operation per
//! process, and a qualified answer replays without asking again; a rejected answer is never
//! regenerated under its key. Each event becomes one `TEXT` audit line of labels, ids and times,
//! never prompts, pictures or answers.
//!
//! Layout: this module holds what every backend shares (the backend choice, environment reading,
//! the facts text and supplier usage); `responses_text` (OpenAI's Responses API) and `codex_text`
//! (the signed-in Codex CLI) hold only their own wire shapes. `DF_TEXT_BACKEND` picks one:
//! `responses` or `codex`; without it, Responses when an OpenAI key is set, else Codex.

pub mod codex_text;
mod limits;
pub mod responses_text;

use std::sync::Arc;

use df_ai::text::{
    TextBackend, TextEvent, TextObserver, TextProfile, TextReceipt, TextRequest, TextService,
    TextUsage,
};
use df_ai::text_attempts::{
    AttemptOwner, AttemptScope, Basis, DurableTextService, OperationKey, TextAuthority,
};
use df_ai::text_fixture::MemoryAttemptStore;

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

/// The host's backend: exactly one of the private adapters, chosen at start.
pub enum HostTextBackend {
    Responses(ResponsesTextBackend),
    Codex(CodexTextBackend),
}

impl TextBackend for HostTextBackend {
    async fn complete(&self, request: &TextRequest) -> TextReceipt {
        match self {
            Self::Responses(backend) => backend.complete(request).await,
            Self::Codex(backend) => backend.complete(request).await,
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

    fn into_backend(self, record: Arc<dyn Fn(String) + Send + Sync>) -> HostTextBackend {
        match self {
            Self::Responses(route) => HostTextBackend::Responses(ResponsesTextBackend::new(route)),
            Self::Codex(route) => HostTextBackend::Codex(CodexTextBackend::new(route, record)),
        }
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

impl TextObserver for LineObserver {
    fn observe(&self, event: &TextEvent) {
        let mut line = format!(
            "TEXT {} purpose={} profile={} output={} elapsed_ms={}",
            event.kind.label(),
            event.purpose.label(),
            event.profile.as_str(),
            event.output_name,
            event.elapsed.as_millis()
        );
        if let (Some(scope), Some(operation)) = (&event.scope, &event.operation) {
            line.push_str(&format!(" scope={scope} operation={operation}"));
        }
        if let Some(reason) = event.kind.reason() {
            line.push_str(&format!(" reason={reason}"));
        }
        (self.record)(line);
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
        let missing = (!choice.is_configured()).then(|| choice.missing());
        let owner = owner_token();
        let mut basis = [1_u8; 32];
        basis[..16].copy_from_slice(owner.as_bytes());
        let service = DurableTextService::new(
            TextService::new(choice.into_backend(Arc::clone(&record)))
                .with_observer(LineObserver { record }),
            Arc::new(MemoryAttemptStore::default()),
            Arc::new(ProcessAuthority { basis }),
            owner,
        );
        Self {
            service,
            identity_profile: profile,
            missing,
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
