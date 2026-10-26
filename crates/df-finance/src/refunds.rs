//! Refund and dispute service operations (planning/finance-ledger-research.md 14, FIN-05):
//! processor dispatch is leased durably under the refund locks before any external call.
//! Paid observations survive intervening disputes as reconciliation receipts (REF-02).

use df_commerce::digest::business_key;
use df_commerce::dispute::{
    DisputeCloseRequest, DisputeOpenRequest, DisputeOutcome, plan_dispute_close, plan_dispute_open,
};
use df_commerce::finance_ids::{
    EntryId, EventId, LotId, OperationKey, PayerId, RefundId, RefundLotId,
};
use df_commerce::funding::LotState;
use df_commerce::ledger::CurrencyTag;
use df_commerce::plan::{ChangeKind, EventKind, FinancePlan, Row, RowChange};
use df_commerce::refund::{
    RefundPaidRequest, RefundRefusal, RefundRequest, plan_refund_observed, plan_refund_paid,
    plan_refund_request, plan_refund_returned,
};
use serde_json::json;

use crate::ports::{FencePort, MediaPort, MemberPort, RefundCall, RefundProcessor};
use crate::service::{Committed, FinanceError, FinanceService};
use crate::wire::{
    parse_dispute_snapshot, parse_refund_lot_snapshot, parse_refund_request_snapshot,
};

/// What paying one refund lot concluded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RefundPayOutcome {
    Paid {
        processor_refund_ref: String,
    },
    Unknown,
    KnownFailure,
    AlreadyPaid,
    BlockedByDispute,
    /// Confirmed cash retained in a durable receipt after the lot stopped being payable.
    ReconciliationRequired {
        processor_refund_ref: String,
    },
}

enum RefundClaim {
    Dispatch {
        owner: [u8; 16],
        amount_micro_dollars: u64,
        lease_until_ms: i64,
    },
    Complete(RefundPayOutcome),
}

impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    /// 14.1: requests a refund of unspent purchased credit, newest lots first.
    /// Idempotent by `(payer, operation)`: a retry returns the first refund's id.
    pub async fn request_refund(
        &mut self,
        payer: PayerId,
        operation: OperationKey,
        amount_micro_dollars: u64,
        currency: CurrencyTag,
    ) -> Result<RefundId, FinanceError> {
        let refund = RefundId::from_array(self.id());
        let request = RefundRequest {
            event_id: EventId::from_array(self.id()),
            entry_id: EntryId::from_array(self.id()),
            refund,
            refund_lot_ids: (0..16)
                .map(|_| RefundLotId::from_array(self.id()))
                .collect(),
            operation,
            amount_micro_dollars,
            now_ms: self.clock.now_ms(),
        };
        let args = json!({"payer": payer.hex(), "operation": operation.hex()});
        let mut chosen = refund;
        self.run_flow("refund_request", args, |document| {
            let snapshot = parse_refund_request_snapshot(document, payer, currency)?;
            if let Some(existing) = snapshot.existing_refund {
                chosen = existing;
                return Ok(None);
            }
            match plan_refund_request(&snapshot, &request) {
                Ok(Some(plan)) => Ok(Some(plan)),
                Ok(None) => Ok(None),
                Err(refusal) => Err(FinanceError::Refund(refusal)),
            }
        })
        .await?;
        Ok(chosen)
    }

    /// Drives one refund lot through the processor. The idempotency key is the refund
    /// lot id, so the retry after a timeout pays at most once (REF-02).
    pub async fn pay_refund_lot<P: RefundProcessor>(
        &mut self,
        refund_lot: RefundLotId,
        processor: &P,
    ) -> Result<RefundPayOutcome, FinanceError> {
        let (owner, amount_micro_dollars, lease_until_ms) =
            match self.claim_refund_lot(refund_lot).await? {
                RefundClaim::Dispatch {
                    owner,
                    amount_micro_dollars,
                    lease_until_ms,
                } => (owner, amount_micro_dollars, lease_until_ms),
                RefundClaim::Complete(outcome) => return Ok(outcome),
            };
        let remaining_ms = lease_until_ms
            .checked_sub(self.clock.now_ms())
            .and_then(|remaining| u64::try_from(remaining).ok())
            .filter(|remaining| *remaining > 0);
        let Some(remaining_ms) = remaining_ms else {
            return Ok(RefundPayOutcome::Unknown);
        };
        // Cancellation leaves the durable unknown claim. Expiry permits only the same processor key.
        let call = tokio::time::timeout(
            std::time::Duration::from_millis(self.config.port_deadline_ms.min(remaining_ms)),
            processor.refund(&refund_lot.hex(), amount_micro_dollars),
        )
        .await
        .unwrap_or(RefundCall::Unknown);
        if let RefundCall::Paid {
            processor_refund_ref,
        } = &call
        {
            self.record_paid_observation(refund_lot, amount_micro_dollars, processor_refund_ref)
                .await?;
        }
        self.observe_refund_call(refund_lot, owner, call).await
    }

    async fn claim_refund_lot(
        &mut self,
        refund_lot: RefundLotId,
    ) -> Result<RefundClaim, FinanceError> {
        let args = json!({"refund_lot": refund_lot.hex()});
        let claim = self.id();
        let event_id = EventId::from_array(self.id());
        let deadline_ms = self.config.port_deadline_ms;
        let transaction = Self::start(&mut self.client).await?;
        let document = Self::lock(&transaction, "refund_lot", args).await?;
        let now_ms = self.clock.now_ms();
        let lease_until_ms = now_ms
            .checked_add(
                i64::try_from(deadline_ms)
                    .map_err(|_| FinanceError::Refund(RefundRefusal::Arithmetic))?,
            )
            .and_then(|value| value.checked_add(1_000))
            .ok_or(FinanceError::Refund(RefundRefusal::Arithmetic))?;
        let mut amount_micro_dollars = 0;
        let mut stopped = None;
        let plan = (|document: &serde_json::Value| {
            let snapshot = parse_refund_lot_snapshot(document)?.ok_or(FinanceError::Stale)?;
            if snapshot.state == "paid" {
                stopped = Some(RefundPayOutcome::AlreadyPaid);
                return Ok(None);
            }
            if document
                .get("paid_observation")
                .is_some_and(|value| !value.is_null())
            {
                return Err(FinanceError::Refund(RefundRefusal::RefundLotNotPayable));
            }
            if snapshot.state == "blocked_by_dispute"
                || matches!(snapshot.lot_state, LotState::Frozen | LotState::ChargedBack)
            {
                stopped = Some(RefundPayOutcome::BlockedByDispute);
                return Ok(None);
            }
            if !matches!(snapshot.state.as_str(), "pending" | "unknown")
                || document["refund_lot"]["payout_method"].as_str() != Some("processor")
            {
                return Err(FinanceError::Refund(RefundRefusal::RefundLotNotPayable));
            }
            if document["outbox"]["lease_until_ms"]
                .as_i64()
                .is_some_and(|until| until > now_ms)
            {
                return Err(FinanceError::Stale);
            }
            amount_micro_dollars = snapshot.principal_micro_dollars;
            let mut plan = plan_refund_observed(&snapshot, event_id, true, now_ms)
                .map_err(FinanceError::Refund)?
                .unwrap_or_else(|| FinancePlan::new(event_id, EventKind::RefundUnknown));
            plan.changes
                .retain(|change| change.kind != ChangeKind::RefundOutboxUpdate);
            plan.changes.push(RowChange::update(
                ChangeKind::RefundOutboxUpdate,
                Row::new()
                    .bytes("refund_lot_id", refund_lot.as_bytes())
                    .bytes("claimed_by", &claim)
                    .int("lease_until_ms", lease_until_ms)
                    .int("attempts", snapshot.outbox_attempts + 1)
                    .int("next_attempt_at_ms", now_ms),
                snapshot.outbox_revision,
            ));
            Ok(Some(plan))
        })(&document)?;
        if let Some(plan) = plan {
            Self::commit_plan(&transaction, &plan).await?;
            transaction.commit().await?;
        } else {
            transaction.rollback().await?;
        }
        Ok(match stopped {
            Some(outcome) => RefundClaim::Complete(outcome),
            None => RefundClaim::Dispatch {
                owner: claim,
                amount_micro_dollars,
                lease_until_ms,
            },
        })
    }

    async fn record_paid_observation(
        &mut self,
        refund_lot: RefundLotId,
        principal_micro_dollars: u64,
        processor_refund_ref: &str,
    ) -> Result<(), FinanceError> {
        let event_id = EventId::from_array(self.id());
        let operation = OperationKey::from_array(self.id());
        let now_ms = self.clock.now_ms();
        let transaction = self.begin().await?;
        // This receipt lock never waits for disputed funding or refund rows.
        transaction
            .query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
                &[&format!("finance.refund_paid:{}", refund_lot.hex())],
            )
            .await?;
        let existing = transaction
            .query_opt(
                "SELECT outcome::text FROM finance.operation_receipt \
             WHERE namespace = 'refund_processor_paid' AND principal = $1 \
               AND outcome ->> 'processor_refund_ref' = $2 LIMIT 1",
                &[&refund_lot.as_bytes().to_vec(), &processor_refund_ref],
            )
            .await?;
        if let Some(existing) = existing {
            let observation: serde_json::Value =
                serde_json::from_str(&existing.get::<_, String>(0))
                    .map_err(|error| crate::service::bad_sql(&error.to_string()))?;
            if observation["principal_micro_dollars"].as_u64() != Some(principal_micro_dollars) {
                return Err(crate::service::bad_sql(
                    "conflicting refund paid observation amount",
                ));
            }
            transaction.rollback().await?;
            return Ok(());
        }
        let mut plan = FinancePlan::new(event_id, EventKind::RefundUnknown);
        plan.changes.push(paid_receipt(
            refund_lot,
            operation,
            principal_micro_dollars,
            processor_refund_ref,
            now_ms,
        ));
        Self::commit_plan(&transaction, &plan).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn observe_refund_call(
        &mut self,
        refund_lot: RefundLotId,
        owner: [u8; 16],
        call: RefundCall,
    ) -> Result<RefundPayOutcome, FinanceError> {
        let event_id = EventId::from_array(self.id());
        let entry_id = EntryId::from_array(self.id());
        let now_ms = self.clock.now_ms();
        let mut reconciliation_required = false;
        self.run_flow(
            "refund_lot",
            json!({"refund_lot": refund_lot.hex()}),
            |document| {
                let (plan, needs_review) = plan_refund_completion(
                    document, refund_lot, owner, &call, event_id, entry_id, now_ms,
                )?;
                reconciliation_required = needs_review;
                Ok(plan)
            },
        )
        .await?;
        match call {
            RefundCall::Paid {
                processor_refund_ref,
            } if reconciliation_required => Ok(RefundPayOutcome::ReconciliationRequired {
                processor_refund_ref,
            }),
            RefundCall::Paid {
                processor_refund_ref,
            } => Ok(RefundPayOutcome::Paid {
                processor_refund_ref,
            }),
            RefundCall::Unknown => Ok(RefundPayOutcome::Unknown),
            RefundCall::KnownFailure => Ok(RefundPayOutcome::KnownFailure),
        }
    }

    /// Returns a failed refund lot to spendable credit (14.1).
    pub async fn return_refund_to_credit(
        &mut self,
        refund_lot: RefundLotId,
    ) -> Result<Option<Committed>, FinanceError> {
        let event_id = EventId::from_array(self.id());
        let entry_id = EntryId::from_array(self.id());
        let now_ms = self.clock.now_ms();
        self.run_flow(
            "refund_lot",
            json!({"refund_lot": refund_lot.hex()}),
            |document| {
                let Some(snapshot) = parse_refund_lot_snapshot(document)? else {
                    return Err(FinanceError::Stale);
                };
                if document
                    .get("paid_observation")
                    .is_some_and(|value| !value.is_null())
                {
                    return Err(FinanceError::Refund(RefundRefusal::RefundLotNotPayable));
                }
                plan_refund_returned(&snapshot, event_id, entry_id, now_ms)
                    .map_err(FinanceError::Refund)
            },
        )
        .await
    }

    /// 14.2: opens a dispute on one lot; only that lot's funds freeze (REF-03).
    pub async fn open_dispute(
        &mut self,
        lot: LotId,
        processor: &str,
        dispute_ref: &str,
        disputed_micro_dollars: u64,
        fee_micro_dollars: u64,
    ) -> Result<Option<Committed>, FinanceError> {
        let request = DisputeOpenRequest {
            event_id: EventId::from_array(self.id()),
            freeze_entry_id: EntryId::from_array(self.id()),
            withheld_entry_id: EntryId::from_array(self.id()),
            processor: processor.to_owned(),
            dispute_ref: dispute_ref.to_owned(),
            disputed_micro_dollars,
            fee_micro_dollars,
            now_ms: self.clock.now_ms(),
        };
        self.run_flow(
            "dispute_open",
            json!({"lot": lot.hex(), "processor": processor, "dispute_ref": dispute_ref}),
            |document| {
                let Some(snapshot) = parse_dispute_snapshot(document)? else {
                    return Err(FinanceError::Stale);
                };
                plan_dispute_open(&snapshot, &request).map_err(FinanceError::Dispute)
            },
        )
        .await
    }

    /// 14.2: closes an open dispute as won or lost for the given taken amount.
    pub async fn close_dispute(
        &mut self,
        lot: LotId,
        processor: &str,
        dispute_ref: &str,
        outcome: DisputeOutcome,
    ) -> Result<Option<Committed>, FinanceError> {
        let request = DisputeCloseRequest {
            event_id: EventId::from_array(self.id()),
            entry_id: EntryId::from_array(self.id()),
            processor: processor.to_owned(),
            dispute_ref: dispute_ref.to_owned(),
            outcome,
            now_ms: self.clock.now_ms(),
        };
        self.run_flow(
            "dispute_close",
            json!({"lot": lot.hex(), "processor": processor, "dispute_ref": dispute_ref}),
            |document| {
                let Some(snapshot) = parse_dispute_snapshot(document)? else {
                    return Err(FinanceError::Stale);
                };
                let mut plan =
                    plan_dispute_close(&snapshot, &request).map_err(FinanceError::Dispute)?;
                if let Some(plan) = &mut plan {
                    restore_manual_refunds(document, plan)?;
                }
                Ok(plan)
            },
        )
        .await
    }
}

fn plan_refund_completion(
    document: &serde_json::Value,
    refund_lot: RefundLotId,
    owner: [u8; 16],
    call: &RefundCall,
    event_id: EventId,
    entry_id: EntryId,
    now_ms: i64,
) -> Result<(Option<FinancePlan>, bool), FinanceError> {
    let snapshot = parse_refund_lot_snapshot(document)?.ok_or(FinanceError::Stale)?;
    if !matches!(call, RefundCall::Paid { .. })
        && document
            .get("paid_observation")
            .is_some_and(|value| !value.is_null())
    {
        let reconciliation_required =
            document["paid_observation"]["outcome"]["reconciliation_required"]
                .as_bool()
                .ok_or_else(|| {
                    crate::service::bad_sql("refund paid receipt reconciliation flag")
                })?;
        return Ok((None, reconciliation_required));
    }
    if let RefundCall::Paid {
        processor_refund_ref,
    } = call
        && snapshot.state == "paid"
        && document["refund_lot"]["processor_refund_ref"].as_str() != Some(processor_refund_ref)
    {
        return Ok((None, true));
    }
    let owns_claim = document["outbox"]["claimed_by"].as_str()
        == Some(format!("\\x{}", df_commerce::finance_ids::hex(&owner)).as_str());
    let reconciliation_required = false;
    let mut plan = match call {
        RefundCall::Paid {
            processor_refund_ref,
        } => {
            let request = RefundPaidRequest {
                event_id,
                entry_id,
                processor_refund_ref: processor_refund_ref.clone(),
                now_ms,
            };
            match plan_refund_paid(&snapshot, &request) {
                Ok(Some(plan)) => plan,
                Ok(None) => return Ok((None, false)),
                Err(RefundRefusal::BlockedByDispute | RefundRefusal::RefundLotNotPayable) => {
                    return Ok((None, true));
                }
                Err(refusal) => return Err(FinanceError::Refund(refusal)),
            }
        }
        RefundCall::KnownFailure | RefundCall::Unknown => {
            if !owns_claim || !matches!(snapshot.state.as_str(), "pending" | "unknown") {
                return Ok((None, false));
            }
            plan_refund_observed(&snapshot, event_id, *call == RefundCall::Unknown, now_ms)
                .map_err(FinanceError::Refund)?
                .unwrap_or_else(|| FinancePlan::new(event_id, EventKind::RefundUnknown))
        }
    };
    if owns_claim && !matches!(call, RefundCall::Paid { .. }) {
        plan.changes
            .retain(|change| change.kind != ChangeKind::RefundOutboxUpdate);
        plan.changes.push(RowChange::update(
            ChangeKind::RefundOutboxUpdate,
            Row::new()
                .bytes("refund_lot_id", refund_lot.as_bytes())
                .opt_bytes("claimed_by", None)
                .opt_int("lease_until_ms", None)
                .int("attempts", snapshot.outbox_attempts)
                .int("next_attempt_at_ms", now_ms),
            snapshot.outbox_revision,
        ));
    }
    Ok((Some(plan), reconciliation_required))
}

fn paid_receipt(
    refund_lot: RefundLotId,
    operation: OperationKey,
    principal_micro_dollars: u64,
    processor_refund_ref: &str,
    now_ms: i64,
) -> RowChange {
    // A matching paid journal entry resolves this immutable observation; until then it needs review.
    let outcome = json!({"processor_refund_ref": processor_refund_ref,
        "principal_micro_dollars": principal_micro_dollars,
        "state": "awaiting_posting", "reconciliation_required": true});
    RowChange::insert(
        ChangeKind::OperationReceiptInsert,
        Row::new()
            .text("namespace", "refund_processor_paid")
            .bytes("principal", refund_lot.as_bytes())
            .bytes("operation_id", operation.as_bytes())
            .bytes(
                "fingerprint",
                &business_key(
                    "refund_processor_paid",
                    &[refund_lot.as_bytes(), processor_refund_ref.as_bytes()],
                ),
            )
            .json("outcome", &outcome.to_string())
            .bytes("result_ref", refund_lot.as_bytes())
            .int("created_at_ms", now_ms),
    )
}

fn restore_manual_refunds(
    document: &serde_json::Value,
    plan: &mut FinancePlan,
) -> Result<(), FinanceError> {
    let refund_lots = document["refund_lots"]
        .as_array()
        .ok_or_else(|| crate::service::bad_sql("refund lots"))?;
    for change in &mut plan.changes {
        if change.kind != ChangeKind::RefundLotUpdate
            || change.row.get("state") != Some(&df_commerce::plan::Value::Text("pending".into()))
        {
            continue;
        }
        let is_manual = refund_lots.iter().any(|lot| {
            lot["payout_method"].as_str() == Some("manual")
                && lot["refund_lot_id"]
                    .as_str()
                    .and_then(|value| value.strip_prefix("\\x"))
                    .and_then(RefundLotId::from_hex)
                    .is_some_and(|id| {
                        change.row.get("refund_lot_id")
                            == Some(&df_commerce::plan::Value::Bytes(id.as_bytes().to_vec()))
                    })
        });
        if is_manual {
            change.row = std::mem::take(&mut change.row).text("state", "manual_pending");
        }
    }
    Ok(())
}
