//! Refunds of unspent purchased credit (planning/finance-ledger-research.md 14.1, 16.9):
//! a partial refund is split over the payer's available purchased lots, newest first,
//! in whole cents per lot, to the original processor payment of each lot. The requested
//! amount leaves the spendable balance at request time and stays in `refund_payable`
//! until the processor confirms; a timeout therefore cannot make it reusable or paid twice.

use crate::digest::business_key;
use crate::finance_ids::{EntryId, EventId, LotId, OperationKey, PayerId, RefundId, RefundLotId};
use crate::funding::{LotClass, LotState};
use crate::keys::Keys;
use crate::ledger::{
    Actor, Correlation, CurrencyTag, EntryKind, EntryReason, EntryRefusal, ExternalRefKind,
    JournalEntry, Line, StreamKind,
};
use crate::plan::{ChangeKind, EventKind, FinancePlan, PlanRefusal, Row, RowChange};

/// One cent in micro-dollars: cash refunds are whole cents per lot (14.1).
pub const CENT_MICRO_DOLLARS: u64 = 10_000;
/// Lots older than this are paid manually, not through the processor (14.1).
pub const PROCESSOR_WINDOW_MS: i64 = 180 * 24 * 3_600_000;

fn floor_cent(amount_micro_dollars: u64) -> u64 {
    amount_micro_dollars / CENT_MICRO_DOLLARS * CENT_MICRO_DOLLARS
}

/// Why a refund flow refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RefundRefusal {
    NothingToRefund,
    BlockedByDispute,
    RefundLotNotPayable,
    Arithmetic,
    Entry(EntryRefusal),
    Plan(PlanRefusal),
}

impl From<EntryRefusal> for RefundRefusal {
    fn from(refusal: EntryRefusal) -> Self {
        Self::Entry(refusal)
    }
}
impl From<PlanRefusal> for RefundRefusal {
    fn from(refusal: PlanRefusal) -> Self {
        Self::Plan(refusal)
    }
}

/// One purchased lot as the refund request flow reads it.
#[derive(Clone, Debug)]
pub struct RefundLotState {
    pub lot: LotId,
    pub currency: CurrencyTag,
    pub class: LotClass,
    pub state: LotState,
    pub processor: Option<String>,
    pub payment_ref: Option<String>,
    pub original_micro_dollars: u64,
    pub tax_micro_dollars: u64,
    pub tax_jurisdiction: Option<String>,
    pub created_at_ms: i64,
    pub available_micro_dollars: u64,
}

/// What the refund request flow read under its locks.
#[derive(Clone, Debug)]
pub struct RefundRequestSnapshot {
    pub payer: PayerId,
    pub currency: CurrencyTag,
    pub lots: Vec<RefundLotState>,
    pub existing_refund: Option<RefundId>,
}

/// A refund to request: `refund_lot_ids` is a pool the planner draws from, newest lot first.
#[derive(Clone, Debug)]
pub struct RefundRequest {
    pub event_id: EventId,
    pub entry_id: EntryId,
    pub refund: RefundId,
    pub refund_lot_ids: Vec<RefundLotId>,
    pub operation: OperationKey,
    pub amount_micro_dollars: u64,
    pub now_ms: i64,
}

/// 14.1: moves the refundable part of the request to `refund_payable`, one refund lot
/// per funding lot. `Ok(None)` when this operation was already requested (REF-02).
pub fn plan_refund_request(
    snapshot: &RefundRequestSnapshot,
    request: &RefundRequest,
) -> Result<Option<FinancePlan>, RefundRefusal> {
    if snapshot.existing_refund.is_some() {
        return Ok(None);
    }
    let mut need = floor_cent(request.amount_micro_dollars);
    if need == 0 {
        return Err(RefundRefusal::NothingToRefund);
    }
    let mut eligible: Vec<&RefundLotState> = snapshot
        .lots
        .iter()
        .filter(|lot| {
            lot.class == LotClass::Purchased
                && lot.state == LotState::Active
                && lot.available_micro_dollars >= CENT_MICRO_DOLLARS
        })
        .collect();
    eligible.sort_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| b.lot.cmp(&a.lot))
    });
    let keys = Keys::new(snapshot.currency);
    let mut plan = FinancePlan::new(request.event_id, EventKind::RefundRequested);
    let mut lines: Vec<Line> = Vec::new();
    let mut total: u64 = 0;
    let mut used = 0_usize;
    for lot in eligible {
        if need == 0 {
            break;
        }
        let take = need.min(floor_cent(lot.available_micro_dollars));
        if take == 0 {
            continue;
        }
        let refund_lot = request
            .refund_lot_ids
            .get(used)
            .copied()
            .ok_or(RefundRefusal::Arithmetic)?;
        used += 1;
        let tax = lot
            .tax_micro_dollars
            .checked_mul(take)
            .map(|value| value / lot.original_micro_dollars.max(1))
            .ok_or(RefundRefusal::Arithmetic)?;
        let manual = lot.processor.is_none()
            || lot.payment_ref.is_none()
            || request.now_ms.saturating_sub(lot.created_at_ms) > PROCESSOR_WINDOW_MS;
        plan.changes.push(RowChange::insert(
            ChangeKind::RefundLotInsert,
            Row::new()
                .bytes("refund_lot_id", refund_lot.as_bytes())
                .bytes("refund_id", request.refund.as_bytes())
                .bytes("lot_id", lot.lot.as_bytes())
                .uint("principal_micro_dollars", take)
                .uint("tax_micro_dollars", tax)
                .text("payout_method", if manual { "manual" } else { "processor" })
                .text("state", if manual { "manual_pending" } else { "pending" })
                .int("updated_at_ms", request.now_ms),
        ));
        if !manual {
            plan.changes.push(RowChange::insert(
                ChangeKind::RefundOutboxInsert,
                Row::new()
                    .bytes("refund_lot_id", refund_lot.as_bytes())
                    .int("next_attempt_at_ms", request.now_ms),
            ));
        }
        plan.changes
            .push(RowChange::key(&keys.refund_payable(lot.lot)));
        lines.push(Line::debit(
            keys.available(LotClass::Purchased, snapshot.payer, lot.lot),
            take,
        )?);
        lines.push(Line::credit(keys.refund_payable(lot.lot), take)?);
        total = total.checked_add(take).ok_or(RefundRefusal::Arithmetic)?;
        need -= take;
    }
    if total == 0 {
        return Err(RefundRefusal::NothingToRefund);
    }
    plan.changes.insert(
        0,
        RowChange::insert(
            ChangeKind::RefundInsert,
            Row::new()
                .bytes("refund_id", request.refund.as_bytes())
                .bytes("payer_account_id", snapshot.payer.as_bytes())
                .bytes("operation_id", request.operation.as_bytes())
                .uint("requested_micro_dollars", total)
                .text("state", "pending")
                .int("requested_at_ms", request.now_ms),
        ),
    );
    plan.changes.push(RowChange::insert(
        ChangeKind::OperationReceiptInsert,
        Row::new()
            .text("namespace", "refund")
            .bytes("principal", snapshot.payer.as_bytes())
            .bytes("operation_id", request.operation.as_bytes())
            .bytes(
                "fingerprint",
                &business_key("refund", &[request.operation.as_bytes()]),
            )
            .json(
                "outcome",
                &format!(
                    "{{\"refund\":\"{}\",\"total\":{total}}}",
                    request.refund.hex()
                ),
            )
            .bytes("result_ref", request.refund.as_bytes())
            .int("created_at_ms", request.now_ms),
    ));
    plan.entries.push(JournalEntry {
        entry_id: request.entry_id,
        kind: EntryKind::RefundRequested,
        business_key: business_key("refund_requested", &[request.refund.as_bytes()]),
        external_ref: None,
        stream: (StreamKind::Payer, *snapshot.payer.as_bytes()),
        effective_at_ms: request.now_ms,
        reason: EntryReason::RefundRequested,
        actor: Actor::System,
        correlation: Correlation::default(),
        lines,
    });
    plan.check().map_err(RefundRefusal::Plan)?;
    Ok(Some(plan))
}

/// One refund lot as the payment/observation flows read it.
#[derive(Clone, Debug)]
pub struct RefundLotSnapshot {
    pub payer: PayerId,
    pub currency: CurrencyTag,
    pub refund: RefundId,
    pub refund_state: String,
    pub refund_revision: i64,
    pub refund_lot: RefundLotId,
    pub lot: LotId,
    pub lot_state: LotState,
    pub lot_refunded_micro_dollars: u64,
    pub lot_revision: i64,
    pub principal_micro_dollars: u64,
    pub tax_micro_dollars: u64,
    pub tax_jurisdiction: Option<String>,
    pub processor: Option<String>,
    pub state: String,
    pub object_revision: i64,
    pub revision: i64,
    pub siblings_paid: bool,
    pub payment_ref: Option<String>,
    pub payment_refunded_total_micro_dollars: u64,
    pub payment_object_revision: i64,
    pub payment_revision: i64,
    pub outbox_attempts: i64,
    pub outbox_revision: i64,
}

#[derive(Clone, Debug)]
pub struct RefundPaidRequest {
    pub event_id: EventId,
    pub entry_id: EntryId,
    pub processor_refund_ref: String,
    pub now_ms: i64,
}

/// 16.9, on processor success: cash out of `refund_payable` against clearing, tax reversed
/// with it. `Ok(None)` when the lot is already paid: the processor key is the refund lot,
/// so a retry after a timeout posts this once (REF-02).
pub fn plan_refund_paid(
    snapshot: &RefundLotSnapshot,
    request: &RefundPaidRequest,
) -> Result<Option<FinancePlan>, RefundRefusal> {
    if snapshot.state == "paid" {
        return Ok(None);
    }
    if snapshot.state == "blocked_by_dispute" {
        return Err(RefundRefusal::BlockedByDispute);
    }
    if snapshot.state != "pending" && snapshot.state != "unknown" {
        return Err(RefundRefusal::RefundLotNotPayable);
    }
    let processor = snapshot
        .processor
        .as_deref()
        .ok_or(RefundRefusal::RefundLotNotPayable)?;
    let keys = Keys::new(snapshot.currency);
    let principal = snapshot.principal_micro_dollars;
    let mut lines = vec![
        Line::debit(keys.refund_payable(snapshot.lot), principal)?,
        Line::credit(
            keys.clearing(processor),
            principal
                .checked_add(snapshot.tax_micro_dollars)
                .ok_or(RefundRefusal::Arithmetic)?,
        )?,
    ];
    if snapshot.tax_micro_dollars > 0 {
        let jurisdiction = snapshot
            .tax_jurisdiction
            .as_deref()
            .ok_or(RefundRefusal::RefundLotNotPayable)?;
        lines.insert(
            1,
            Line::debit(keys.tax_payable(jurisdiction), snapshot.tax_micro_dollars)?,
        );
    }
    let mut plan = FinancePlan::new(request.event_id, EventKind::RefundPaid);
    if snapshot.tax_micro_dollars > 0 {
        let jurisdiction = snapshot.tax_jurisdiction.as_deref().unwrap_or_default();
        plan.changes
            .push(RowChange::key(&keys.tax_payable(jurisdiction)));
    }
    plan.changes.push(RowChange::update(
        ChangeKind::RefundLotUpdate,
        Row::new()
            .bytes("refund_lot_id", snapshot.refund_lot.as_bytes())
            .text("state", "paid")
            .text("processor_refund_ref", &request.processor_refund_ref)
            .int("object_revision", snapshot.object_revision + 1)
            .int("updated_at_ms", request.now_ms),
        snapshot.revision,
    ));
    plan.changes.push(RowChange::update(
        ChangeKind::RefundUpdate,
        Row::new()
            .bytes("refund_id", snapshot.refund.as_bytes())
            .text(
                "state",
                if snapshot.siblings_paid {
                    "paid"
                } else {
                    "partly_paid"
                },
            ),
        snapshot.refund_revision,
    ));
    plan.changes.push(RowChange::update(
        ChangeKind::FundingLotRefundUpdate,
        Row::new().bytes("lot_id", snapshot.lot.as_bytes()).uint(
            "refunded_micro_dollars",
            snapshot
                .lot_refunded_micro_dollars
                .checked_add(principal)
                .ok_or(RefundRefusal::Arithmetic)?,
        ),
        snapshot.lot_revision,
    ));
    if let Some(payment_ref) = &snapshot.payment_ref {
        plan.changes.push(RowChange::update(
            ChangeKind::ProcessorPaymentStateUpdate,
            Row::new()
                .text("processor", processor)
                .text("payment_ref", payment_ref)
                .uint(
                    "refunded_total_micro_dollars",
                    snapshot
                        .payment_refunded_total_micro_dollars
                        .checked_add(principal)
                        .ok_or(RefundRefusal::Arithmetic)?,
                )
                .int("object_revision", snapshot.payment_object_revision + 1)
                .int("applied_at_ms", request.now_ms),
            snapshot.payment_revision,
        ));
    }
    plan.entries.push(JournalEntry {
        entry_id: request.entry_id,
        kind: EntryKind::RefundPaid,
        business_key: business_key("refund_paid", &[snapshot.refund_lot.as_bytes()]),
        external_ref: Some((
            ExternalRefKind::ProcessorRefund,
            request.processor_refund_ref.clone(),
        )),
        stream: (StreamKind::Payer, *snapshot.payer.as_bytes()),
        effective_at_ms: request.now_ms,
        reason: EntryReason::RefundPaid,
        actor: Actor::System,
        correlation: Correlation::default(),
        lines,
    });
    plan.check().map_err(RefundRefusal::Plan)?;
    Ok(Some(plan))
}

/// Marks a refund lot unknown (a timed-out processor call) or failed in review.
/// No journal entry: no money moved (14.1, REF-02).
pub fn plan_refund_observed(
    snapshot: &RefundLotSnapshot,
    event_id: EventId,
    unknown: bool,
    now_ms: i64,
) -> Result<Option<FinancePlan>, RefundRefusal> {
    let (event_kind, state) = if unknown {
        (EventKind::RefundUnknown, "unknown")
    } else {
        (EventKind::RefundFailed, "failed_review")
    };
    if snapshot.state == state || snapshot.state == "paid" {
        return Ok(None);
    }
    let mut plan = FinancePlan::new(event_id, event_kind);
    plan.changes.push(RowChange::update(
        ChangeKind::RefundLotUpdate,
        Row::new()
            .bytes("refund_lot_id", snapshot.refund_lot.as_bytes())
            .text("state", state)
            .int("object_revision", snapshot.object_revision + 1)
            .int("updated_at_ms", now_ms),
        snapshot.revision,
    ));
    plan.changes.push(RowChange::update(
        ChangeKind::RefundOutboxUpdate,
        Row::new()
            .bytes("refund_lot_id", snapshot.refund_lot.as_bytes())
            .int("attempts", snapshot.outbox_attempts + 1)
            .int("next_attempt_at_ms", now_ms),
        snapshot.outbox_revision,
    ));
    Ok(Some(plan))
}

/// Returns a failed refund lot to spendable credit (14.1, 14.4).
pub fn plan_refund_returned(
    snapshot: &RefundLotSnapshot,
    event_id: EventId,
    entry_id: EntryId,
    now_ms: i64,
) -> Result<Option<FinancePlan>, RefundRefusal> {
    if snapshot.state == "returned_to_credit" {
        return Ok(None);
    }
    if snapshot.state != "failed_review" {
        return Err(RefundRefusal::RefundLotNotPayable);
    }
    let keys = Keys::new(snapshot.currency);
    let credit = match snapshot.lot_state {
        LotState::Active => keys.available(LotClass::Purchased, snapshot.payer, snapshot.lot),
        LotState::Frozen => keys.frozen(snapshot.lot),
        _ => return Err(RefundRefusal::RefundLotNotPayable),
    };
    let mut plan = FinancePlan::new(event_id, EventKind::RefundReturned);
    plan.changes.push(RowChange::update(
        ChangeKind::RefundLotUpdate,
        Row::new()
            .bytes("refund_lot_id", snapshot.refund_lot.as_bytes())
            .text("state", "returned_to_credit")
            .int("object_revision", snapshot.object_revision + 1)
            .int("updated_at_ms", now_ms),
        snapshot.revision,
    ));
    plan.changes.push(RowChange::update(
        ChangeKind::RefundUpdate,
        Row::new()
            .bytes("refund_id", snapshot.refund.as_bytes())
            .text("state", "returned"),
        snapshot.refund_revision,
    ));
    plan.entries.push(JournalEntry {
        entry_id,
        kind: EntryKind::RefundReturnedToCredit,
        business_key: business_key("refund_returned", &[snapshot.refund_lot.as_bytes()]),
        external_ref: None,
        stream: (StreamKind::Payer, *snapshot.payer.as_bytes()),
        effective_at_ms: now_ms,
        reason: EntryReason::RefundRequested,
        actor: Actor::System,
        correlation: Correlation::default(),
        lines: vec![
            Line::debit(
                keys.refund_payable(snapshot.lot),
                snapshot.principal_micro_dollars,
            )?,
            Line::credit(credit, snapshot.principal_micro_dollars)?,
        ],
    });
    plan.check().map_err(RefundRefusal::Plan)?;
    Ok(Some(plan))
}
