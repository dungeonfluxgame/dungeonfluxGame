//! PostgreSQL operator authority. Each command commits its grant, audit and replay receipt
//! together. One owned connection bounds concurrency; dropped/timed-out transactions roll back.

use std::{sync::Arc, time::Duration};

use df_customer::{operator_contract::*, operator_ports::OperatorStore};
use df_types::{AccountId, OperationId, TenantId, WebSessionId};
use serde_json::{Value, json};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_postgres::{Client, Config, Row, Transaction};

use crate::postgres_tls::{ConnectError, PostgresTransport, check_transport, connect};

const CALL_DEADLINE: Duration = Duration::from_secs(12);
const SETTINGS: &str = "SET statement_timeout = '5s'; SET lock_timeout = '5s'; \
    SET transaction_timeout = '10s'; SET idle_in_transaction_session_timeout = '10s'";
const GRANT_COLUMNS: &str = "g.grant_id, g.account_id, g.capabilities, g.scope_kind, g.scope_id, \
    g.version, g.issued_at_ms, g.expires_at_ms, g.revoked_at_ms, g.enrolment_generation";

struct Connection {
    client: Client,
    driver: JoinHandle<()>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

struct Pool {
    config: Config,
    transport: PostgresTransport,
    connection: Mutex<Option<Connection>>,
}

impl Pool {
    async fn checkout(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<Connection>>, OperatorError> {
        // Serial ownership is held over I/O; dropping the future releases the slot and the
        // transaction's Drop queues rollback before the next command can use this connection.
        let mut slot = self.connection.lock().await;
        if slot
            .as_ref()
            .is_none_or(|connection| connection.client.is_closed())
        {
            let (client, connection) =
                connect(&self.config, &self.transport)
                    .await
                    .map_err(|error| match error {
                        ConnectError::Postgres(error) => storage_error(error),
                        ConnectError::Refused(reason) => {
                            df_observe::record_audit_line(
                                "operator.transport_refused",
                                format!("OPERATOR transport_refused reason={reason:?}"),
                            );
                            OperatorError::Unavailable
                        }
                    })?;
            let driver = tokio::spawn(async move {
                if let Err(error) = connection.await {
                    storage_error(error);
                }
            });
            let owned = Connection { client, driver };
            owned
                .client
                .batch_execute(SETTINGS)
                .await
                .map_err(storage_error)?;
            *slot = Some(owned);
        }
        Ok(slot)
    }
}

/// Clones share one bounded, reconnecting connection. Host composition applies migration first.
#[derive(Clone)]
pub struct PostgresOperatorStore(Arc<Pool>);

impl PostgresOperatorStore {
    pub async fn open(config: Config, transport: PostgresTransport) -> Result<Self, OperatorError> {
        check_transport(&config, &transport).map_err(|_| OperatorError::Unavailable)?;
        let store = Self(Arc::new(Pool {
            config,
            transport,
            connection: Mutex::new(None),
        }));
        tokio::time::timeout(CALL_DEADLINE, async {
            let slot = store.0.checkout().await?;
            let client = &slot.as_ref().ok_or(OperatorError::Unavailable)?.client;
            let applied = client
                .query_opt(
                    "SELECT 1 FROM accounts.schema_migration WHERE version = 35",
                    &[],
                )
                .await
                .map_err(storage_error)?
                .is_some();
            if !applied {
                return Err(OperatorError::Unavailable);
            }
            Ok(())
        })
        .await
        .map_err(|_| OperatorError::Unavailable)??;
        Ok(store)
    }
}

/// Migrator-only versioned schema application, serialized with the existing account migrations.
/// Runtime roles only read the migration record and cannot run this function's DDL.
pub async fn migrate_operator_access(client: &mut Client) -> Result<(), OperatorError> {
    tokio::time::timeout(CALL_DEADLINE, async {
        let transaction = client.transaction().await.map_err(storage_error)?;
        transaction
            .batch_execute(
                "SET LOCAL statement_timeout = '5s'; SET LOCAL lock_timeout = '5s'; \
            SET LOCAL transaction_timeout = '10s'; \
            SELECT pg_advisory_xact_lock(hashtext('df_accounts_migration'))",
            )
            .await
            .map_err(storage_error)?;
        if transaction
            .query_opt(
                "SELECT 1 FROM accounts.schema_migration WHERE version = 35",
                &[],
            )
            .await
            .map_err(storage_error)?
            .is_none()
        {
            transaction
                .batch_execute(include_str!("../migrations/0035_operator_access.sql"))
                .await
                .map_err(storage_error)?;
            transaction
                .execute(
                    "INSERT INTO accounts.schema_migration(version) VALUES (35)",
                    &[],
                )
                .await
                .map_err(storage_error)?;
        }
        transaction.commit().await.map_err(commit_error)
    })
    .await
    .map_err(|_| OperatorError::Unknown)?
}

impl OperatorStore for PostgresOperatorStore {
    async fn receipt(
        &self,
        command: &OperatorCommand,
    ) -> Result<Option<OperatorReceipt>, OperatorError> {
        tokio::time::timeout(CALL_DEADLINE, async {
            let slot = self.0.checkout().await?;
            let client = &slot.as_ref().ok_or(OperatorError::Unavailable)?.client;
            read_receipt(client, command).await
        })
        .await
        .map_err(|_| OperatorError::Unavailable)?
    }

    async fn execute(
        &self,
        command: OperatorCommand,
        now_ms: u64,
    ) -> Result<OperatorReceipt, OperatorError> {
        tokio::time::timeout(CALL_DEADLINE, async {
            let mut slot = self.0.checkout().await?;
            let client = &mut slot.as_mut().ok_or(OperatorError::Unavailable)?.client;
            let transaction = client.transaction().await.map_err(storage_error)?;
            // The receipt key serializes equal and conflicting retries, including different accounts.
            let operation_lock = i64::from_be_bytes(
                command.operation().as_bytes()[..8]
                    .try_into()
                    .map_err(|_| OperatorError::InvalidRecord)?,
            );
            transaction
                .query_one("SELECT pg_advisory_xact_lock($1)", &[&operation_lock])
                .await
                .map_err(storage_error)?;
            if let Some(receipt) = read_receipt(&transaction, &command).await? {
                return Ok(receipt);
            }
            let (grant, version, action, reason, correlation) = match &command {
                OperatorCommand::Grant(value) => {
                    let grant = insert_grant(&transaction, value, now_ms).await?;
                    (
                        grant,
                        1,
                        OperatorAuditAction::Grant,
                        &value.reason,
                        value.correlation,
                    )
                }
                OperatorCommand::Revoke(value) => {
                    let version = revoke_grant(&transaction, value, now_ms).await?;
                    (
                        value.grant,
                        version,
                        OperatorAuditAction::Revoke,
                        &value.reason,
                        value.correlation,
                    )
                }
            };
            let event = NewOperatorAudit {
                basis: OperatorAuditBasis::Host,
                action,
                target: OperatorAuditTarget::Grant(grant),
                reason: reason.clone(),
                operation: command.operation(),
                fingerprint: command.fingerprint(),
                outcome: OperatorAuditOutcome::Applied,
                correlation,
                at_ms: now_ms,
            };
            let audit = append_operator_audit(&transaction, command.account(), &event).await?;
            let receipt = OperatorReceipt {
                operation: command.operation(),
                grant,
                version,
                audit,
            };
            transaction
                .execute(
                    "INSERT INTO accounts.operator_receipt \
                (operation_id, fingerprint, grant_id, version, audit_id) VALUES ($1,$2,$3,$4,$5)",
                    &[
                        &&receipt.operation.as_bytes()[..],
                        &&event.fingerprint.0[..],
                        &&grant.0.as_bytes()[..],
                        &signed(version)?,
                        &signed(audit.0)?,
                    ],
                )
                .await
                .map_err(storage_error)?;
            transaction.commit().await.map_err(commit_error)?;
            Ok(receipt)
        })
        .await
        .map_err(|_| OperatorError::Unknown)?
    }

    async fn current_grant(
        &self,
        id: OperatorGrantId,
        now_ms: u64,
    ) -> Result<Option<OperatorGrant>, OperatorError> {
        tokio::time::timeout(CALL_DEADLINE, async {
            let slot = self.0.checkout().await?;
            let client = &slot.as_ref().ok_or(OperatorError::Unavailable)?.client;
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {GRANT_COLUMNS} FROM accounts.operator_grant g \
                JOIN accounts.account a ON a.account_id = g.account_id \
                JOIN accounts.operator_mfa m ON m.account_id = a.account_id \
                WHERE g.grant_id = $1 AND a.status = 'active' AND a.email_verified_at_ms <= $2 \
                AND m.state = 'active' AND m.confirmed_at_ms <= $2 \
                AND m.credential_generation = a.credential_generation \
                AND m.generation = g.enrolment_generation \
                AND g.issued_at_ms <= $2 AND g.expires_at_ms > $2 AND g.revoked_at_ms IS NULL"
                    ),
                    &[&&id.0.as_bytes()[..], &signed(now_ms)?],
                )
                .await
                .map_err(storage_error)?;
            row.as_ref().map(decode_grant).transpose()
        })
        .await
        .map_err(|_| OperatorError::Unavailable)?
    }

    async fn grants(
        &self,
        account: AccountId,
        after: Option<OperatorGrantId>,
        limit: OperatorPageSize,
    ) -> Result<Vec<OperatorGrant>, OperatorError> {
        tokio::time::timeout(CALL_DEADLINE, async {
            let slot = self.0.checkout().await?;
            let client = &slot.as_ref().ok_or(OperatorError::Unavailable)?.client;
            let after = after.map(|id| id.0.as_bytes().to_vec());
            client
                .query(
                    &format!(
                        "SELECT {GRANT_COLUMNS} FROM accounts.operator_grant g \
                WHERE g.account_id = $1 AND ($2::bytea IS NULL OR g.grant_id > $2) \
                ORDER BY g.grant_id LIMIT $3"
                    ),
                    &[&&account.as_bytes()[..], &after, &i64::from(limit.get())],
                )
                .await
                .map_err(storage_error)?
                .iter()
                .map(decode_grant)
                .collect()
        })
        .await
        .map_err(|_| OperatorError::Unavailable)?
    }

    async fn audit(
        &self,
        account: AccountId,
        after: Option<OperatorAuditId>,
        limit: OperatorPageSize,
    ) -> Result<Vec<OperatorAuditRecord>, OperatorError> {
        tokio::time::timeout(CALL_DEADLINE, async {
            let slot = self.0.checkout().await?;
            let client = &slot.as_ref().ok_or(OperatorError::Unavailable)?.client;
            let after = after.map(|id| signed(id.0)).transpose()?.unwrap_or(0);
            client
                .query(
                    "SELECT audit_id, basis::text, action, target::text, reason, operation_id, \
                fingerprint, outcome, correlation_id, at_ms FROM accounts.operator_audit \
                WHERE account_id = $1 AND audit_id > $2 ORDER BY audit_id LIMIT $3",
                    &[&&account.as_bytes()[..], &after, &i64::from(limit.get())],
                )
                .await
                .map_err(storage_error)?
                .iter()
                .map(decode_audit)
                .collect()
        })
        .await
        .map_err(|_| OperatorError::Unavailable)?
    }
}

async fn read_receipt(
    client: &(impl tokio_postgres::GenericClient + Sync),
    command: &OperatorCommand,
) -> Result<Option<OperatorReceipt>, OperatorError> {
    let row = client
        .query_opt(
            "SELECT fingerprint, grant_id, version, audit_id \
        FROM accounts.operator_receipt WHERE operation_id = $1",
            &[&&command.operation().as_bytes()[..]],
        )
        .await
        .map_err(storage_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let stored: Vec<u8> = row.try_get(0).map_err(storage_error)?;
    if stored.as_slice() != command.fingerprint().0 {
        return Err(OperatorError::PayloadConflict);
    }
    Ok(Some(OperatorReceipt {
        operation: command.operation(),
        grant: OperatorGrantId(operation_id(&row, 1)?),
        version: unsigned(row.try_get(2).map_err(storage_error)?)?,
        audit: OperatorAuditId(unsigned(row.try_get(3).map_err(storage_error)?)?),
    }))
}

async fn insert_grant(
    transaction: &Transaction<'_>,
    value: &GrantOperator,
    now_ms: u64,
) -> Result<OperatorGrantId, OperatorError> {
    if value.expires_at_ms <= now_ms || value.expires_at_ms - now_ms > MAXIMUM_OPERATOR_GRANT_MS {
        return Err(OperatorError::InvalidExpiry);
    }
    let account = transaction
        .query_opt(
            "SELECT credential_generation, \
        status = 'active' AND email_verified_at_ms <= $2 AS eligible \
        FROM accounts.account WHERE account_id = $1 FOR SHARE",
            &[&&value.account.as_bytes()[..], &signed(now_ms)?],
        )
        .await
        .map_err(storage_error)?;
    let Some(account) = account else {
        return Err(OperatorError::AccountNotVerified);
    };
    if account
        .try_get::<_, Option<bool>>(1)
        .map_err(storage_error)?
        != Some(true)
    {
        return Err(OperatorError::AccountNotVerified);
    }
    let generation: i64 = account.try_get(0).map_err(storage_error)?;
    let mfa = transaction
        .query_opt(
            "SELECT generation FROM accounts.operator_mfa \
        WHERE account_id = $1 AND state = 'active' AND confirmed_at_ms <= $2 \
        AND credential_generation = $3 FOR SHARE",
            &[
                &&value.account.as_bytes()[..],
                &signed(now_ms)?,
                &generation,
            ],
        )
        .await
        .map_err(storage_error)?;
    let Some(mfa) = mfa else {
        return Err(OperatorError::ConfirmedMfaRequired);
    };
    let enrolment: i64 = mfa.try_get(0).map_err(storage_error)?;
    let (scope_kind, scope_id) = value.scope.parts();
    let scope_id = scope_id.map(|id| id.to_vec());
    let capabilities: Vec<i16> = value
        .capabilities
        .as_slice()
        .iter()
        .map(|value| *value as i16)
        .collect();
    transaction
        .execute(
            "INSERT INTO accounts.operator_grant (grant_id, account_id, capabilities, \
        scope_kind, scope_id, version, issued_at_ms, expires_at_ms, enrolment_generation, reason) \
        VALUES ($1,$2,$3,$4,$5,1,$6,$7,$8,$9)",
            &[
                &&value.operation.as_bytes()[..],
                &&value.account.as_bytes()[..],
                &capabilities,
                &scope_kind,
                &scope_id,
                &signed(now_ms)?,
                &signed(value.expires_at_ms)?,
                &enrolment,
                &value.reason.as_str(),
            ],
        )
        .await
        .map_err(storage_error)?;
    Ok(OperatorGrantId(value.operation))
}

async fn revoke_grant(
    transaction: &Transaction<'_>,
    value: &RevokeOperator,
    now_ms: u64,
) -> Result<u64, OperatorError> {
    let row = transaction
        .query_opt(
            "SELECT version, revoked_at_ms FROM accounts.operator_grant \
        WHERE grant_id = $1 AND account_id = $2 FOR UPDATE",
            &[
                &&value.grant.0.as_bytes()[..],
                &&value.account.as_bytes()[..],
            ],
        )
        .await
        .map_err(storage_error)?;
    let Some(row) = row else {
        return Err(OperatorError::NotFound);
    };
    let version = unsigned(row.try_get(0).map_err(storage_error)?)?;
    if version != value.expected_version
        || row
            .try_get::<_, Option<i64>>(1)
            .map_err(storage_error)?
            .is_some()
    {
        return Err(OperatorError::StaleVersion);
    }
    let next = version.checked_add(1).ok_or(OperatorError::InvalidRecord)?;
    transaction
        .execute(
            "UPDATE accounts.operator_grant SET version = $2, revoked_at_ms = $3 \
        WHERE grant_id = $1",
            &[
                &&value.grant.0.as_bytes()[..],
                &signed(next)?,
                &signed(now_ms)?,
            ],
        )
        .await
        .map_err(storage_error)?;
    Ok(next)
}

/// Append inside the command owner's transaction, before returning protected data or committing
/// state. This primitive does not admit callers or commit/rollback the owner's transaction.
pub async fn append_operator_audit(
    transaction: &Transaction<'_>,
    account: AccountId,
    event: &NewOperatorAudit,
) -> Result<OperatorAuditId, OperatorError> {
    let basis = encode_basis(event.basis).to_string();
    let target = encode_target(&event.target).to_string();
    let row = transaction.query_one("INSERT INTO accounts.operator_audit \
        (account_id, basis, action, target, reason, operation_id, fingerprint, outcome, correlation_id, at_ms) \
        VALUES ($1,$2::text::jsonb,$3,$4::text::jsonb,$5,$6,$7,$8,$9,$10) RETURNING audit_id",
        &[&&account.as_bytes()[..], &basis, &(event.action as i16), &target, &event.reason.as_str(),
            &&event.operation.as_bytes()[..], &&event.fingerprint.0[..], &(event.outcome as i16),
            &&event.correlation.as_bytes()[..], &signed(event.at_ms)?]).await.map_err(storage_error)?;
    Ok(OperatorAuditId(unsigned(
        row.try_get(0).map_err(storage_error)?,
    )?))
}

fn decode_grant(row: &Row) -> Result<OperatorGrant, OperatorError> {
    let account: Vec<u8> = row.try_get(1).map_err(storage_error)?;
    let capabilities: Vec<i16> = row.try_get(2).map_err(storage_error)?;
    let capabilities = capabilities
        .into_iter()
        .map(|code| OperatorCapability::from_code(code).ok_or(OperatorError::InvalidRecord))
        .collect::<Result<Vec<_>, _>>()?;
    let scope_kind: i16 = row.try_get(3).map_err(storage_error)?;
    let scope_id: Option<Vec<u8>> = row.try_get(4).map_err(storage_error)?;
    let scope = match (scope_kind, scope_id) {
        (0, None) => OperatorScope::Global,
        (1, Some(id)) => OperatorScope::Account(
            AccountId::from_bytes(&id).map_err(|_| OperatorError::InvalidRecord)?,
        ),
        (2, Some(id)) => OperatorScope::Tenant(
            TenantId::from_bytes(&id).map_err(|_| OperatorError::InvalidRecord)?,
        ),
        _ => return Err(OperatorError::InvalidRecord),
    };
    Ok(OperatorGrant {
        id: OperatorGrantId(operation_id(row, 0)?),
        account: AccountId::from_bytes(&account).map_err(|_| OperatorError::InvalidRecord)?,
        capabilities: OperatorCapabilities::new(capabilities)?,
        scope,
        version: unsigned(row.try_get(5).map_err(storage_error)?)?,
        issued_at_ms: unsigned(row.try_get(6).map_err(storage_error)?)?,
        expires_at_ms: unsigned(row.try_get(7).map_err(storage_error)?)?,
        revoked_at_ms: row
            .try_get::<_, Option<i64>>(8)
            .map_err(storage_error)?
            .map(unsigned)
            .transpose()?,
        enrolment_generation: unsigned(row.try_get(9).map_err(storage_error)?)?,
    })
}

fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn encode_basis(basis: OperatorAuditBasis) -> Value {
    match basis {
        OperatorAuditBasis::Host => Value::Null,
        OperatorAuditBasis::Session(value) => json!({
            "account": hex(value.account.as_bytes()), "session": hex(value.session.as_bytes()),
            "credential_generation": value.credential_generation,
            "enrolment_generation": value.enrolment_generation, "proof": hex(value.proof.as_bytes()),
            "verified_at_ms": value.verified_at_ms, "grant": hex(value.grant.0.as_bytes()),
            "grant_version": value.grant_version,
        }),
    }
}

fn decode_basis(value: &Value) -> Result<OperatorAuditBasis, OperatorError> {
    if value.is_null() {
        return Ok(OperatorAuditBasis::Host);
    }
    Ok(OperatorAuditBasis::Session(OperatorSessionBasis {
        account: AccountId::from_hex(text(value, "account")?)
            .map_err(|_| OperatorError::InvalidRecord)?,
        session: WebSessionId::from_hex(text(value, "session")?)
            .map_err(|_| OperatorError::InvalidRecord)?,
        credential_generation: number(value, "credential_generation")?,
        enrolment_generation: number(value, "enrolment_generation")?,
        proof: OperationId::from_hex(text(value, "proof")?)
            .map_err(|_| OperatorError::InvalidRecord)?,
        verified_at_ms: number(value, "verified_at_ms")?,
        grant: OperatorGrantId(
            OperationId::from_hex(text(value, "grant")?)
                .map_err(|_| OperatorError::InvalidRecord)?,
        ),
        grant_version: number(value, "grant_version")?,
    }))
}

fn encode_target(target: &OperatorAuditTarget) -> Value {
    let (kind, id) = match target {
        OperatorAuditTarget::Account(id) => (1, hex(id.as_bytes())),
        OperatorAuditTarget::Grant(id) => (2, hex(id.0.as_bytes())),
        OperatorAuditTarget::WebSession(id) => (3, hex(id.as_bytes())),
        OperatorAuditTarget::RuntimeSession(id) => (4, hex(id.as_bytes())),
        OperatorAuditTarget::Payment(id) => (5, hex(id.as_bytes())),
        OperatorAuditTarget::SupportTicket(id) => (6, id.as_str().to_owned()),
        OperatorAuditTarget::Content(id) => (7, id.as_str().to_owned()),
        OperatorAuditTarget::Audit => (8, String::new()),
    };
    json!({"kind": kind, "id": id})
}

fn decode_target(value: &Value) -> Result<OperatorAuditTarget, OperatorError> {
    let id = text(value, "id")?;
    let invalid = |_| OperatorError::InvalidRecord;
    match number(value, "kind")? {
        1 => Ok(OperatorAuditTarget::Account(
            AccountId::from_hex(id).map_err(invalid)?,
        )),
        2 => Ok(OperatorAuditTarget::Grant(OperatorGrantId(
            OperationId::from_hex(id).map_err(invalid)?,
        ))),
        3 => Ok(OperatorAuditTarget::WebSession(
            WebSessionId::from_hex(id).map_err(invalid)?,
        )),
        4 => Ok(OperatorAuditTarget::RuntimeSession(
            df_types::SessionId::from_hex(id).map_err(invalid)?,
        )),
        5 => Ok(OperatorAuditTarget::Payment(
            OperationId::from_hex(id).map_err(invalid)?,
        )),
        6 => Ok(OperatorAuditTarget::SupportTicket(
            OperatorTargetReference::parse(id)?,
        )),
        7 => Ok(OperatorAuditTarget::Content(
            OperatorTargetReference::parse(id)?,
        )),
        8 if id.is_empty() => Ok(OperatorAuditTarget::Audit),
        _ => Err(OperatorError::InvalidRecord),
    }
}

fn decode_audit(row: &Row) -> Result<OperatorAuditRecord, OperatorError> {
    let basis: String = row.try_get(1).map_err(storage_error)?;
    let target: String = row.try_get(3).map_err(storage_error)?;
    let fingerprint: Vec<u8> = row.try_get(6).map_err(storage_error)?;
    let outcome = match row.try_get::<_, i16>(7).map_err(storage_error)? {
        1 => OperatorAuditOutcome::Applied,
        2 => OperatorAuditOutcome::Refused,
        3 => OperatorAuditOutcome::Unavailable,
        4 => OperatorAuditOutcome::Unknown,
        _ => return Err(OperatorError::InvalidRecord),
    };
    Ok(OperatorAuditRecord {
        id: OperatorAuditId(unsigned(row.try_get(0).map_err(storage_error)?)?),
        event: NewOperatorAudit {
            basis: decode_basis(
                &serde_json::from_str(&basis).map_err(|_| OperatorError::InvalidRecord)?,
            )?,
            action: OperatorAuditAction::from_code(row.try_get(2).map_err(storage_error)?)
                .ok_or(OperatorError::InvalidRecord)?,
            target: decode_target(
                &serde_json::from_str(&target).map_err(|_| OperatorError::InvalidRecord)?,
            )?,
            reason: OperatorReason::parse(&row.try_get::<_, String>(4).map_err(storage_error)?)?,
            operation: operation_id(row, 5)?,
            fingerprint: OperatorFingerprint(
                fingerprint
                    .try_into()
                    .map_err(|_| OperatorError::InvalidRecord)?,
            ),
            outcome,
            correlation: operation_id(row, 8)?,
            at_ms: unsigned(row.try_get(9).map_err(storage_error)?)?,
        },
    })
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, OperatorError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(OperatorError::InvalidRecord)
}
fn number(value: &Value, key: &str) -> Result<u64, OperatorError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or(OperatorError::InvalidRecord)
}
fn operation_id(row: &Row, column: usize) -> Result<OperationId, OperatorError> {
    OperationId::from_bytes(&row.try_get::<_, Vec<u8>>(column).map_err(storage_error)?)
        .map_err(|_| OperatorError::InvalidRecord)
}
fn signed(value: u64) -> Result<i64, OperatorError> {
    i64::try_from(value).map_err(|_| OperatorError::InvalidRecord)
}
fn unsigned(value: i64) -> Result<u64, OperatorError> {
    u64::try_from(value).map_err(|_| OperatorError::InvalidRecord)
}
fn storage_error(error: tokio_postgres::Error) -> OperatorError {
    df_observe::record_audit_line(
        "operator.store_unavailable",
        format!(
            "OPERATOR store_unavailable sqlstate={}",
            error.code().map_or("connection", |code| code.code())
        ),
    );
    OperatorError::Unavailable
}
fn commit_error(error: tokio_postgres::Error) -> OperatorError {
    df_observe::record_audit_line(
        "operator.commit_unknown",
        format!(
            "OPERATOR commit_unknown sqlstate={}",
            error.code().map_or("connection", |code| code.code())
        ),
    );
    OperatorError::Unknown
}

#[cfg(test)]
mod tests;
