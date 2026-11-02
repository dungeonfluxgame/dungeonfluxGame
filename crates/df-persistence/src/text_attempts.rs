//! The PostgreSQL side of durable text attempts (`df_ai::text_attempts`). One row per scoped
//! operation key in the `df_text` schema. Each transition is a single statement guarded by the
//! phase it expects, so concurrent callers and restarts see one ordered history per operation.
//!
//! `connect` is loopback TCP only; `connect_tls` reaches any host over verified TLS. Neither carries
//! a credential in this module; the composition root supplies the `Config`. One connection is used at a time. A connection failure drops it so
//! the next operation reconnects instead of trusting a connection that may have lost an answer.

use std::{net::IpAddr, time::Duration};

use df_ai::{
    text::{TextOutcome, TextPurpose, TextUsage, UnavailableReason},
    text_attempts::{
        Admission, AttemptMeta, AttemptOwner, AttemptPhase, AttemptScope, AttemptState,
        AttemptStoreError, Basis, Fingerprint, OperationKey, OutcomeKind, QualifyResult,
        StoredAttempt, TextAttemptStore, Verdict,
    },
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::Mutex,
    task::JoinHandle,
    time::{error::Elapsed, timeout},
};
use tokio_postgres::{
    Client, Config, NoTls, Row,
    config::{Host, SslMode},
    tls::TlsStream,
    types::ToSql,
};

use crate::postgres_tls::VerifiedTlsConnector;

const SCHEMA: &str = include_str!("text_attempts/schema.sql");
const OPERATION_DEADLINE: Duration = Duration::from_secs(8);
const CONNECT_DEADLINE: Duration = Duration::from_secs(5);
/// The most rows one inspection read returns.
pub const MAXIMUM_RECENT: u32 = 100;

const INSERT_ACCEPTED: &str = "INSERT INTO df_text.text_attempt \
    (scope, operation_key, fingerprint, purpose, profile, output_name, phase) \
    VALUES ($1, $2, $3, $4, $5, $6, 'accepted') ON CONFLICT (scope, operation_key) DO NOTHING";
const LOOKUP: &str = "SELECT fingerprint, phase, owner, outcome_kind, outcome_reason, answer, \
    usage FROM df_text.text_attempt WHERE scope = $1 AND operation_key = $2";
const BEGIN_DISPATCH: &str = "UPDATE df_text.text_attempt SET phase = 'dispatched', owner = $3, \
    basis = $4, updated_at = now() WHERE scope = $1 AND operation_key = $2 AND phase = 'accepted'";
const RECORD_CANDIDATE: &str = "UPDATE df_text.text_attempt SET phase = 'candidate', answer = $3, \
    usage = $4, updated_at = now() \
    WHERE scope = $1 AND operation_key = $2 AND phase = 'dispatched'";
const SETTLE: &str = "UPDATE df_text.text_attempt SET phase = 'settled', outcome_kind = $3, \
    outcome_reason = $4, usage = $5, updated_at = now() \
    WHERE scope = $1 AND operation_key = $2 AND phase = 'dispatched'";
/// One statement decides the verdict against the recorded basis, so the check and the transition
/// cannot be separated by another writer. A moved basis settles the candidate as stale.
const QUALIFY: &str = "UPDATE df_text.text_attempt SET \
    phase = CASE WHEN basis IS DISTINCT FROM $4::bytea THEN 'settled' \
                 WHEN $3::text = 'accepted' THEN 'settled' ELSE 'rejected' END, \
    outcome_kind = CASE WHEN basis IS DISTINCT FROM $4::bytea THEN 'stale' \
                        WHEN $3::text = 'accepted' THEN 'answered' ELSE NULL END, \
    answer = CASE WHEN basis IS DISTINCT FROM $4::bytea THEN NULL \
                  WHEN $3::text = 'accepted' THEN answer ELSE NULL END, \
    updated_at = now() \
    WHERE scope = $1 AND operation_key = $2 AND phase = 'candidate' \
    RETURNING (basis IS DISTINCT FROM $4::bytea) AS moved";
const INVALIDATE: &str = "UPDATE df_text.text_attempt SET phase = 'rejected', \
    outcome_kind = NULL, outcome_reason = NULL, answer = NULL, updated_at = now() \
    WHERE scope = $1 AND operation_key = $2 AND phase = 'settled' AND outcome_kind = 'answered'";
const MARK_UNKNOWN: &str = "UPDATE df_text.text_attempt SET phase = 'unknown', updated_at = now() \
    WHERE scope = $1 AND operation_key = $2 AND phase = 'dispatched'";
const CANCEL: &str = "UPDATE df_text.text_attempt SET phase = 'cancelled', updated_at = now() \
    WHERE scope = $1 AND operation_key = $2 AND phase = 'accepted'";
const SELECT_SUMMARY: &str = "SELECT scope, operation_key, purpose, profile, output_name, phase, \
    outcome_kind, outcome_reason, octet_length(answer) AS answer_bytes, usage, \
    updated_at::text AS updated_at FROM df_text.text_attempt";
const PRUNE: &str = "DELETE FROM df_text.text_attempt WHERE phase IN ('settled', 'rejected', \
    'cancelled') AND updated_at < now() - make_interval(days => $1)";

struct Slot {
    client: Option<Client>,
    driver: Option<JoinHandle<()>>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(driver) = self.driver.take() {
            driver.abort();
        }
    }
}

/// A read-only view of one attempt. It carries the size of a stored answer, never its text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptSummary {
    pub scope: String,
    pub operation_key: String,
    pub purpose: TextPurpose,
    pub profile: String,
    pub output_name: String,
    pub phase: AttemptPhase,
    pub outcome: Option<OutcomeKind>,
    pub reason: Option<UnavailableReason>,
    pub answer_bytes: Option<usize>,
    pub usage_total: Option<u64>,
    pub updated_at: String,
}

/// The durable attempt store on one PostgreSQL database.
pub struct PostgresTextAttempts {
    config: Config,
    /// Present for a store that reaches a non-loopback host; every reconnect uses it too.
    tls: Option<VerifiedTlsConnector>,
    slot: Mutex<Slot>,
}

impl PostgresTextAttempts {
    /// Connects over loopback TCP and creates the `df_text` schema when it is absent.
    pub async fn connect(config: &Config) -> Result<Self, AttemptStoreError> {
        require_loopback(config)?;
        let slot = open(config, None).await?;
        Ok(Self {
            config: config.clone(),
            tls: None,
            slot: Mutex::new(slot),
        })
    }

    /// Connects to any host over TLS that verifies the certificate chain and host name, through the
    /// same connector the other stores use, and creates the `df_text` schema when it is absent.
    pub async fn connect_tls(
        config: &Config,
        connector: VerifiedTlsConnector,
    ) -> Result<Self, AttemptStoreError> {
        if config.get_hosts().is_empty() {
            return Err(AttemptStoreError::Invalid);
        }
        let slot = open(config, Some(&connector)).await?;
        Ok(Self {
            config: config.clone(),
            tls: Some(connector),
            slot: Mutex::new(slot),
        })
    }

    /// The newest attempts, optionally of one scope or phase. `limit` is 1 to 100.
    pub async fn recent(
        &self,
        scope: Option<&AttemptScope>,
        phase: Option<AttemptPhase>,
        limit: u32,
    ) -> Result<Vec<AttemptSummary>, AttemptStoreError> {
        if !(1..=MAXIMUM_RECENT).contains(&limit) {
            return Err(AttemptStoreError::Invalid);
        }
        let sql = format!(
            "{SELECT_SUMMARY} WHERE ($1::text IS NULL OR scope = $1) \
             AND ($2::text IS NULL OR phase = $2) \
             ORDER BY updated_at DESC, scope, operation_key LIMIT $3"
        );
        let scope = scope.map(AttemptScope::as_str);
        let phase = phase.map(AttemptPhase::label);
        let limit = i64::from(limit);
        self.many(&sql, &[&scope, &phase, &limit])
            .await?
            .iter()
            .map(summary_from_row)
            .collect()
    }

    /// One attempt by key, or `None`.
    pub async fn attempt(
        &self,
        key: &OperationKey,
    ) -> Result<Option<AttemptSummary>, AttemptStoreError> {
        let sql = format!("{SELECT_SUMMARY} WHERE scope = $1 AND operation_key = $2");
        self.one(&sql, &[&key.scope().as_str(), &key.operation()])
            .await?
            .as_ref()
            .map(summary_from_row)
            .transpose()
    }

    /// Deletes settled, rejected and cancelled attempts untouched for `days`, and returns how many.
    /// Attempts that may still matter (accepted, dispatched, candidate, unknown) are never deleted:
    /// an unknown send is liability history, not cache.
    pub async fn prune(&self, days: u32) -> Result<u64, AttemptStoreError> {
        let days = i32::try_from(days).map_err(|_| AttemptStoreError::Invalid)?;
        self.execute(PRUNE, &[&days]).await
    }

    async fn slot(&self) -> Result<tokio::sync::MutexGuard<'_, Slot>, AttemptStoreError> {
        let mut slot = self.slot.lock().await;
        if slot.client.as_ref().is_none_or(Client::is_closed) {
            *slot = open(&self.config, self.tls.as_ref()).await?;
        }
        Ok(slot)
    }

    async fn execute(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, AttemptStoreError> {
        let mut slot = self.slot().await?;
        let client = slot.client.as_ref().ok_or(AttemptStoreError::Unreachable)?;
        let result = timeout(OPERATION_DEADLINE, client.execute(sql, params)).await;
        finish(&mut slot, result)
    }

    /// Runs one statement and reports whether it changed exactly one row.
    async fn changed(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<bool, AttemptStoreError> {
        Ok(self.execute(sql, params).await? == 1)
    }

    async fn one(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, AttemptStoreError> {
        let mut slot = self.slot().await?;
        let client = slot.client.as_ref().ok_or(AttemptStoreError::Unreachable)?;
        let result = timeout(OPERATION_DEADLINE, client.query_opt(sql, params)).await;
        finish(&mut slot, result)
    }

    async fn many(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, AttemptStoreError> {
        let mut slot = self.slot().await?;
        let client = slot.client.as_ref().ok_or(AttemptStoreError::Unreachable)?;
        let result = timeout(OPERATION_DEADLINE, client.query(sql, params)).await;
        finish(&mut slot, result)
    }
}

impl TextAttemptStore for PostgresTextAttempts {
    async fn lookup(&self, key: &OperationKey) -> Result<Option<StoredAttempt>, AttemptStoreError> {
        self.one(LOOKUP, &[&key.scope().as_str(), &key.operation()])
            .await?
            .as_ref()
            .map(stored_from_row)
            .transpose()
    }

    async fn admit(
        &self,
        key: &OperationKey,
        meta: &AttemptMeta<'_>,
    ) -> Result<Admission, AttemptStoreError> {
        let params: &[&(dyn ToSql + Sync)] = &[
            &key.scope().as_str(),
            &key.operation(),
            &meta.fingerprint.as_slice(),
            &meta.purpose.label(),
            &meta.profile.as_str(),
            &meta.output_name,
        ];
        if self.changed(INSERT_ACCEPTED, params).await? {
            return Ok(Admission::Created);
        }
        match self.lookup(key).await? {
            Some(stored) => Ok(Admission::Existing(stored)),
            // The row was removed between the insert and the read: retry from the caller.
            None => Err(AttemptStoreError::Rejected),
        }
    }

    async fn begin_dispatch(
        &self,
        key: &OperationKey,
        owner: AttemptOwner,
        basis: Basis,
    ) -> Result<bool, AttemptStoreError> {
        let owner = owner.as_bytes().as_slice();
        let basis = basis.as_slice();
        self.changed(
            BEGIN_DISPATCH,
            &[&key.scope().as_str(), &key.operation(), &owner, &basis],
        )
        .await
    }

    async fn record_candidate(
        &self,
        key: &OperationKey,
        answer: &Value,
        usage: Option<TextUsage>,
    ) -> Result<bool, AttemptStoreError> {
        if !answer.is_object() {
            return Err(AttemptStoreError::Invalid);
        }
        let answer = answer.to_string();
        let usage = encode_usage(usage);
        self.changed(
            RECORD_CANDIDATE,
            &[&key.scope().as_str(), &key.operation(), &answer, &usage],
        )
        .await
    }

    async fn settle(
        &self,
        key: &OperationKey,
        outcome: &TextOutcome,
        usage: Option<TextUsage>,
    ) -> Result<bool, AttemptStoreError> {
        let (kind, reason) = encode_outcome(outcome)?;
        let usage = encode_usage(usage);
        self.changed(
            SETTLE,
            &[
                &key.scope().as_str(),
                &key.operation(),
                &kind.label(),
                &reason,
                &usage,
            ],
        )
        .await
    }

    async fn qualify(
        &self,
        key: &OperationKey,
        verdict: Verdict,
        basis_now: Option<Basis>,
    ) -> Result<QualifyResult, AttemptStoreError> {
        let verdict = match verdict {
            Verdict::Accepted => "accepted",
            Verdict::Rejected => "rejected",
        };
        let basis_now: Option<&[u8]> = basis_now.as_ref().map(<[u8; 32]>::as_slice);
        let row = self
            .one(
                QUALIFY,
                &[
                    &key.scope().as_str(),
                    &key.operation(),
                    &verdict,
                    &basis_now,
                ],
            )
            .await?;
        let Some(row) = row else {
            return Ok(QualifyResult::NotCandidate);
        };
        let moved: bool = row.try_get("moved").map_err(|_| invalid())?;
        Ok(if moved {
            QualifyResult::BasisMoved
        } else {
            QualifyResult::Applied
        })
    }

    async fn invalidate(&self, key: &OperationKey) -> Result<bool, AttemptStoreError> {
        self.changed(INVALIDATE, &[&key.scope().as_str(), &key.operation()])
            .await
    }

    async fn mark_unknown(&self, key: &OperationKey) -> Result<bool, AttemptStoreError> {
        self.changed(MARK_UNKNOWN, &[&key.scope().as_str(), &key.operation()])
            .await
    }

    async fn cancel(&self, key: &OperationKey) -> Result<bool, AttemptStoreError> {
        self.changed(CANCEL, &[&key.scope().as_str(), &key.operation()])
            .await
    }
}

/// Maps one statement's result to the typed error. A failure that says nothing about the server
/// (a lost or timed-out connection) drops the connection; a statement the server refused does not.
fn finish<T>(
    slot: &mut Slot,
    result: Result<Result<T, tokio_postgres::Error>, Elapsed>,
) -> Result<T, AttemptStoreError> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) if error.as_db_error().is_some() => Err(AttemptStoreError::Rejected),
        Ok(Err(_)) => {
            slot.client = None;
            Err(AttemptStoreError::Unreachable)
        }
        Err(_) => {
            slot.client = None;
            Err(AttemptStoreError::TimedOut)
        }
    }
}

/// The driver ends when the connection closes; the next statement sees the closed client and
/// reconnects, so its error carries nothing the caller can act on.
fn drive<S, T>(connection: tokio_postgres::Connection<S, T>) -> JoinHandle<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    T: TlsStream + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let _ended = connection.await;
    })
}

async fn open(
    config: &Config,
    tls: Option<&VerifiedTlsConnector>,
) -> Result<Slot, AttemptStoreError> {
    let (client, driver) = match tls {
        None => {
            let (client, connection) = timeout(CONNECT_DEADLINE, config.connect(NoTls))
                .await
                .map_err(|_| AttemptStoreError::TimedOut)?
                .map_err(|_| AttemptStoreError::Unreachable)?;
            (client, drive(connection))
        }
        Some(connector) => {
            let mut config = config.clone();
            config.ssl_mode(SslMode::Require);
            let (client, connection) = timeout(CONNECT_DEADLINE, config.connect(connector.clone()))
                .await
                .map_err(|_| AttemptStoreError::TimedOut)?
                .map_err(|_| AttemptStoreError::Unreachable)?;
            (client, drive(connection))
        }
    };
    let mut slot = Slot {
        client: Some(client),
        driver: Some(driver),
    };
    let client = slot.client.as_ref().ok_or(AttemptStoreError::Unreachable)?;
    let applied = timeout(OPERATION_DEADLINE, client.batch_execute(SCHEMA)).await;
    finish(&mut slot, applied)?;
    Ok(slot)
}

/// Plain TCP is accepted only to a literal loopback address, so the store never sends text to a
/// remote host without TLS. A `hostaddr` overrides where the connection goes whatever the host
/// name says, and a name such as `localhost` is resolved by the system, so neither is trusted:
/// every host and every hostaddr must be a literal loopback IP. Anything else is refused before a
/// connection is attempted.
fn require_loopback(config: &Config) -> Result<(), AttemptStoreError> {
    let hosts = config.get_hosts();
    if hosts.is_empty() {
        return Err(AttemptStoreError::Invalid);
    }
    if config
        .get_hostaddrs()
        .iter()
        .any(|address| !address.is_loopback())
    {
        return Err(AttemptStoreError::Invalid);
    }
    for host in hosts {
        match host {
            Host::Tcp(name) if name.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()) => {}
            _ => return Err(AttemptStoreError::Invalid),
        }
    }
    Ok(())
}

fn stored_from_row(row: &Row) -> Result<StoredAttempt, AttemptStoreError> {
    let fingerprint: Vec<u8> = row.try_get("fingerprint").map_err(|_| invalid())?;
    let fingerprint: Fingerprint = fingerprint.try_into().map_err(|_| invalid())?;
    let phase: String = row.try_get("phase").map_err(|_| invalid())?;
    let owner: Option<Vec<u8>> = row.try_get("owner").map_err(|_| invalid())?;
    let state = match AttemptPhase::parse(&phase).ok_or_else(invalid)? {
        AttemptPhase::Accepted => AttemptState::Accepted,
        AttemptPhase::Dispatched => AttemptState::Dispatched {
            owner: owner_of(owner)?,
        },
        AttemptPhase::Candidate => AttemptState::Candidate {
            answer: answer_of(row)?.ok_or_else(invalid)?,
            usage: usage_of(row)?,
        },
        AttemptPhase::Settled => AttemptState::Settled {
            outcome: outcome_of(row)?,
            usage: usage_of(row)?,
        },
        AttemptPhase::Rejected => AttemptState::Rejected {
            usage: usage_of(row)?,
        },
        AttemptPhase::Unknown => AttemptState::Unknown,
        AttemptPhase::Cancelled => AttemptState::Cancelled,
    };
    Ok(StoredAttempt { fingerprint, state })
}

fn owner_of(owner: Option<Vec<u8>>) -> Result<AttemptOwner, AttemptStoreError> {
    let bytes: [u8; 16] = owner
        .ok_or_else(invalid)?
        .try_into()
        .map_err(|_| invalid())?;
    AttemptOwner::new(bytes).ok_or_else(invalid)
}

fn answer_of(row: &Row) -> Result<Option<Value>, AttemptStoreError> {
    let text: Option<String> = row.try_get("answer").map_err(|_| invalid())?;
    let Some(text) = text else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&text).map_err(|_| invalid())?;
    if value.is_object() {
        Ok(Some(value))
    } else {
        Err(invalid())
    }
}

fn outcome_of(row: &Row) -> Result<TextOutcome, AttemptStoreError> {
    let kind: Option<String> = row.try_get("outcome_kind").map_err(|_| invalid())?;
    let reason: Option<String> = row.try_get("outcome_reason").map_err(|_| invalid())?;
    let kind = kind
        .as_deref()
        .and_then(OutcomeKind::parse)
        .ok_or_else(invalid)?;
    match kind {
        OutcomeKind::Answered => Ok(TextOutcome::Answered(answer_of(row)?.ok_or_else(invalid)?)),
        OutcomeKind::Unavailable => reason
            .as_deref()
            .and_then(UnavailableReason::parse)
            .map(TextOutcome::Unavailable)
            .ok_or_else(invalid),
        OutcomeKind::Deadline => Ok(TextOutcome::Deadline),
        OutcomeKind::Invalid => Ok(TextOutcome::Invalid),
        OutcomeKind::Stale => Ok(TextOutcome::Stale),
    }
}

/// The stored columns for an outcome that is not an answer. An answer settles only through a
/// candidate and a verdict, and an unknown send is recorded by the unknown phase.
fn encode_outcome(
    outcome: &TextOutcome,
) -> Result<(OutcomeKind, Option<&'static str>), AttemptStoreError> {
    match outcome {
        TextOutcome::Unavailable(reason) => Ok((OutcomeKind::Unavailable, Some(reason.label()))),
        TextOutcome::Deadline => Ok((OutcomeKind::Deadline, None)),
        TextOutcome::Invalid => Ok((OutcomeKind::Invalid, None)),
        TextOutcome::Stale => Ok((OutcomeKind::Stale, None)),
        TextOutcome::Answered(_) | TextOutcome::Unknown => Err(invalid()),
    }
}

/// The stored usage object, or null when the supplier reported none. Unreported categories stay
/// null: the codec never writes zero for an unknown count.
fn encode_usage(usage: Option<TextUsage>) -> Option<String> {
    usage.map(|usage| {
        json!({
            "total_tokens": usage.total_tokens,
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "cached_input_tokens": usage.cached_input_tokens,
            "cache_write_tokens": usage.cache_write_tokens,
            "reasoning_output_tokens": usage.reasoning_output_tokens,
        })
        .to_string()
    })
}

fn usage_of(row: &Row) -> Result<Option<TextUsage>, AttemptStoreError> {
    let text: Option<String> = row.try_get("usage").map_err(|_| invalid())?;
    let Some(text) = text else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&text).map_err(|_| invalid())?;
    let category = |name: &str| value.get(name).and_then(Value::as_u64);
    Ok(Some(TextUsage {
        total_tokens: category("total_tokens").ok_or_else(invalid)?,
        input_tokens: category("input_tokens"),
        output_tokens: category("output_tokens"),
        cached_input_tokens: category("cached_input_tokens"),
        cache_write_tokens: category("cache_write_tokens"),
        reasoning_output_tokens: category("reasoning_output_tokens"),
    }))
}

fn summary_from_row(row: &Row) -> Result<AttemptSummary, AttemptStoreError> {
    let text = |column: &str| -> Result<String, AttemptStoreError> {
        row.try_get(column).map_err(|_| invalid())
    };
    let purpose = TextPurpose::parse(&text("purpose")?).ok_or_else(invalid)?;
    let phase = AttemptPhase::parse(&text("phase")?).ok_or_else(invalid)?;
    let outcome: Option<String> = row.try_get("outcome_kind").map_err(|_| invalid())?;
    let reason: Option<String> = row.try_get("outcome_reason").map_err(|_| invalid())?;
    let answer_bytes: Option<i32> = row.try_get("answer_bytes").map_err(|_| invalid())?;
    Ok(AttemptSummary {
        scope: text("scope")?,
        operation_key: text("operation_key")?,
        purpose,
        profile: text("profile")?,
        output_name: text("output_name")?,
        phase,
        outcome: outcome
            .map(|label| OutcomeKind::parse(&label).ok_or_else(invalid))
            .transpose()?,
        reason: reason
            .map(|label| UnavailableReason::parse(&label).ok_or_else(invalid))
            .transpose()?,
        answer_bytes: answer_bytes
            .map(|bytes| usize::try_from(bytes).map_err(|_| invalid()))
            .transpose()?,
        usage_total: usage_of(row)?.map(|usage| usage.total_tokens),
        updated_at: text("updated_at")?,
    })
}

fn invalid() -> AttemptStoreError {
    AttemptStoreError::Invalid
}

#[cfg(test)]
mod tests {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};

    use super::*;

    fn connector() -> VerifiedTlsConnector {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let certificate = params.self_signed(&key).unwrap();
        let path = std::env::temp_dir().join(format!(
            "df-text-attempts-ca-{}-{}.pem",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, certificate.pem()).unwrap();
        let connector = VerifiedTlsConnector::from_ca_file(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        connector
    }

    #[tokio::test]
    async fn a_tls_connection_without_a_host_is_refused_before_connecting() {
        let result = PostgresTextAttempts::connect_tls(&Config::new(), connector()).await;
        assert_eq!(result.err(), Some(AttemptStoreError::Invalid));
    }

    #[tokio::test]
    async fn a_tls_connection_to_a_closed_port_is_unreachable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut config = Config::new();
        config.host("127.0.0.1").port(port).user("postgres");
        let result = PostgresTextAttempts::connect_tls(&config, connector()).await;
        assert_eq!(result.err(), Some(AttemptStoreError::Unreachable));
    }
}
