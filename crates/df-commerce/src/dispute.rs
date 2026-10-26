//! Disputes and chargebacks (planning/finance-ledger-research.md 14.2, 16.10): opening a
//! dispute freezes only the disputed lot's available and unreserved allocated balances.
//! Reserved shares stay reserved and other payers' lots are never named, so no charge can
//! shift to another contributor (REF-03). A loss extinguishes frozen funds first, then
//! whole pending refund lots, then books the rest as chargeback loss; it never goes negative.

use crate::digest::business_key;
use crate::finance_ids::{AllocationId, EntryId, EventId, PayerId, RefundLotId};
use crate::funding::{LotClass, LotState};
use crate::keys::Keys;
use crate::ledger::{
    Actor, Correlation, CurrencyTag, EntryKind, EntryReason, EntryRefusal, ExternalRefKind,
    JournalEntry, Line, StreamKind,
};
use crate::plan::{ChangeKind, EventKind, FinancePlan, PlanRefusal, Row, RowChange};
use crate::snapshot::LotRow;

/// Why a dispute flow refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisputeRefusal {
    AlreadyOpen,
    NotOpen,
    LotNotPurchased,
    Arithmetic,
    Entry(EntryRefusal),
    Plan(PlanRefusal),
}

impl From<EntryRefusal> for DisputeRefusal {
    fn from(refusal: EntryRefusal) -> Self {
        Self::Entry(refusal)
    }
}
impl From<PlanRefusal> for DisputeRefusal {
    fn from(refusal: PlanRefusal) -> Self {
        Self::Plan(refusal)
    }
}

/// One allocation's unreserved balance on the disputed lot.
#[derive(Clone, Debug)]
pub struct DisputeAllocation {
    pub allocation: AllocationId,
    pub allocated_micro_dollars: u64,
    pub returned_micro_dollars: u64,
    pub revision: i64,
}

/// One pending refund lot on the disputed lot.
#[derive(Clone, Debug)]
pub struct DisputeRefundLot {
    pub refund_lot: RefundLotId,
    pub principal_micro_dollars: u64,
    pub state: String,
    pub object_revision: i64,
    pub revision: i64,
}

/// What a dispute flow read under its locks.
#[derive(Clone, Debug)]
pub struct DisputeSnapshot {
    pub payer: PayerId,
    pub currency: CurrencyTag,
    pub lot: LotRow,
    pub available_micro_dollars: u64,
    pub frozen_micro_dollars: u64,
    pub allocations: Vec<DisputeAllocation>,
    pub refund_lots: Vec<DisputeRefundLot>,
    pub dispute_exists: bool,
    pub dispute_open: bool,
    pub dispute_revision: i64,
    pub disputed_micro_dollars: u64,
    pub payment_ref: Option<String>,
    pub payment_disputed_micro_dollars: u64,
    pub payment_object_revision: i64,
    pub payment_revision: i64,
}

#[derive(Clone, Debug)]
pub struct DisputeOpenRequest {
    pub event_id: EventId,
    pub freeze_entry_id: EntryId,
    pub withheld_entry_id: EntryId,
    pub processor: String,
    pub dispute_ref: String,
    pub disputed_micro_dollars: u64,
    pub fee_micro_dollars: u64,
    pub now_ms: i64,
}

/// 14.2, opened: the lot freezes; its available and unreserved allocated funds move to
/// `purchased_frozen`, pending refund lots block, and the processor's withdrawal posts.
/// Only this lot's keys are named. `Ok(None)` for a dispute already recorded.
pub fn plan_dispute_open(
    snapshot: &DisputeSnapshot,
    request: &DisputeOpenRequest,
) -> Result<Option<FinancePlan>, DisputeRefusal> {
    if snapshot.dispute_exists {
        return Ok(None);
    }
    let processor = snapshot
        .lot
        .processor
        .as_deref()
        .ok_or(DisputeRefusal::LotNotPurchased)?;
    if snapshot.lot.class != LotClass::Purchased {
        return Err(DisputeRefusal::LotNotPurchased);
    }
    let keys = Keys::new(snapshot.currency);
    let mut plan = FinancePlan::new(request.event_id, EventKind::DisputeOpened);
    plan.changes.push(RowChange::insert(
        ChangeKind::DisputeInsert,
        Row::new()
            .text("processor", &request.processor)
            .text("dispute_ref", &request.dispute_ref)
            .bytes("lot_id", snapshot.lot.lot.as_bytes())
            .uint("disputed_micro_dollars", request.disputed_micro_dollars)
            .text("status", "open")
            .uint("fee_micro_dollars", request.fee_micro_dollars)
            .int("opened_at_ms", request.now_ms),
    ));
    plan.changes
        .push(RowChange::key(&keys.frozen(snapshot.lot.lot)));
    plan.changes
        .push(RowChange::key(&keys.dispute_withheld(processor)));
    plan.changes
        .push(RowChange::key(&keys.processing_fees(processor)));
    plan.changes
        .push(RowChange::key(&keys.chargeback_loss(processor)));
    let freeze_total = snapshot.allocations.iter().try_fold(
        snapshot.available_micro_dollars,
        |sum, allocation| {
            sum.checked_add(allocation.allocated_micro_dollars)
                .ok_or(DisputeRefusal::Arithmetic)
        },
    )?;
    if freeze_total > 0 {
        let mut lines = Vec::new();
        if snapshot.available_micro_dollars > 0 {
            lines.push(Line::debit(
                keys.available(LotClass::Purchased, snapshot.payer, snapshot.lot.lot),
                snapshot.available_micro_dollars,
            )?);
        }
        for allocation in &snapshot.allocations {
            if allocation.allocated_micro_dollars == 0 {
                continue;
            }
            lines.push(Line::debit(
                keys.allocated(LotClass::Purchased, allocation.allocation, snapshot.lot.lot),
                allocation.allocated_micro_dollars,
            )?);
            plan.changes.push(RowChange::update(
                ChangeKind::AllocationUpdate,
                Row::new()
                    .bytes("allocation_id", allocation.allocation.as_bytes())
                    .uint(
                        "returned_micro_dollars",
                        allocation
                            .returned_micro_dollars
                            .checked_add(allocation.allocated_micro_dollars)
                            .ok_or(DisputeRefusal::Arithmetic)?,
                    ),
                allocation.revision,
            ));
        }
        lines.push(Line::credit(keys.frozen(snapshot.lot.lot), freeze_total)?);
        plan.entries.push(JournalEntry {
            entry_id: request.freeze_entry_id,
            kind: EntryKind::DisputeFreeze,
            business_key: business_key(
                "dispute_freeze",
                &[request.processor.as_bytes(), request.dispute_ref.as_bytes()],
            ),
            external_ref: Some((
                ExternalRefKind::ProcessorDispute,
                format!("{}|{}", request.processor, request.dispute_ref),
            )),
            stream: (StreamKind::Payer, *snapshot.payer.as_bytes()),
            effective_at_ms: request.now_ms,
            reason: EntryReason::DisputeOpened,
            actor: Actor::System,
            correlation: Correlation::default(),
            lines,
        });
    }
    plan.changes.push(RowChange::update(
        ChangeKind::FundingLotDisputeUpdate,
        Row::new()
            .bytes("lot_id", snapshot.lot.lot.as_bytes())
            .text("state", "frozen"),
        snapshot.lot.revision,
    ));
    for refund_lot in &snapshot.refund_lots {
        if refund_lot.state == "pending" || refund_lot.state == "manual_pending" {
            plan.changes.push(RowChange::update(
                ChangeKind::RefundLotUpdate,
                Row::new()
                    .bytes("refund_lot_id", refund_lot.refund_lot.as_bytes())
                    .text("state", "blocked_by_dispute")
                    .int("object_revision", refund_lot.object_revision + 1)
                    .int("updated_at_ms", request.now_ms),
                refund_lot.revision,
            ));
        }
    }
    if let Some(payment_ref) = &snapshot.payment_ref {
        plan.changes.push(RowChange::update(
            ChangeKind::ProcessorPaymentStateUpdate,
            Row::new()
                .text("processor", processor)
                .text("payment_ref", payment_ref)
                .text("dispute_ref", &request.dispute_ref)
                .text("dispute_status", "open")
                .uint("disputed_micro_dollars", request.disputed_micro_dollars)
                .int("object_revision", snapshot.payment_object_revision + 1)
                .int("applied_at_ms", request.now_ms),
            snapshot.payment_revision,
        ));
    }
    let mut withheld_lines = vec![
        Line::debit(
            keys.dispute_withheld(processor),
            request.disputed_micro_dollars,
        )?,
        Line::credit(
            keys.clearing(processor),
            request
                .disputed_micro_dollars
                .checked_add(request.fee_micro_dollars)
                .ok_or(DisputeRefusal::Arithmetic)?,
        )?,
    ];
    if request.fee_micro_dollars > 0 {
        withheld_lines.insert(
            1,
            Line::debit(keys.processing_fees(processor), request.fee_micro_dollars)?,
        );
    }
    plan.entries.push(JournalEntry {
        entry_id: request.withheld_entry_id,
        kind: EntryKind::DisputeWithheld,
        business_key: business_key(
            "dispute_withheld",
            &[request.processor.as_bytes(), request.dispute_ref.as_bytes()],
        ),
        external_ref: Some((
            ExternalRefKind::ProcessorDispute,
            format!("{}|{}", request.processor, request.dispute_ref),
        )),
        stream: (StreamKind::Platform, *snapshot.payer.as_bytes()),
        effective_at_ms: request.now_ms,
        reason: EntryReason::DisputeOpened,
        actor: Actor::System,
        correlation: Correlation::default(),
        lines: withheld_lines,
    });
    plan.check().map_err(DisputeRefusal::Plan)?;
    Ok(Some(plan))
}

/// Whether a closed dispute was won or lost, and for how much the processor took.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisputeOutcome {
    Won,
    Lost { disputed_micro_dollars: u64 },
}

#[derive(Clone, Debug)]
pub struct DisputeCloseRequest {
    pub event_id: EventId,
    pub entry_id: EntryId,
    pub processor: String,
    pub dispute_ref: String,
    pub outcome: DisputeOutcome,
    pub now_ms: i64,
}

/// 14.2, closed. Won returns frozen funds to available and unblocks refund lots. Lost
/// extinguishes in the fixed order and releases any frozen remainder; only this lot's
/// keys are named either way.
pub fn plan_dispute_close(
    snapshot: &DisputeSnapshot,
    request: &DisputeCloseRequest,
) -> Result<Option<FinancePlan>, DisputeRefusal> {
    if !snapshot.dispute_open {
        return Ok(None);
    }
    let processor = snapshot
        .lot
        .processor
        .as_deref()
        .ok_or(DisputeRefusal::LotNotPurchased)?;
    let keys = Keys::new(snapshot.currency);
    let frozen = snapshot.frozen_micro_dollars;
    let mut plan = FinancePlan::new(request.event_id, EventKind::DisputeClosed);
    plan.changes.push(RowChange::update(
        ChangeKind::DisputeUpdate,
        Row::new()
            .text("processor", &request.processor)
            .text("dispute_ref", &request.dispute_ref)
            .text(
                "status",
                match request.outcome {
                    DisputeOutcome::Won => "won",
                    DisputeOutcome::Lost { .. } => "lost",
                },
            )
            .int("closed_at_ms", request.now_ms),
        snapshot.dispute_revision,
    ));
    let (kind, reason, lines, lot_state, charged_back, extinguished) = match request.outcome {
        DisputeOutcome::Won => {
            let mut lines = Vec::new();
            if frozen > 0 {
                lines.push(Line::debit(keys.frozen(snapshot.lot.lot), frozen)?);
                lines.push(Line::credit(
                    keys.available(LotClass::Purchased, snapshot.payer, snapshot.lot.lot),
                    frozen,
                )?);
            }
            if snapshot.disputed_micro_dollars > 0 {
                lines.push(Line::debit(
                    keys.clearing(processor),
                    snapshot.disputed_micro_dollars,
                )?);
                lines.push(Line::credit(
                    keys.dispute_withheld(processor),
                    snapshot.disputed_micro_dollars,
                )?);
            }
            (
                EntryKind::DisputeWon,
                EntryReason::DisputeResolved,
                lines,
                LotState::Active,
                snapshot.lot.charged_back_micro_dollars,
                snapshot.lot.extinguished_micro_dollars,
            )
        }
        DisputeOutcome::Lost {
            disputed_micro_dollars: taken,
        } => {
            let x1 = frozen.min(taken);
            let mut remainder = taken - x1;
            let mut x2: u64 = 0;
            for refund_lot in &snapshot.refund_lots {
                if remainder == 0 {
                    break;
                }
                if matches!(
                    refund_lot.state.as_str(),
                    "pending" | "failed_review" | "manual_pending" | "blocked_by_dispute"
                ) && remainder >= refund_lot.principal_micro_dollars
                {
                    x2 = x2
                        .checked_add(refund_lot.principal_micro_dollars)
                        .ok_or(DisputeRefusal::Arithmetic)?;
                    remainder -= refund_lot.principal_micro_dollars;
                    plan.changes.push(RowChange::update(
                        ChangeKind::RefundLotUpdate,
                        Row::new()
                            .bytes("refund_lot_id", refund_lot.refund_lot.as_bytes())
                            .text("state", "cancelled_dispute_lost")
                            .int("object_revision", refund_lot.object_revision + 1)
                            .int("updated_at_ms", request.now_ms),
                        refund_lot.revision,
                    ));
                }
            }
            let x3 = remainder;
            let released = frozen - x1;
            let mut lines = Vec::new();
            if frozen > 0 {
                lines.push(Line::debit(keys.frozen(snapshot.lot.lot), frozen)?);
            }
            if x2 > 0 {
                lines.push(Line::debit(keys.refund_payable(snapshot.lot.lot), x2)?);
            }
            if x3 > 0 {
                lines.push(Line::debit(keys.chargeback_loss(processor), x3)?);
            }
            lines.push(Line::credit(keys.dispute_withheld(processor), taken)?);
            if released > 0 {
                lines.push(Line::credit(
                    keys.available(LotClass::Purchased, snapshot.payer, snapshot.lot.lot),
                    released,
                )?);
            }
            let charged_back = snapshot
                .lot
                .charged_back_micro_dollars
                .checked_add(taken)
                .ok_or(DisputeRefusal::Arithmetic)?;
            let lot_state = if charged_back
                .checked_add(snapshot.lot.refunded_micro_dollars)
                .ok_or(DisputeRefusal::Arithmetic)?
                >= snapshot.lot.original_micro_dollars
            {
                LotState::ChargedBack
            } else {
                LotState::Active
            };
            (
                EntryKind::DisputeLost,
                EntryReason::DisputeResolved,
                lines,
                lot_state,
                charged_back,
                snapshot
                    .lot
                    .extinguished_micro_dollars
                    .checked_add(x1 + x2)
                    .ok_or(DisputeRefusal::Arithmetic)?,
            )
        }
    };
    if matches!(request.outcome, DisputeOutcome::Won) {
        for refund_lot in &snapshot.refund_lots {
            if refund_lot.state == "blocked_by_dispute" {
                plan.changes.push(RowChange::update(
                    ChangeKind::RefundLotUpdate,
                    Row::new()
                        .bytes("refund_lot_id", refund_lot.refund_lot.as_bytes())
                        .text("state", "pending")
                        .int("object_revision", refund_lot.object_revision + 1)
                        .int("updated_at_ms", request.now_ms),
                    refund_lot.revision,
                ));
            }
        }
    }
    plan.changes.push(RowChange::update(
        ChangeKind::FundingLotDisputeUpdate,
        Row::new()
            .bytes("lot_id", snapshot.lot.lot.as_bytes())
            .text("state", lot_state_label(lot_state))
            .uint("charged_back_micro_dollars", charged_back)
            .uint("extinguished_micro_dollars", extinguished),
        snapshot.lot.revision,
    ));
    if let Some(payment_ref) = &snapshot.payment_ref {
        plan.changes.push(RowChange::update(
            ChangeKind::ProcessorPaymentStateUpdate,
            Row::new()
                .text("processor", processor)
                .text("payment_ref", payment_ref)
                .text(
                    "dispute_status",
                    match request.outcome {
                        DisputeOutcome::Won => "won",
                        DisputeOutcome::Lost { .. } => "lost",
                    },
                )
                .int("object_revision", snapshot.payment_object_revision + 1)
                .int("applied_at_ms", request.now_ms),
            snapshot.payment_revision,
        ));
    }
    if !lines.is_empty() {
        plan.entries.push(JournalEntry {
            entry_id: request.entry_id,
            kind,
            business_key: business_key(
                "dispute_close",
                &[request.processor.as_bytes(), request.dispute_ref.as_bytes()],
            ),
            external_ref: Some((
                ExternalRefKind::ProcessorDispute,
                format!("{}|{}", request.processor, request.dispute_ref),
            )),
            stream: (StreamKind::Payer, *snapshot.payer.as_bytes()),
            effective_at_ms: request.now_ms,
            reason,
            actor: Actor::System,
            correlation: Correlation::default(),
            lines,
        });
    }
    plan.check().map_err(DisputeRefusal::Plan)?;
    Ok(Some(plan))
}

fn lot_state_label(state: LotState) -> &'static str {
    match state {
        LotState::Active => "active",
        LotState::Frozen => "frozen",
        LotState::ChargedBack => "charged_back",
        LotState::Closed => "closed",
        LotState::Forfeited => "forfeited",
    }
}
