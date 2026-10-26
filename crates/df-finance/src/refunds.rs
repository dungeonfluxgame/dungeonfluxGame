//! Refund and dispute service operations (planning/finance-ledger-research.md 14, FIN-05):
//! each is one locked flow plus one `finance.commit_event`, and the processor call of a
//! refund lot happens between its snapshot and its payment commit, keyed by the refund
//! lot id, so a timeout leaves the lot `unknown` and its credit unusable until the
//! processor conclusively confirms (REF-02).

use df_commerce::dispute::{
    DisputeCloseRequest, DisputeOpenRequest, DisputeOutcome, plan_dispute_close, plan_dispute_open,
};
use df_commerce::finance_ids::{
    EntryId, EventId, LotId, OperationKey, PayerId, RefundId, RefundLotId,
};
use df_commerce::ledger::CurrencyTag;
use df_commerce::refund::{
    RefundPaidRequest, RefundRequest, plan_refund_observed, plan_refund_paid, plan_refund_request,
    plan_refund_returned,
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
    Paid { processor_refund_ref: String },
    Unknown,
    KnownFailure,
    AlreadyPaid,
    BlockedByDispute,
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
        let args = json!({"refund_lot": refund_lot.hex()});
        let peek = {
            let transaction = self.begin().await?;
            let document = Self::lock(&transaction, "refund_lot", args.clone()).await?;
            transaction.rollback().await?;
            parse_refund_lot_snapshot(&document)?
        };
        let Some(snapshot) = peek else {
            return Err(FinanceError::Stale);
        };
        if snapshot.state == "paid" {
            return Ok(RefundPayOutcome::AlreadyPaid);
        }
        if snapshot.state == "blocked_by_dispute" {
            return Ok(RefundPayOutcome::BlockedByDispute);
        }
        let call = processor
            .refund(&refund_lot.hex(), snapshot.principal_micro_dollars)
            .await;
        match call {
            RefundCall::Paid {
                processor_refund_ref,
            } => {
                let request = RefundPaidRequest {
                    event_id: EventId::from_array(self.id()),
                    entry_id: EntryId::from_array(self.id()),
                    processor_refund_ref: processor_refund_ref.clone(),
                    now_ms: self.clock.now_ms(),
                };
                let committed = self
                    .run_flow("refund_lot", args, |document| {
                        let Some(snapshot) = parse_refund_lot_snapshot(document)? else {
                            return Err(FinanceError::Stale);
                        };
                        plan_refund_paid(&snapshot, &request).map_err(FinanceError::Refund)
                    })
                    .await?;
                if committed.is_none() {
                    return Ok(RefundPayOutcome::AlreadyPaid);
                }
                Ok(RefundPayOutcome::Paid {
                    processor_refund_ref,
                })
            }
            RefundCall::Unknown => {
                let event_id = EventId::from_array(self.id());
                let now_ms = self.clock.now_ms();
                self.run_flow("refund_lot", args, |document| {
                    let Some(snapshot) = parse_refund_lot_snapshot(document)? else {
                        return Err(FinanceError::Stale);
                    };
                    plan_refund_observed(&snapshot, event_id, true, now_ms)
                        .map_err(FinanceError::Refund)
                })
                .await?;
                Ok(RefundPayOutcome::Unknown)
            }
            RefundCall::KnownFailure => {
                let event_id = EventId::from_array(self.id());
                let now_ms = self.clock.now_ms();
                self.run_flow("refund_lot", args, |document| {
                    let Some(snapshot) = parse_refund_lot_snapshot(document)? else {
                        return Err(FinanceError::Stale);
                    };
                    plan_refund_observed(&snapshot, event_id, false, now_ms)
                        .map_err(FinanceError::Refund)
                })
                .await?;
                Ok(RefundPayOutcome::KnownFailure)
            }
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
                plan_dispute_close(&snapshot, &request).map_err(FinanceError::Dispute)
            },
        )
        .await
    }
}
