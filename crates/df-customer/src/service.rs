//! The customer account service: shared state, startup, store deadlines and the owned tasks.
//!
//! The service never runs on the game owner thread and never calls into the game. Its mail,
//! login-refusal and retention tasks are owned by `ServiceTasks` and stop when it drops.

use std::{
    future::Future,
    net::IpAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use df_auth::throttle::ThrottleDecision;
use df_types::{AccountId, TenantId};
use tokio::{sync::mpsc, task::JoinSet};

use crate::{
    clock::Clock,
    common_passwords::CommonPasswords,
    email::{Message, MessageBuilder},
    hosts::HostConfig,
    password_hashing::{HashError, HashingParameters, PasswordHasher},
    ports::{AccountStore, EmailSender, StoreError, StoreResult},
    provider::{IdentityFailure, IdentitySource},
    records::{AccountEvent, DeviceLabel},
    throttle::{Subject, ThrottleScope, Throttles},
    tokens::RandomUnavailable,
    tunables::Tunables,
};

/// Every store call's deadline. A mutation that passes it is `Unknown`, never failed.
pub const STORE_DEADLINE: Duration = Duration::from_secs(5);
/// Every email send's deadline.
pub const MAIL_DEADLINE: Duration = Duration::from_secs(10);
/// Every identity provider call's deadline. A mutation that passes it is `Unknown`.
pub const PROVIDER_DEADLINE: Duration = Duration::from_secs(10);
/// Mail jobs waiting for the worker; a full queue refuses the request visibly (`Busy`).
pub const MAIL_QUEUE_CAPACITY: usize = 256;
/// Login refusal jobs wait independently of mail delivery and reset jobs.
pub const LOGIN_EVENT_QUEUE_CAPACITY: usize = 256;
/// Maximum time spent on each refusal job, including the unknown-address no-op.
pub const LOGIN_EVENT_JOB_BUDGET: Duration = Duration::from_secs(2);
/// Refusal queue overflow audit lines are limited to this interval.
const LOGIN_EVENT_OVERFLOW_AUDIT_INTERVAL: Duration = Duration::from_secs(60);
/// How often the retention pass runs.
pub const RETENTION_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Service settings chosen by composition.
pub struct CustomerConfig {
    /// Public hostnames: link bases, mail From and Reply-To (decision 32).
    pub hosts: HostConfig,
    /// Where the credentials live: local passwords (development) or an identity provider.
    pub identity: IdentitySource,
    pub tunables: Tunables,
    pub hashing: HashingParameters,
    pub maximum_concurrent_hashes: usize,
    pub hash_permit_wait: Duration,
    /// Provider sign-up responses and failed sign-ins take at least this long, hiding the extra
    /// work for registered addresses. Hosted composition sets this above the slow paths.
    pub provider_signup_floor: Duration,
    /// Development only: allows local password hashes on a deployment whose origin is a public
    /// `https` one. Without it such a start is refused (local passwords are for development and
    /// self-hosting; hosted production uses a provider).
    pub dev_allow_local_passwords_on_https: bool,
}

/// Who is asking, as far as the service needs to know.
#[derive(Clone, Copy, Debug)]
pub struct Client {
    pub ip: Option<IpAddr>,
    pub device: DeviceLabel,
}

/// A failure the person sees as "try again". Never a domain refusal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceError {
    /// The store failed and made no change.
    Unavailable,
    /// A mutation's outcome is unknown; the person should look before retrying.
    Unknown,
    /// Too much work in flight (hashing permits or the mail queue).
    Busy,
    /// A stored row failed validation.
    InvalidRecord,
    /// The random source failed; nothing was issued.
    RandomUnavailable,
}

impl From<StoreError> for ServiceError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Unavailable => Self::Unavailable,
            StoreError::Unknown => Self::Unknown,
            StoreError::InvalidRecord => Self::InvalidRecord,
        }
    }
}

impl From<HashError> for ServiceError {
    fn from(error: HashError) -> Self {
        match error {
            HashError::Busy => Self::Busy,
            HashError::Failed => Self::Unavailable,
        }
    }
}

impl From<RandomUnavailable> for ServiceError {
    fn from(_: RandomUnavailable) -> Self {
        Self::RandomUnavailable
    }
}

impl ServiceError {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Unknown => "unknown",
            Self::Busy => "busy",
            Self::InvalidRecord => "invalid_record",
            Self::RandomUnavailable => "random_unavailable",
        }
    }
}

pub(crate) enum MailJob {
    Deliver(Message),
    /// A reset request: the worker looks the address up, so the request handler's timing does
    /// not depend on whether the account exists (plan 6.3).
    PasswordReset {
        normalized_email: String,
    },
}

pub(crate) struct LoginRefusalJob {
    pub(crate) normalized_email: String,
    pub(crate) event: AccountEvent,
}

pub(crate) struct Shared<S, M> {
    pub(crate) store: S,
    pub(crate) mail: M,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) hasher: PasswordHasher,
    pub(crate) throttles: Throttles,
    pub(crate) messages: MessageBuilder,
    pub(crate) identity: IdentitySource,
    pub(crate) signup_floor: Duration,
    pub(crate) sign_in_failure_floor: Duration,
    pub(crate) current_terms_version: String,
    pub(crate) tunables: Tunables,
    pub(crate) common_passwords: Arc<dyn CommonPasswords>,
    pub(crate) mail_queue: mpsc::Sender<MailJob>,
    pub(crate) login_event_queue: mpsc::Sender<LoginRefusalJob>,
    pub(crate) login_event_queue_dropped: AtomicU64,
    login_event_overflow_audit_at: Mutex<Option<Instant>>,
}

/// The account service. Cheap to clone; every clone shares one state.
pub struct CustomerService<S, M> {
    pub(crate) shared: Arc<Shared<S, M>>,
    pub(crate) google: Option<Arc<crate::oauth::OAuthState>>,
}

impl<S, M> Clone for CustomerService<S, M> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            google: self.google.clone(),
        }
    }
}

/// The service's owned background tasks. Dropping it (or `shutdown`) stops them.
pub struct ServiceTasks {
    tasks: JoinSet<()>,
}

impl ServiceTasks {
    pub async fn shutdown(mut self) {
        self.tasks.shutdown().await;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartError {
    RandomUnavailable,
    Hashing(HashError),
    /// Local passwords were configured for a public `https` origin without the explicit
    /// development flag.
    LocalPasswordsOnHostedOrigin,
}

impl<S: AccountStore, M: EmailSender> CustomerService<S, M> {
    /// Returns the account owner's usable personal tenant under the normal store deadline.
    pub async fn tenant_for(&self, account: AccountId) -> Result<Option<TenantId>, ServiceError> {
        read(self.shared.store.tenant_for(account)).await
    }

    /// Builds the service and starts its mail worker and retention pass.
    pub async fn start(
        config: CustomerConfig,
        store: S,
        mail: M,
        clock: Arc<dyn Clock>,
        common_passwords: Arc<dyn CommonPasswords>,
    ) -> Result<(Self, ServiceTasks), StartError> {
        if config.identity.is_local()
            && config.hosts.origin().as_str().starts_with("https://")
            && !config.dev_allow_local_passwords_on_https
        {
            return Err(StartError::LocalPasswordsOnHostedOrigin);
        }
        let hasher = PasswordHasher::new(
            config.hashing,
            config.maximum_concurrent_hashes,
            config.hash_permit_wait,
        )
        .await
        .map_err(StartError::Hashing)?;
        let throttles = Throttles::new().map_err(|_| StartError::RandomUnavailable)?;
        let (mail_queue, receiver) = mpsc::channel(MAIL_QUEUE_CAPACITY);
        let (login_event_queue, login_event_receiver) = mpsc::channel(LOGIN_EVENT_QUEUE_CAPACITY);
        let service = Self {
            google: None,
            shared: Arc::new(Shared {
                store,
                mail,
                clock,
                hasher,
                throttles,
                messages: MessageBuilder::new(&config.hosts),
                identity: config.identity,
                signup_floor: config.provider_signup_floor,
                sign_in_failure_floor: config.tunables.sign_in_failure_floor,
                current_terms_version: config.tunables.current_terms_version.clone(),
                tunables: config.tunables,
                common_passwords,
                mail_queue,
                login_event_queue,
                login_event_queue_dropped: AtomicU64::new(0),
                login_event_overflow_audit_at: Mutex::new(None),
            }),
        };
        let mut tasks = JoinSet::new();
        tasks.spawn(crate::mail_worker::run(
            Arc::clone(&service.shared),
            receiver,
        ));
        tasks.spawn(crate::mail_worker::run_login_events(
            Arc::clone(&service.shared),
            login_event_receiver,
        ));
        tasks.spawn(crate::retention::run(Arc::clone(&service.shared)));
        Ok((service, ServiceTasks { tasks }))
    }

    /// Runs one retention pass now (tests move the clock past a closing window and call this).
    #[cfg(any(test, feature = "test-fakes"))]
    pub async fn run_retention_pass(&self) {
        crate::retention::pass(&self.shared).await;
    }

    pub(crate) fn now_ms(&self) -> u64 {
        self.shared.clock.now_ms()
    }

    pub(crate) async fn pad_sign_in_failure(&self, started: std::time::Instant) {
        tokio::time::sleep_until(tokio::time::Instant::from_std(
            started + self.shared.sign_in_failure_floor,
        ))
        .await;
    }

    pub(crate) fn queue_login_refusal(&self, normalized_email: &str, event: AccountEvent) {
        if self
            .shared
            .login_event_queue
            .try_send(LoginRefusalJob {
                normalized_email: normalized_email.to_owned(),
                event,
            })
            .is_err()
        {
            let dropped = self
                .shared
                .login_event_queue_dropped
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1);
            let now = Instant::now();
            let mut last_audit = self
                .shared
                .login_event_overflow_audit_at
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let should_audit = last_audit.is_none_or(|last| {
                now.saturating_duration_since(last) >= LOGIN_EVENT_OVERFLOW_AUDIT_INTERVAL
            });
            if should_audit {
                *last_audit = Some(now);
                audit(
                    "account.login_event_queue_full",
                    format!("ACCOUNT login_event_queue_full dropped_total={dropped}"),
                );
            }
        }
    }

    /// Number of best-effort login refusal events dropped because their queue was full.
    pub fn login_refusal_queue_dropped(&self) -> u64 {
        self.shared
            .login_event_queue_dropped
            .load(Ordering::Relaxed)
    }

    pub fn tunables(&self) -> &Tunables {
        &self.shared.tunables
    }

    /// Whether accounts keep local password hashes (development) rather than at a provider.
    pub fn uses_local_passwords(&self) -> bool {
        self.shared.identity.is_local()
    }

    pub fn current_terms_version(&self) -> &str {
        &self.shared.current_terms_version
    }

    /// Verifications run so far: a test statistic for the equal-timing tests.
    pub fn verification_count(&self) -> u64 {
        self.shared.hasher.verification_count()
    }

    /// Counts one attempt against `scope` and says whether it may proceed.
    pub(crate) fn admit(&self, scope: ThrottleScope, subject: Subject<'_>) -> bool {
        let policy = self.shared.tunables.policy(scope);
        let decision = self
            .shared
            .throttles
            .admit(scope, subject, policy, self.now_ms());
        self.record_throttle(scope, decision)
    }

    /// Whether an attempt may proceed, without counting it (for failure-only limits).
    pub(crate) fn allowed(&self, scope: ThrottleScope, subject: Subject<'_>) -> bool {
        let policy = self.shared.tunables.policy(scope);
        let decision = self
            .shared
            .throttles
            .check(scope, subject, policy, self.now_ms());
        self.record_throttle(scope, decision)
    }

    pub(crate) fn count_failure(&self, scope: ThrottleScope, subject: Subject<'_>) {
        let policy = self.shared.tunables.policy(scope);
        self.shared
            .throttles
            .count(scope, subject, policy, self.now_ms());
    }

    fn record_throttle(&self, scope: ThrottleScope, decision: ThrottleDecision) -> bool {
        match decision {
            ThrottleDecision::Allowed => true,
            ThrottleDecision::Throttled { retry_after_ms } => {
                df_observe::record_audit_line(
                    "account.throttled",
                    format!(
                        "ACCOUNT throttled scope={} retry_after_ms={retry_after_ms}",
                        scope.label()
                    ),
                );
                false
            }
        }
    }

    /// Reserves a mail queue slot before any state changes, so a full queue refuses the whole
    /// request instead of committing an account whose message is then dropped.
    pub(crate) fn reserve_mail(&self) -> Result<mpsc::Permit<'_, MailJob>, ServiceError> {
        self.shared.mail_queue.try_reserve().map_err(|_| {
            df_observe::record_audit_line("email.queue_full", "EMAIL queue_full".to_owned());
            ServiceError::Busy
        })
    }

    pub(crate) fn is_common(&self) -> impl Fn(&str) -> bool + '_ {
        |candidate| self.shared.common_passwords.contains(candidate)
    }
}

/// Runs an identity provider call under the provider deadline. A late `mutation` may have
/// happened, so it is `Unknown`; a late read or check changed nothing.
pub(crate) async fn provider_call<T>(
    mutation: bool,
    work: impl Future<Output = Result<T, IdentityFailure>>,
) -> Result<T, ServiceError> {
    let late = if mutation {
        ServiceError::Unknown
    } else {
        ServiceError::Unavailable
    };
    match tokio::time::timeout(PROVIDER_DEADLINE, work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(IdentityFailure::Unavailable)) => Err(ServiceError::Unavailable),
        Ok(Err(IdentityFailure::Unknown)) => Err(ServiceError::Unknown),
        Ok(Err(IdentityFailure::Malformed)) => {
            audit(
                "account.provider_malformed",
                "ACCOUNT provider_malformed".to_owned(),
            );
            Err(ServiceError::Unavailable)
        }
        Err(_) => Err(late),
    }
}

/// Runs a store read under the deadline; a late read made no change.
pub(crate) async fn read<T>(work: impl Future<Output = StoreResult<T>>) -> Result<T, ServiceError> {
    match tokio::time::timeout(STORE_DEADLINE, work).await {
        Ok(result) => result.map_err(ServiceError::from),
        Err(_) => Err(ServiceError::Unavailable),
    }
}

/// Runs a store mutation under the deadline; a late mutation may have committed.
pub(crate) async fn write<T>(
    work: impl Future<Output = StoreResult<T>>,
) -> Result<T, ServiceError> {
    match tokio::time::timeout(STORE_DEADLINE, work).await {
        Ok(result) => result.map_err(ServiceError::from),
        Err(_) => Err(ServiceError::Unknown),
    }
}

pub(crate) fn account_label(account: AccountId) -> String {
    crate::tokens::hex(account.as_bytes())
}

pub(crate) fn audit(event: &'static str, line: String) {
    df_observe::record_audit_line(event, line);
}
