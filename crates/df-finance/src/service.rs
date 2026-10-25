//! The finance service: one method per flow, each a single PostgreSQL transaction at READ
//! COMMITTED (planning/finance-ledger-research.md 11.2, 19.1). A flow takes its locks with
//! `finance.lock_flow`, decides in pure code, writes with `finance.commit_event`, and for
//! admission asks the fence and the media catalog before it commits.
//!
//! The connection's role is the caller's choice: production connects as `df_finance`, which holds
//! no table write privilege and carries its lock and statement timeouts as role defaults, so every
//! write is a definer function by construction.

use df_commerce::admission::AdmissionRefusal;
use df_commerce::finance_ids::{
    AllocationId, CampaignKey, EntryId, EventId, LotId, OperationKey, PayerId, PriceVersionId,
    SessionKey, TenantKey,
};
use df_commerce::funding::TrialPolicy;
use df_commerce::funding_flows::{
    AllocateRequest, Consent, FundingRefusal, OpenSession, PurchaseRequest, PurchaseSnapshot,
    SessionPolicy, TrialRequest, WithdrawRequest, WithdrawalRefusal, plan_allocation,
    plan_open_session, plan_publish_price_version, plan_purchase, plan_trial, plan_withdrawal,
};
use df_commerce::ledger::CurrencyTag;
use df_commerce::lifecycle::LifecycleRefusal;
use df_commerce::plan::FinancePlan;
use df_commerce::price_schedule::PriceVersion;
use df_commerce::settlement::SettleRefusal;
use serde_json::{Value, json};
use tokio_postgres::{Client, IsolationLevel, Transaction};

use crate::ports::{Clock, FencePort, FenceVerdict, IdSource, MediaPort, MediaVerdict, MemberPort};
use crate::wire::{
    WireError, event_json, parse_allocate_snapshot, parse_trial_snapshot, parse_withdrawal_snapshot,
};

/// Everything a flow can fail with. A refusal holds nothing and writes nothing.
#[derive(Debug)]
pub enum FinanceError {
    Admission(AdmissionRefusal),
    Funding(FundingRefusal),
    Withdrawal(WithdrawalRefusal),
    Settle(SettleRefusal),
    Lifecycle(LifecycleRefusal),
    Fence(FenceVerdict),
    Media(MediaVerdict),
    /// A port could not answer in time; nothing was written.
    PortUnavailable,
    /// The reservation or attempt was settled or released by another flow first.
    Stale,
    /// The database refused the event: the allowlist, an equation, a constraint or a lock.
    Boundary {
        sqlstate: String,
        detail: String,
    },
    Wire(WireError),
    ReconciliationPeriod,
    ReconciliationSourcePeriod,
    ReconciliationOverflow,
    ReconciliationUnknownAccount(u16),
    ReconciliationUnknownReferenceKind(String),
    ReconciliationPort,
    Database(tokio_postgres::Error),
}

impl From<WireError> for FinanceError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl From<tokio_postgres::Error> for FinanceError {
    fn from(error: tokio_postgres::Error) -> Self {
        match error.as_db_error() {
            Some(db) => Self::Boundary {
                sqlstate: db.code().code().to_owned(),
                detail: db.message().to_owned(),
            },
            None => Self::Database(error),
        }
    }
}

impl FinanceError {
    /// The SQLSTATE of a boundary refusal, if that is what this is.
    pub fn sqlstate(&self) -> Option<&str> {
        match self {
            Self::Boundary { sqlstate, .. } => Some(sqlstate),
            _ => None,
        }
    }

    /// A lock the flow waited for too long (`55P03`) is not a failure of the money: the caller
    /// retries the whole flow, which is idempotent by its operation id.
    pub fn is_lock_timeout(&self) -> bool {
        self.sqlstate() == Some("55P03")
    }
}

/// What a committed event reports for each entry it posted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostedEntry {
    pub entry_id: EntryId,
    pub stream_seq: i64,
    pub entry_hash: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Committed {
    pub event_id: EventId,
    pub entries: Vec<PostedEntry>,
}

/// The tunables of the flows, with the design's defaults (25).
#[derive(Clone, Copy, Debug)]
pub struct FinanceConfig {
    pub trial: TrialPolicy,
    pub reserved_expiry_ms: i64,
    /// How long a cross-owner call (fence, media, membership) may take before the flow gives up and
    /// rolls back (19.1).
    pub port_deadline_ms: u64,
}

impl Default for FinanceConfig {
    fn default() -> Self {
        Self {
            trial: TrialPolicy::default(),
            reserved_expiry_ms: 300_000,
            port_deadline_ms: 1_500,
        }
    }
}

/// A session's funding to open, with the ids it pins.
#[derive(Clone, Copy, Debug)]
pub struct SessionOpening {
    pub session: SessionKey,
    pub tenant: TenantKey,
    pub campaign: CampaignKey,
    pub price_version: PriceVersionId,
    pub currency: CurrencyTag,
    pub policy: SessionPolicy,
}

/// A confirmed payment, as B hands it over after re-reading the processor object.
#[derive(Clone, Copy, Debug)]
pub struct PurchaseCommand<'a> {
    pub payer: PayerId,
    pub processor: &'a str,
    pub payment_ref: &'a str,
    pub currency: CurrencyTag,
    pub amount_micro_dollars: u64,
}

pub struct FinanceService<F, M, S> {
    pub(crate) client: Client,
    pub(crate) fence: F,
    pub(crate) media: M,
    pub(crate) members: S,
    pub(crate) clock: Box<dyn Clock>,
    pub(crate) ids: Box<dyn IdSource>,
    pub(crate) config: FinanceConfig,
}

pub(crate) fn bad_sql(error: &str) -> FinanceError {
    FinanceError::Wire(WireError(error.to_owned()))
}

impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    pub fn new(
        client: Client,
        fence: F,
        media: M,
        members: S,
        clock: Box<dyn Clock>,
        ids: Box<dyn IdSource>,
        config: FinanceConfig,
    ) -> Self {
        Self {
            client,
            fence,
            media,
            members,
            clock,
            ids,
            config,
        }
    }

    /// The underlying connection, for reads by tests and the dev view.
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub(crate) fn id(&self) -> [u8; 16] {
        self.ids.next_id()
    }

    pub(crate) async fn begin(&mut self) -> Result<Transaction<'_>, FinanceError> {
        Self::start(&mut self.client).await
    }

    /// Starts a READ COMMITTED transaction on `client` alone, so the caller can keep using the
    /// service's ports while it is open.
    pub(crate) async fn start(client: &mut Client) -> Result<Transaction<'_>, FinanceError> {
        Ok(client
            .build_transaction()
            .isolation_level(IsolationLevel::ReadCommitted)
            .start()
            .await?)
    }

    pub(crate) async fn lock(
        transaction: &Transaction<'_>,
        flow: &str,
        args: Value,
    ) -> Result<Value, FinanceError> {
        let text = args.to_string();
        let row = transaction
            .query_one(
                "SELECT finance.lock_flow($1, $2::text::jsonb)::text",
                &[&flow, &text],
            )
            .await?;
        let document: String = row.get(0);
        serde_json::from_str(&document).map_err(|error| bad_sql(&error.to_string()))
    }

    pub(crate) async fn commit_plan(
        transaction: &Transaction<'_>,
        plan: &FinancePlan,
    ) -> Result<Committed, FinanceError> {
        let text = event_json(plan).to_string();
        let row = transaction
            .query_one(
                "SELECT finance.commit_event($1::text::jsonb)::text",
                &[&text],
            )
            .await?;
        let document: String = row.get(0);
        let result: Value =
            serde_json::from_str(&document).map_err(|error| bad_sql(&error.to_string()))?;
        let entries = result
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| bad_sql("no entries in the result"))?
            .iter()
            .map(|entry| {
                let id = entry
                    .get("entry_id")
                    .and_then(Value::as_str)
                    .and_then(EntryId::from_hex)
                    .ok_or_else(|| bad_sql("entry id"))?;
                let hash = entry
                    .get("entry_hash")
                    .and_then(Value::as_str)
                    .and_then(df_commerce::finance_ids::unhex)
                    .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                    .ok_or_else(|| bad_sql("entry hash"))?;
                Ok(PostedEntry {
                    entry_id: id,
                    stream_seq: entry.get("stream_seq").and_then(Value::as_i64).unwrap_or(0),
                    entry_hash: hash,
                })
            })
            .collect::<Result<Vec<_>, FinanceError>>()?;
        Ok(Committed {
            event_id: plan.event_id,
            entries,
        })
    }

    /// Runs one locked flow: takes `flow`'s locks, lets `decide` turn the snapshot into a plan (or
    /// into nothing, which rolls back), commits the plan.
    pub(crate) async fn run_flow(
        &mut self,
        flow: &str,
        args: Value,
        decide: impl FnOnce(&Value) -> Result<Option<FinancePlan>, FinanceError>,
    ) -> Result<Option<Committed>, FinanceError> {
        let transaction = self.begin().await?;
        let snapshot = Self::lock(&transaction, flow, args).await?;
        let Some(plan) = decide(&snapshot)? else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let committed = Self::commit_plan(&transaction, &plan).await?;
        transaction.commit().await?;
        Ok(Some(committed))
    }

    /// Commits a caller-built plan as one event (event-only flows and the tests' raw plans).
    pub async fn record_event(&mut self, plan: &FinancePlan) -> Result<Committed, FinanceError> {
        let transaction = self.begin().await?;
        let committed = Self::commit_plan(&transaction, plan).await?;
        transaction.commit().await?;
        Ok(committed)
    }

    /// Publishes an immutable price version (9.1).
    pub async fn publish_price_version(
        &mut self,
        version: &PriceVersion,
    ) -> Result<Committed, FinanceError> {
        let plan = plan_publish_price_version(
            EventId::from_array(self.id()),
            version,
            self.clock.now_ms(),
        );
        self.record_event(&plan).await
    }

    /// Opens a session's funding and pins its price version (8.1).
    pub async fn open_session(
        &mut self,
        opening: SessionOpening,
    ) -> Result<Committed, FinanceError> {
        let plan = plan_open_session(OpenSession {
            event_id: EventId::from_array(self.id()),
            session: opening.session,
            tenant: opening.tenant,
            campaign: opening.campaign,
            price_version: opening.price_version,
            currency: opening.currency,
            policy: opening.policy,
            now_ms: self.clock.now_ms(),
        });
        self.record_event(&plan).await
    }

    /// 7.4: the trial grant. `Ok(None)` means it was refused (already granted, or off).
    pub async fn grant_trial(
        &mut self,
        payer: PayerId,
        email_grant_key: [u8; 32],
        currency: CurrencyTag,
    ) -> Result<Option<Committed>, FinanceError> {
        let request = TrialRequest {
            event_id: EventId::from_array(self.id()),
            lot: LotId::from_array(self.id()),
            entry_id: EntryId::from_array(self.id()),
            payer,
            email_grant_key,
            currency,
            policy: self.config.trial,
            now_ms: self.clock.now_ms(),
        };
        let args = json!({
            "payer": payer.hex(),
            "email_grant_key": df_commerce::finance_ids::hex(&email_grant_key),
        });
        self.run_flow("trial", args, |snapshot| {
            match plan_trial(parse_trial_snapshot(snapshot), request) {
                Ok(plan) => Ok(Some(plan)),
                Err(FundingRefusal::Trial(_)) => Ok(None),
                Err(refusal) => Err(FinanceError::Funding(refusal)),
            }
        })
        .await
    }

    /// 7.3: a purchase confirmed by an authoritative processor state. `Ok(None)` for a payment
    /// already applied (PAY-02).
    pub async fn apply_purchase(
        &mut self,
        command: PurchaseCommand<'_>,
    ) -> Result<Option<Committed>, FinanceError> {
        let request = PurchaseRequest {
            event_id: EventId::from_array(self.id()),
            lot: LotId::from_array(self.id()),
            entry_id: EntryId::from_array(self.id()),
            payer: command.payer,
            processor: command.processor.to_owned(),
            payment_ref: command.payment_ref.to_owned(),
            charge_ref: None,
            currency: command.currency,
            amount_micro_dollars: command.amount_micro_dollars,
            now_ms: self.clock.now_ms(),
        };
        let args = json!({
            "payer": command.payer.hex(),
            "processor": command.processor,
            "payment_ref": command.payment_ref,
        });
        self.run_flow("purchase", args, |snapshot| {
            let present = |key: &str| snapshot.get(key).is_some_and(|value| !value.is_null());
            let state = PurchaseSnapshot {
                payer_exists: present("payer"),
                payment_state_exists: present("payment_state"),
            };
            match plan_purchase(state, &request) {
                Ok(plan) => Ok(Some(plan)),
                Err(FundingRefusal::AlreadyApplied) => Ok(None),
                Err(refusal) => Err(FinanceError::Funding(refusal)),
            }
        })
        .await
    }

    /// 8.2: moves credit into the session allocation.
    pub async fn allocate(
        &mut self,
        payer: PayerId,
        session: SessionKey,
        operation: OperationKey,
        amount_micro_dollars: u64,
        consent: Consent,
    ) -> Result<Committed, FinanceError> {
        let request = AllocateRequest {
            event_id: EventId::from_array(self.id()),
            entry_id: EntryId::from_array(self.id()),
            new_allocation: AllocationId::from_array(self.id()),
            operation,
            payer,
            amount_micro_dollars,
            consent,
            now_ms: self.clock.now_ms(),
        };
        let args = json!({"payer": payer.hex(), "session": session.hex()});
        let committed = self
            .run_flow("allocate", args, |snapshot| {
                plan_allocation(&parse_allocate_snapshot(snapshot)?, &request)
                    .map(Some)
                    .map_err(FinanceError::Funding)
            })
            .await?;
        committed.ok_or(FinanceError::Stale)
    }

    /// 5.5: release the payer's unreserved allocation; admitted holds remain in their reservation.
    pub async fn withdraw_allocation(
        &mut self,
        payer: PayerId,
        session: SessionKey,
        operation: OperationKey,
    ) -> Result<Option<Committed>, FinanceError> {
        let request = WithdrawRequest {
            event_id: EventId::from_array(self.id()),
            entry_id: EntryId::from_array(self.id()),
            operation,
            now_ms: self.clock.now_ms(),
        };
        let args = json!({
            "payer": payer.hex(),
            "session": session.hex(),
            "operation": operation.hex(),
        });
        self.run_flow("withdrawal", args, |snapshot| {
            plan_withdrawal(&parse_withdrawal_snapshot(snapshot)?, &request)
                .map_err(FinanceError::Withdrawal)
        })
        .await
    }
}
