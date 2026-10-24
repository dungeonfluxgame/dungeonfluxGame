//! Implements the actual consumer-owned synchronous SessionRepository.
//! Native composition owns a dedicated joined actor thread and the existing runtime.
use std::marker::PhantomData;

use df_auth::membership::MembershipAuthority;
use df_model::checkpoint::{Basis, Checkpoint};
use df_observe::OperationContext;
use df_session::submission::{CommitOutcome, OperationLookup, RepositoryError, SessionRepository};
use tokio::runtime::Handle;

use crate::checkpoint_codec::CodecLimits;
use crate::decision_adapter::{DecisionAdapter, RecoverySource};
use crate::native_connection::actor_block_on;
use crate::native_connection::{DiscardState, OwnedConnection, TransactionBounds};
use crate::native_scope::DatabaseBindingVerifier;
use crate::native_scope::NativeScope;
use tokio::time::{Instant, timeout_at};
use tokio_postgres::{Client, Connection, Socket};

/// One owned connection belongs to one serialized actor. Connection loss requires native
/// reconstruction with a freshly verified scope; this type never replays a transaction.
pub struct PostgresRepository<A: MembershipAuthority> {
    runtime: Handle,
    adapter: DecisionAdapter,
    authority: PhantomData<fn() -> A>,
    reconnect: Option<(tokio_postgres::Config, NativeRepositoryOptions)>,
    pending_setup: Option<NativeSetupFailure>,
}
impl<A: MembershipAuthority> PostgresRepository<A> {
    /// Native construction from the exact real connected pair and the existing owner runtime.
    /// This NoTls boundary is qualified only for registered loopback fixtures. The static verifier
    /// source is trusted native configuration, never a request/DTO or authorization claim.
    /// Cancellation during setup requires the native owner to retain and join its setup task;
    /// dropping the setup future is emergency abort, never a completed-cleanup observation.
    pub async fn from_connected_no_tls(
        runtime: Handle,
        client: Client,
        connection: Connection<Socket, tokio_postgres::tls::NoTlsStream>,
        options: NativeRepositoryOptions,
        context: &OperationContext,
    ) -> Result<Self, NativeSetupFailure> {
        let mut span = df_observe::begin(context, "persistence.native_connection_setup");
        let result = Self::initialize_owned(runtime, client, connection, options).await;
        span.finish_unmeasured(match &result {
            Ok(_) => "native_owner_initialized",
            Err(failure) => match failure
                .pending_cleanup
                .as_deref()
                .map(OwnedConnection::discard_state)
            {
                Some(DiscardState::JoinPending) => "native_setup_failed_driver_join_pending",
                Some(DiscardState::JoinedFailed) => "native_setup_failed_driver_join_failed",
                Some(_) => "native_setup_failed_cleanup_required",
                None => "native_setup_failed_driver_closed",
            },
        });
        result
    }

    async fn initialize_owned(
        runtime: Handle,
        client: Client,
        connection: Connection<Socket, tokio_postgres::tls::NoTlsStream>,
        options: NativeRepositoryOptions,
    ) -> Result<Self, NativeSetupFailure> {
        options
            .validate()
            .map_err(NativeSetupFailure::before_driver)?;
        let mut owned = OwnedConnection::from_connected(
            &runtime,
            client,
            connection,
            options.transaction_bounds,
        )
        .map_err(NativeSetupFailure::before_driver)?;
        let verifier = match options.verifier {
            None => DatabaseBindingVerifier::unconfigured(),
            Some(source) => {
                let client = match owned.take_client() {
                    Ok(client) => client,
                    Err(error) => return Err(NativeSetupFailure::close_driver(error, owned).await),
                };
                let deadline =
                    match Instant::now().checked_add(options.transaction_bounds.transaction) {
                        Some(deadline) => deadline,
                        None => {
                            drop(client);
                            return Err(NativeSetupFailure::close_driver(
                                RepositoryError::Capacity,
                                owned,
                            )
                            .await);
                        }
                    };
                let statement = match timeout_at(deadline, client.prepare(source.query)).await {
                    Ok(Ok(statement)) => statement,
                    Ok(Err(_)) | Err(_) => {
                        drop(client);
                        return Err(NativeSetupFailure::close_driver(
                            RepositoryError::Unavailable,
                            owned,
                        )
                        .await);
                    }
                };
                if owned.return_client(client).is_err() {
                    return Err(NativeSetupFailure::close_driver(
                        RepositoryError::Unavailable,
                        owned,
                    )
                    .await);
                }
                DatabaseBindingVerifier {
                    statement: Some(statement),
                    maximum_binding_bytes: source.maximum_binding_bytes,
                    maximum_authority_value_bytes: source.maximum_authority_value_bytes,
                    maximum_namespace_bytes: source.maximum_namespace_bytes,
                }
            }
        };
        let adapter = DecisionAdapter::from_prechecked_owned(
            owned,
            options.transaction_bounds,
            options.codec_limits,
            options.maximum_receipt_bytes,
            verifier,
            options.recovery,
        );
        Ok(Self::from_owned(runtime, adapter))
    }

    pub(crate) fn from_owned(runtime: Handle, adapter: DecisionAdapter) -> Self {
        Self {
            runtime,
            adapter,
            authority: PhantomData,
            reconnect: None,
            pending_setup: None,
        }
    }

    /// Trusted native composition enables finite reconnect to the same admitted
    /// database/inventory. This configuration is never accepted from an RPC payload.
    /// Refusal retains this repository and its original connection owner.
    pub fn configure_reconnect(
        &mut self,
        configuration: tokio_postgres::Config,
        options: NativeRepositoryOptions,
    ) -> Result<(), RepositoryError> {
        options.validate()?;
        self.reconnect = Some((configuration, options));
        Ok(())
    }

    async fn reconnect_owned(&mut self) -> Result<(), RepositoryError> {
        if self.adapter.connection_usable() {
            return Ok(());
        }
        // A join timeout preserves ownership of the original poisoned adapter.
        self.adapter.close().await?;
        if let Some(failure) = self.pending_setup.as_mut() {
            failure.close().await?;
        }
        self.pending_setup.take();
        let (configuration, options) = self
            .reconnect
            .as_ref()
            .ok_or(RepositoryError::Unavailable)?;
        let (client, connection) = tokio::time::timeout(
            options.transaction_bounds.transaction,
            configuration.connect(tokio_postgres::NoTls),
        )
        .await
        .map_err(|_| RepositoryError::Unavailable)?
        .map_err(|_| RepositoryError::Unavailable)?;
        match Self::initialize_owned(self.runtime.clone(), client, connection, options.clone())
            .await
        {
            Ok(replacement) => {
                self.adapter = replacement.adapter;
                Ok(())
            }
            Err(failure) => {
                let error = failure.error();
                self.pending_setup = Some(failure);
                Err(error)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture_discard_state(&self) -> DiscardState {
        self.adapter.fixture_discard_state()
    }

    /// Joined shutdown must run on the actor thread before its owner joins that thread.
    /// Runtime-worker invocation refuses without issuing database work.
    pub fn close(&mut self) -> Result<(), RepositoryError> {
        let runtime = self.runtime.clone();
        actor_block_on(&runtime, self.close_owned())?
    }

    /// Setup/shutdown on the native root may join before the serialization actor starts.
    /// A failed close retains the repository and every pending driver for another attempt.
    pub async fn close_owned(&mut self) -> Result<(), RepositoryError> {
        self.adapter.close().await?;
        if let Some(failure) = self.pending_setup.as_mut() {
            failure.close().await?;
        }
        self.pending_setup.take();
        Ok(())
    }
}
impl<A: MembershipAuthority> SessionRepository for PostgresRepository<A>
where
    A::Principal: Send,
    A::Tenant: Send,
    A::Campaign: Send,
    A::Role: Send,
    A::Revision: Send,
{
    type Scope = NativeScope<A>;

    fn recover_connection(&mut self, context: &OperationContext) -> Result<(), RepositoryError> {
        let started = std::time::Instant::now();
        let mut span = df_observe::begin(context, "persistence.native_connection_recovery");
        let runtime = self.runtime.clone();
        let result = actor_block_on(&runtime, self.reconnect_owned()).and_then(|result| result);
        df_observe::record_audit_line(
            "persistence.connection",
            format!(
                "DATABASE connection event=recovery outcome={} state={} ms={}",
                if result.is_ok() { "ready" } else { "pending" },
                if self.adapter.connection_usable() {
                    "usable"
                } else {
                    "unusable"
                },
                started.elapsed().as_millis()
            ),
        );
        span.finish_unmeasured(if result.is_ok() {
            "connection_ready"
        } else {
            "recovery_pending"
        });
        result
    }

    fn lookup_operation(
        &mut self,
        scope: &Self::Scope,
        context: &OperationContext,
    ) -> Result<OperationLookup, RepositoryError> {
        let runtime = self.runtime.clone();
        actor_block_on(&runtime, self.adapter.lookup_scoped(scope, context))?
    }

    fn commit_decision(
        &mut self,
        scope: &Self::Scope,
        checkpoint: &Checkpoint,
        expected: Basis,
        context: &OperationContext,
    ) -> Result<CommitOutcome, RepositoryError> {
        let runtime = self.runtime.clone();
        actor_block_on(
            &runtime,
            self.adapter
                .commit_scoped(scope, checkpoint, expected, context),
        )?
    }

    fn load_current(
        &mut self,
        scope: &Self::Scope,
        context: &OperationContext,
    ) -> Result<Checkpoint, RepositoryError> {
        let runtime = self.runtime.clone();
        actor_block_on(&runtime, self.adapter.load_scoped(scope, context))?
    }
}

/// Finite native configuration; these budgets and inventories confer no caller authority.
/// `recovery` must come from the root's admitted sources, never from checkpoint payload bytes.
#[derive(Clone)]
pub struct NativeRepositoryOptions {
    pub transaction_bounds: TransactionBounds,
    pub codec_limits: CodecLimits,
    pub maximum_receipt_bytes: usize,
    pub verifier: Option<NativeVerifierSource>,
    pub recovery: Option<RecoverySource>,
}
/// Root-reviewed static SQL implementing the exact current-authority verifier ABI.
/// Statement text alone proves no permission: qualification includes its actual issuer and RLS.
#[derive(Clone)]
pub struct NativeVerifierSource {
    pub query: &'static str,
    pub maximum_binding_bytes: usize,
    pub maximum_authority_value_bytes: usize,
    pub maximum_namespace_bytes: usize,
}
impl NativeRepositoryOptions {
    fn validate(&self) -> Result<(), RepositoryError> {
        self.transaction_bounds.validate()?;
        self.codec_limits
            .validate()
            .map_err(|_| RepositoryError::Capacity)?;
        if self.maximum_receipt_bytes == 0 {
            return Err(RepositoryError::Capacity);
        }
        if let Some(source) = &self.verifier
            && (source.query.is_empty()
                || source.maximum_binding_bytes == 0
                || source.maximum_authority_value_bytes == 0
                || source.maximum_namespace_bytes == 0)
        {
            return Err(RepositoryError::Capacity);
        }
        if let Some(source) = &self.recovery
            && (source.limits.maximum_records == 0
                || source.limits.maximum_text_bytes == 0
                || source.limits.maximum_total_text_bytes == 0
                || source.limits.maximum_retained_bytes == 0)
        {
            return Err(RepositoryError::Capacity);
        }
        Ok(())
    }
}

/// Setup refusals own every still-pending aborted connection task until the root retries close.
/// A completed timeout is explicit failure, never evidence that a driver has physically joined.
pub struct NativeSetupFailure {
    error: RepositoryError,
    pending_cleanup: Option<Box<OwnedConnection>>,
}
impl NativeSetupFailure {
    fn before_driver(error: RepositoryError) -> Self {
        Self {
            error,
            pending_cleanup: None,
        }
    }
    async fn close_driver(error: RepositoryError, mut connection: OwnedConnection) -> Self {
        let pending_cleanup = match connection.discard().await {
            Ok(()) => None,
            Err(_) => Some(Box::new(connection)),
        };
        Self {
            error,
            pending_cleanup,
        }
    }
    pub fn error(&self) -> RepositoryError {
        self.error
    }
    pub fn cleanup_pending(&self) -> bool {
        self.pending_cleanup.is_some()
    }
    /// The existing owner runtime must poll this bounded retry while retaining this failure.
    /// A failed retry retains the owner; only completed close clears it.
    pub async fn close(&mut self) -> Result<(), RepositoryError> {
        if let Some(connection) = self.pending_cleanup.as_mut() {
            connection.close().await?;
        }
        self.pending_cleanup.take();
        Ok(())
    }
}
impl std::fmt::Debug for NativeSetupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeSetupFailure")
            .field("error", &self.error)
            .field("cleanup_pending", &self.cleanup_pending())
            .finish_non_exhaustive()
    }
}
