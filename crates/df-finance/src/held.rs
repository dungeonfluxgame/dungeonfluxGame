//! The flows on an admitted attempt (planning/finance-ledger-research.md 11.3, 12.2, 12.5, 16.5,
//! 16.6, 9.3): prepare and send, settle, release, credit, and the quote around admission.

use df_commerce::admission::{Route, WorkClass};
use df_commerce::finance_ids::{
    AttemptId, EntryId, EventId, OperationKey, QuoteId, SessionKey, UsageEventId,
};
use df_commerce::lifecycle::{
    Deadlines, QuoteRequest, plan_mark_sent, plan_prepare_dispatch, plan_quote, plan_quote_decision,
};
use df_commerce::price_schedule::PriceVersion;
use df_commerce::settlement::{
    CreditReason, CreditRequest, ReleaseReason, ReleaseRequest, SettleRequest, UnusableOutcome,
    UnusableRequest, UsageSource, plan_credit, plan_release, plan_settlement,
    plan_unusable_settlement,
};
use serde_json::{Value, json};

use crate::ports::{FencePort, MediaPort, MemberPort};
use crate::service::{Committed, FinanceError, FinanceService};
use crate::wire::{parse_attempt_row, parse_credit_snapshot, parse_held_snapshot, parse_quote_row};

/// An operator credit of a charge.
#[derive(Clone, Debug)]
pub struct CreditCommand {
    pub usage_event: UsageEventId,
    pub operation: OperationKey,
    pub amount_micro_dollars: u64,
    pub reason: CreditReason,
    pub actor_id: [u8; 16],
    pub case_ref: String,
    pub approver_id: Option<[u8; 16]>,
}

/// A quote to store for a session.
#[derive(Clone, Debug)]
pub struct QuoteCommand {
    pub session: SessionKey,
    pub route: Route,
    pub work_class: WorkClass,
    pub maximum_units: Vec<(String, u64)>,
    pub estimate_units: Vec<(String, u64)>,
    pub request_digest: [u8; 32],
    pub lifetime_ms: i64,
}

impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    /// 12.2: settles a usable, billed result against its held shares. `Ok(None)` when the
    /// reservation is no longer held (JOB-02: a second settlement changes nothing).
    pub async fn settle(
        &mut self,
        attempt: AttemptId,
        actual_units: Vec<(String, u64)>,
        usage_source: UsageSource,
    ) -> Result<Option<Committed>, FinanceError> {
        let request = SettleRequest {
            event_id: EventId::from_array(self.id()),
            entry_id: EntryId::from_array(self.id()),
            usage_event: UsageEventId::from_array(self.id()),
            actual_units,
            usage_source,
            now_ms: self.clock.now_ms(),
        };
        self.run_flow(
            "settlement",
            json!({"attempt": attempt.hex()}),
            |document| {
                let Some(snapshot) = parse_held_snapshot(document)? else {
                    return Ok(None);
                };
                plan_settlement(&snapshot, &request)
                    .map(Some)
                    .map_err(FinanceError::Settle)
            },
        )
        .await
    }

    /// 12.2: settles a billed result the metering outcome marked unusable. The customer
    /// is never charged and the supplier cost is a company loss; this is not a credit
    /// of a settled charge. `Ok(None)` when the reservation is no longer held.
    pub async fn settle_unusable(
        &mut self,
        attempt: AttemptId,
        actual_units: Vec<(String, u64)>,
        usage_source: UsageSource,
        outcome: UnusableOutcome,
    ) -> Result<Option<Committed>, FinanceError> {
        let request = UnusableRequest {
            event_id: EventId::from_array(self.id()),
            entry_id: EntryId::from_array(self.id()),
            actual_units,
            usage_source,
            outcome,
            now_ms: self.clock.now_ms(),
        };
        self.run_flow(
            "settlement",
            json!({"attempt": attempt.hex()}),
            |document| {
                let Some(snapshot) = parse_held_snapshot(document)? else {
                    return Ok(None);
                };
                plan_unusable_settlement(&snapshot, &request)
                    .map(Some)
                    .map_err(FinanceError::Settle)
            },
        )
        .await
    }

    /// 16.6: releases a hold (a cancel before dispatch, or the customer-hold deadline).
    pub async fn release(
        &mut self,
        attempt: AttemptId,
        reason: ReleaseReason,
    ) -> Result<Option<Committed>, FinanceError> {
        let request = ReleaseRequest {
            event_id: EventId::from_array(self.id()),
            entry_id: EntryId::from_array(self.id()),
            reason,
            now_ms: self.clock.now_ms(),
        };
        self.run_flow("release", json!({"attempt": attempt.hex()}), |document| {
            let Some(snapshot) = parse_held_snapshot(document)? else {
                return Ok(None);
            };
            plan_release(&snapshot, &request)
                .map(Some)
                .map_err(FinanceError::Settle)
        })
        .await
    }

    /// 12.5: an operator credit of a charge, returned to its original lots.
    pub async fn credit(&mut self, command: CreditCommand) -> Result<Committed, FinanceError> {
        let request = CreditRequest {
            event_id: EventId::from_array(self.id()),
            entry_id: EntryId::from_array(self.id()),
            operation: command.operation,
            amount_micro_dollars: command.amount_micro_dollars,
            reason: command.reason,
            actor_id: command.actor_id,
            case_ref: command.case_ref,
            approver_id: command.approver_id,
            now_ms: self.clock.now_ms(),
        };
        let args = json!({"usage_event": command.usage_event.hex()});
        let committed = self
            .run_flow("credit", args, |document| {
                let snapshot = parse_credit_snapshot(document)?;
                plan_credit(&snapshot, &request)
                    .map(Some)
                    .map_err(FinanceError::Settle)
            })
            .await?;
        committed.ok_or(FinanceError::Stale)
    }

    async fn run_attempt_flow(
        &mut self,
        attempt: AttemptId,
        decide: impl FnOnce(
            &df_commerce::snapshot::AttemptRow,
        ) -> Result<
            df_commerce::plan::FinancePlan,
            df_commerce::lifecycle::LifecycleRefusal,
        >,
    ) -> Result<Committed, FinanceError> {
        let committed = self
            .run_flow("attempt", json!({"attempt": attempt.hex()}), |document| {
                let row = document
                    .get("attempt")
                    .filter(|row| !row.is_null())
                    .ok_or(FinanceError::Stale)?;
                let row = parse_attempt_row(row)?;
                decide(&row).map(Some).map_err(FinanceError::Lifecycle)
            })
            .await?;
        committed.ok_or(FinanceError::Stale)
    }

    /// 11.3: `Reserved -> DispatchPrepared`. TODO(FIN-06): the runtime-placement check is O1's definer.
    pub async fn prepare_dispatch(
        &mut self,
        attempt: AttemptId,
        deadlines: Deadlines,
    ) -> Result<Committed, FinanceError> {
        let event_id = EventId::from_array(self.id());
        let now_ms = self.clock.now_ms();
        self.run_attempt_flow(attempt, |row| {
            plan_prepare_dispatch(event_id, row, deadlines, now_ms)
        })
        .await
    }

    /// 11.3: `DispatchPrepared -> Sent`. TODO(FIN-06): `current_dispatcher()` and the worker match.
    pub async fn mark_attempt_sent(
        &mut self,
        attempt: AttemptId,
        dispatcher_instance: [u8; 16],
        deadlines: Deadlines,
    ) -> Result<Committed, FinanceError> {
        let event_id = EventId::from_array(self.id());
        let now_ms = self.clock.now_ms();
        self.run_attempt_flow(attempt, |row| {
            plan_mark_sent(event_id, row, dispatcher_instance, deadlines, now_ms)
        })
        .await
    }

    /// 9.3: stores a quote for a session.
    pub async fn create_quote(
        &mut self,
        version: &PriceVersion,
        command: QuoteCommand,
    ) -> Result<QuoteId, FinanceError> {
        let quote = QuoteId::from_array(self.id());
        let request = QuoteRequest {
            event_id: EventId::from_array(self.id()),
            quote,
            session: command.session,
            route: command.route,
            work_class: command.work_class,
            maximum_units: command.maximum_units,
            estimate_units: command.estimate_units,
            request_digest: command.request_digest,
            lifetime_ms: command.lifetime_ms,
            now_ms: self.clock.now_ms(),
        };
        let plan = plan_quote(version, &request).map_err(FinanceError::Lifecycle)?;
        self.record_event(&plan).await?;
        Ok(quote)
    }

    /// 9.3: approves or declines an open quote of `session`.
    pub async fn decide_quote(
        &mut self,
        session: SessionKey,
        quote: QuoteId,
        approve: bool,
    ) -> Result<Committed, FinanceError> {
        let event_id = EventId::from_array(self.id());
        let now_ms = self.clock.now_ms();
        let args = json!({"session": session.hex(), "quote": quote.hex()});
        let committed = self
            .run_flow("quote", args, |document| {
                let row = parse_quote_row(document.get("quote").unwrap_or(&Value::Null))?
                    .ok_or(FinanceError::Stale)?;
                plan_quote_decision(event_id, session, &row, approve, now_ms)
                    .map(Some)
                    .map_err(FinanceError::Lifecycle)
            })
            .await?;
        committed.ok_or(FinanceError::Stale)
    }
}
