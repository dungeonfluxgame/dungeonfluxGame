//! The durable text service as the gameplay process composes it: one attempt store, one local
//! authority, one observer and one owner token, shared by every caller. A caller supplies only its
//! backend, request profile and operation keys; the runtime supplies what the plan keeps out of
//! features (store, authority, logging). When no database is reachable the runtime keeps attempts
//! in memory, which still stops a duplicate send inside one process but not across a restart; the
//! `TEXT` start-up line says which one a run has.

use std::{
    future::Future,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use df_ai::{
    text::{TextBackend, TextEvent, TextObserver, TextOutcome, TextService, TextUsage},
    text_attempts::{
        Admission, AttemptMeta, AttemptOwner, AttemptScope, AttemptStoreError, Basis,
        DurableTextService, OperationKey, QualifyResult, StoredAttempt, TextAttemptStore,
        TextAuthority, Verdict,
    },
    text_fixture::MemoryAttemptStore,
};
use df_observe::DiagnosticSink;
use df_persistence::text_attempts::PostgresTextAttempts;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{text_deadline::DeadlineBackend, text_telemetry::TextTelemetry};

/// The attempt store a run has: PostgreSQL when the game database answered, memory otherwise.
#[derive(Clone)]
pub(super) enum RuntimeStore {
    Postgres(Arc<PostgresTextAttempts>),
    Memory(Arc<MemoryAttemptStore>),
}

macro_rules! delegate {
    ($self:ident, $method:ident($($argument:expr),*)) => {
        async move {
            match $self {
                RuntimeStore::Postgres(store) => store.$method($($argument),*).await,
                RuntimeStore::Memory(store) => store.$method($($argument),*).await,
            }
        }
    };
}

impl TextAttemptStore for RuntimeStore {
    fn lookup(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<Option<StoredAttempt>, AttemptStoreError>> + Send {
        delegate!(self, lookup(key))
    }

    fn admit(
        &self,
        key: &OperationKey,
        meta: &AttemptMeta<'_>,
    ) -> impl Future<Output = Result<Admission, AttemptStoreError>> + Send {
        delegate!(self, admit(key, meta))
    }

    fn begin_dispatch(
        &self,
        key: &OperationKey,
        owner: AttemptOwner,
        basis: Basis,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        delegate!(self, begin_dispatch(key, owner, basis))
    }

    fn record_candidate(
        &self,
        key: &OperationKey,
        answer: &Value,
        usage: Option<TextUsage>,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        delegate!(self, record_candidate(key, answer, usage))
    }

    fn settle(
        &self,
        key: &OperationKey,
        outcome: &TextOutcome,
        usage: Option<TextUsage>,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        delegate!(self, settle(key, outcome, usage))
    }

    fn qualify(
        &self,
        key: &OperationKey,
        verdict: Verdict,
        basis_now: Option<Basis>,
    ) -> impl Future<Output = Result<QualifyResult, AttemptStoreError>> + Send {
        delegate!(self, qualify(key, verdict, basis_now))
    }

    fn invalidate(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        delegate!(self, invalidate(key))
    }

    fn mark_unknown(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        delegate!(self, mark_unknown(key))
    }

    fn cancel(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        delegate!(self, cancel(key))
    }
}

/// The owner of a local run: it holds one basis for the whole run and lets go of it at shutdown,
/// so an answer that lands after the run ended is recorded as stale instead of being reused.
pub(super) struct LocalAuthority {
    basis: Mutex<Option<Basis>>,
}

impl LocalAuthority {
    fn holding(basis: Basis) -> Self {
        Self {
            basis: Mutex::new(Some(basis)),
        }
    }

    /// The owner no longer holds the run: every later check answers `None`.
    pub(super) fn release(&self) {
        *self.basis.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

impl TextAuthority for LocalAuthority {
    fn current_basis(&self, _: &OperationKey) -> impl Future<Output = Option<Basis>> + Send {
        let basis = *self.basis.lock().unwrap_or_else(PoisonError::into_inner);
        std::future::ready(basis)
    }
}

/// Logs through the shared telemetry sink when the run has one.
pub(super) enum RuntimeObserver {
    Logged(Arc<TextTelemetry>),
    Silent,
}

impl TextObserver for RuntimeObserver {
    fn observe(&self, event: &TextEvent) {
        if let Self::Logged(telemetry) = self {
            telemetry.observe(event);
        }
    }
}

/// Everything one run shares across text callers.
pub(super) struct TextRuntime {
    store: RuntimeStore,
    authority: Arc<LocalAuthority>,
    telemetry: Option<Arc<TextTelemetry>>,
    owner: AttemptOwner,
    scope: AttemptScope,
}

static CURRENT: Mutex<Option<Arc<TextRuntime>>> = Mutex::new(None);

fn locked() -> MutexGuard<'static, Option<Arc<TextRuntime>>> {
    CURRENT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Installs the run's text runtime. `run` names the game (its database); the run's basis and scope
/// derive from it. Returns `false` when the process owner token could not be formed.
pub(super) fn install(
    store: RuntimeStore,
    sink: Option<Arc<dyn DiagnosticSink>>,
    run: &str,
) -> bool {
    let (Some(owner), Ok(scope)) = (process_owner(), AttemptScope::new(&format!("run:{run}")))
    else {
        return false;
    };
    let runtime = TextRuntime {
        store,
        authority: Arc::new(LocalAuthority::holding(run_basis(run, &owner))),
        telemetry: sink.map(|sink| Arc::new(TextTelemetry::new(sink))),
        owner,
        scope,
    };
    *locked() = Some(Arc::new(runtime));
    true
}

pub(super) fn current() -> Option<Arc<TextRuntime>> {
    locked().clone()
}

/// The owner lets go of the run: answers still in flight are discarded as stale.
pub(super) fn release() {
    if let Some(runtime) = current() {
        runtime.authority.release();
    }
}

#[cfg(test)]
pub(super) fn uninstall() {
    *locked() = None;
}

pub(super) type RuntimeService<B> =
    DurableTextService<DeadlineBackend<B>, RuntimeStore, Arc<LocalAuthority>, RuntimeObserver>;

impl TextRuntime {
    /// The scope this run's operation keys live in.
    pub(super) fn scope(&self) -> &AttemptScope {
        &self.scope
    }

    /// The durable service for one backend. Every caller gets the same store, authority, observer
    /// and owner, and every backend call is bounded by its request deadline; none of them reach a
    /// backend any other way.
    pub(super) fn service<B: TextBackend>(&self, backend: B) -> RuntimeService<B> {
        let observer = self
            .telemetry
            .clone()
            .map_or(RuntimeObserver::Silent, RuntimeObserver::Logged);
        DurableTextService::new(
            TextService::new(DeadlineBackend::new(backend)).with_observer(observer),
            self.store.clone(),
            self.authority.clone(),
            self.owner,
        )
    }
}

/// One owner token per process start, from the process id and the start time.
pub(super) fn process_owner() -> Option<AttemptOwner> {
    let mut bytes = [0_u8; 16];
    bytes[..4].copy_from_slice(&std::process::id().to_be_bytes());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    bytes[4..].copy_from_slice(&nanos.to_be_bytes()[4..]);
    AttemptOwner::new(bytes)
}

/// The basis a run holds: its identity and the process that started it.
fn run_basis(run: &str, owner: &AttemptOwner) -> Basis {
    let mut hasher = Sha256::new();
    hasher.update(b"local-run/1");
    hasher.update(run.as_bytes());
    hasher.update(owner.as_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_ai::{
        text::{TextProfile, TextPurpose, TextRequest},
        text_attempts::Submission,
        text_fixture::ScriptedBackend,
    };
    use std::{sync::atomic::AtomicUsize, time::Duration};

    fn request() -> TextRequest {
        TextRequest {
            purpose: TextPurpose::Storyteller,
            profile: TextProfile::new("runtime-test").unwrap(),
            instructions: "Narrate one line.".to_owned(),
            facts: serde_json::json!({"room": "harbor"}),
            output_name: "narration".to_owned(),
            output_schema: serde_json::json!({"type": "object"}),
            deadline: Duration::from_secs(30),
        }
    }

    fn runtime() -> Arc<TextRuntime> {
        assert!(install(
            RuntimeStore::Memory(Arc::new(MemoryAttemptStore::default())),
            None,
            "run-1",
        ));
        current().unwrap()
    }

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn a_runtime_service_sends_once_and_replays_the_candidate_for_the_same_operation() {
        let runtime = runtime();
        let calls = Arc::new(AtomicUsize::new(0));
        let backend = ScriptedBackend::new(
            [TextOutcome::Answered(serde_json::json!({"line": "ok"}))],
            calls.clone(),
        );
        let service = runtime.service(backend);
        let key = OperationKey::new(runtime.scope(), "turn-1").unwrap();
        let first = block_on(service.submit(&key, &request()));
        let again = block_on(service.submit(&key, &request()));
        assert!(matches!(
            first,
            Ok(Submission::Candidate {
                replayed: false,
                ..
            })
        ));
        assert!(matches!(
            again,
            Ok(Submission::Candidate { replayed: true, .. })
        ));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        uninstall();
    }

    #[test]
    fn releasing_the_run_makes_a_later_request_unauthorized() {
        let runtime = runtime();
        release();
        let backend = ScriptedBackend::new(
            [TextOutcome::Answered(serde_json::json!({"line": "ok"}))],
            Arc::new(AtomicUsize::new(0)),
        );
        let key = OperationKey::new(runtime.scope(), "turn-2").unwrap();
        let result = block_on(runtime.service(backend).submit(&key, &request()));
        assert_eq!(
            result,
            Err(df_ai::text_attempts::DurableTextError::Unauthorized)
        );
        uninstall();
    }
}
