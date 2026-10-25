//! Settlement, release and customer credit (planning/finance-ledger-research.md 12.2, 12.5, 14.4,
//! 16.5, 16.6, 16.8): the pure decisions that turn the rows read under the flow's locks into a
//! plan. A charge is split over the held shares by largest remainder; promotional credit is spent
//! first inside a payer's share; anything that comes back goes where `return_target` says.

use std::collections::BTreeMap;

use crate::digest::business_key;
use crate::finance_ids::{
    AllocationId, EntryId, EventId, LotId, OperationKey, PayerId, UsageEventId,
};
use crate::funding::LotClass;
use crate::keys::Keys;
use crate::ledger::{
    AccountKey, Actor, Correlation, EntryKind, EntryReason, EntryRefusal, JournalEntry, Line,
    StreamKind,
};
use crate::plan::{ChangeKind, EventKind, FinancePlan, PlanRefusal, Row, RowChange};
use crate::pot::split_exactly;
use crate::price_schedule::{PriceVersion, UnitsRefusal};
use crate::returns::{AllocationStatus, ReturnTarget, credit_target, return_target};
use crate::snapshot::{
    AllocationRow, AttemptRow, DispatchState, LotRow, ReservationRow, SessionState, ShareRow,
    UsageRow,
};

/// Why a settlement, release or credit was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettleRefusal {
    /// The reservation is no longer held: another flow settled it first (JOB-02).
    Stale,
    /// The attempt was never sent, so there is nothing to charge.
    NotDispatched,
    /// A sent attempt cannot be released by a user cancel; only the hold deadline releases it.
    CannotReleaseSent,
    Units(UnitsRefusal),
    /// A share names a lot or allocation the snapshot does not hold.
    MissingRow,
    /// The lot state allows no such return (14.4).
    ReturnRefused,
    /// A credit to a charged-back or closed lot is refused (14.4).
    CreditTargetRefused,
    /// More than the charge still has reversible.
    OverReversal {
        remaining: u64,
        requested: u64,
    },
    NothingToCredit,
    Arithmetic,
    Entry(EntryRefusal),
    Plan(PlanRefusal),
}

impl From<EntryRefusal> for SettleRefusal {
    fn from(refusal: EntryRefusal) -> Self {
        Self::Entry(refusal)
    }
}

impl From<UnitsRefusal> for SettleRefusal {
    fn from(refusal: UnitsRefusal) -> Self {
        Self::Units(refusal)
    }
}

/// What a settlement or release read under its locks.
#[derive(Clone, Debug)]
pub struct HeldSnapshot {
    pub session: SessionState,
    pub reservation: ReservationRow,
    pub attempt: AttemptRow,
    pub shares: Vec<ShareRow>,
    pub allocations: Vec<AllocationRow>,
    pub lots: Vec<LotRow>,
    pub price_version: PriceVersion,
}

impl HeldSnapshot {
    fn lot(&self, lot: LotId) -> Result<&LotRow, SettleRefusal> {
        self.lots
            .iter()
            .find(|row| row.lot == lot)
            .ok_or(SettleRefusal::MissingRow)
    }

    fn allocation(&self, allocation: AllocationId) -> Result<&AllocationRow, SettleRefusal> {
        self.allocations
            .iter()
            .find(|row| row.allocation == allocation)
            .ok_or(SettleRefusal::MissingRow)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageSource {
    Reported,
    Estimated,
}

impl UsageSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Reported => "reported",
            Self::Estimated => "estimated",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SettleRequest {
    pub event_id: EventId,
    pub entry_id: EntryId,
    pub usage_event: UsageEventId,
    pub actual_units: Vec<(String, u64)>,
    pub usage_source: UsageSource,
    pub now_ms: i64,
}

/// Accumulates signed lines by key so one key appears once per entry.
#[derive(Default)]
struct Lines(BTreeMap<AccountKey, i64>);

impl Lines {
    fn debit(&mut self, key: AccountKey, amount: u64) -> Result<(), SettleRefusal> {
        self.add(
            key,
            i64::try_from(amount).map_err(|_| SettleRefusal::Arithmetic)?,
        )
    }

    fn credit(&mut self, key: AccountKey, amount: u64) -> Result<(), SettleRefusal> {
        self.add(
            key,
            -i64::try_from(amount).map_err(|_| SettleRefusal::Arithmetic)?,
        )
    }

    fn add(&mut self, key: AccountKey, signed: i64) -> Result<(), SettleRefusal> {
        if signed == 0 {
            return Ok(());
        }
        let slot = self.0.entry(key).or_insert(0);
        *slot = slot.checked_add(signed).ok_or(SettleRefusal::Arithmetic)?;
        Ok(())
    }

    fn finish(self) -> Vec<Line> {
        self.0
            .into_iter()
            .filter(|(_, amount)| *amount != 0)
            .map(|(key, amount_micro_dollars)| Line {
                key,
                amount_micro_dollars,
            })
            .collect()
    }
}

/// The money a share took and gave back, by key.
struct Resolved {
    charged: u64,
    released: u64,
}

fn post_return(
    lines: &mut Lines,
    keys: &Keys,
    snapshot: &HeldSnapshot,
    share: &ShareRow,
    released: u64,
) -> Result<ReturnTarget, SettleRefusal> {
    if released == 0 {
        return Ok(ReturnTarget::Allocated);
    }
    let lot = snapshot.lot(share.lot)?;
    let allocation = snapshot.allocation(share.allocation)?;
    let target = return_target(share.class, lot.state, allocation.status);
    match target {
        ReturnTarget::Allocated => lines.credit(
            keys.allocated(share.class, share.allocation, share.lot),
            released,
        )?,
        ReturnTarget::Available => lines.credit(
            keys.available(share.class, allocation.payer, share.lot),
            released,
        )?,
        ReturnTarget::Frozen => lines.credit(keys.frozen(share.lot), released)?,
        ReturnTarget::ChargebackRecovery => {
            let processor = lot.processor.as_deref().ok_or(SettleRefusal::MissingRow)?;
            lines.credit(keys.chargeback_loss(processor), released)?;
        }
        ReturnTarget::Refuse => return Err(SettleRefusal::ReturnRefused),
    }
    Ok(target)
}

/// Whether a flow charged the reservation (a settlement, even of a zero charge) or only released it.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Closing {
    Settled,
    Released,
}

/// Writes the share, reservation, allocation and lot rows that follow from what each share
/// charged and released.
fn share_changes(
    plan: &mut FinancePlan,
    snapshot: &HeldSnapshot,
    resolved: &[(usize, Resolved, ReturnTarget)],
    closing: Closing,
) -> Result<(), SettleRefusal> {
    let mut lot_consumed: BTreeMap<LotId, (u64, u64)> = BTreeMap::new();
    let mut allocation_totals: BTreeMap<AllocationId, (u64, u64)> = BTreeMap::new();
    let (mut charged_total, mut released_total) = (0_u64, 0_u64);
    for (index, outcome, target) in resolved {
        let share = &snapshot.shares[*index];
        plan.changes.push(RowChange::update(
            ChangeKind::ReservationShareUpdate,
            Row::new()
                .bytes("reservation_id", share.reservation.as_bytes())
                .bytes("allocation_id", share.allocation.as_bytes())
                .bytes("lot_id", share.lot.as_bytes())
                .uint("charged_micro_dollars", outcome.charged)
                .uint("released_micro_dollars", outcome.released),
            share.revision,
        ));
        charged_total += outcome.charged;
        released_total += outcome.released;
        let lot_entry = lot_consumed.entry(share.lot).or_default();
        lot_entry.0 += outcome.charged;
        if *target == ReturnTarget::ChargebackRecovery {
            lot_entry.1 += outcome.released;
        }
        let allocation_entry = allocation_totals.entry(share.allocation).or_default();
        allocation_entry.0 += outcome.charged;
        if *target != ReturnTarget::Allocated {
            allocation_entry.1 += outcome.released;
        }
    }
    plan.changes.push(RowChange::update(
        ChangeKind::ReservationUpdate,
        Row::new()
            .bytes(
                "reservation_id",
                snapshot.reservation.reservation.as_bytes(),
            )
            .text(
                "state",
                if closing == Closing::Settled {
                    "charged"
                } else {
                    "released"
                },
            )
            .uint("charged_micro_dollars", charged_total)
            .uint("released_micro_dollars", released_total),
        snapshot.reservation.revision,
    ));
    for (lot, (consumed, extinguished)) in lot_consumed {
        if consumed == 0 && extinguished == 0 {
            continue;
        }
        let row = snapshot.lot(lot)?;
        plan.changes.push(RowChange::update(
            ChangeKind::FundingLotUpdate,
            Row::new()
                .bytes("lot_id", lot.as_bytes())
                .uint(
                    "consumed_micro_dollars",
                    row.consumed_micro_dollars + consumed,
                )
                .uint(
                    "extinguished_micro_dollars",
                    row.extinguished_micro_dollars + extinguished,
                ),
            row.revision,
        ));
    }
    for (allocation, (charged, returned)) in allocation_totals {
        if charged == 0 && returned == 0 {
            continue;
        }
        let row = snapshot.allocation(allocation)?;
        plan.changes.push(RowChange::update(
            ChangeKind::AllocationUpdate,
            Row::new()
                .bytes("allocation_id", allocation.as_bytes())
                .uint("charged_micro_dollars", row.charged_micro_dollars + charged)
                .uint(
                    "returned_micro_dollars",
                    row.returned_micro_dollars + returned,
                ),
            row.revision,
        ));
    }
    Ok(())
}

fn complete_timers(plan: &mut FinancePlan, snapshot: &HeldSnapshot) {
    for kind in [
        "reserved_expiry",
        "prepared_expiry",
        "sent_reconcile",
        "customer_hold",
        "unknown_resolution",
    ] {
        plan.changes.push(RowChange::insert(
            ChangeKind::WorkItemComplete,
            Row::new()
                .text("kind", kind)
                .bytes("ref_id", snapshot.attempt.attempt.as_bytes()),
        ));
    }
}

fn correlation(snapshot: &HeldSnapshot) -> Correlation {
    Correlation {
        campaign: snapshot.attempt.campaign,
        session: Some(snapshot.session.session),
        attempt: Some(snapshot.attempt.attempt),
        reservation: Some(snapshot.reservation.reservation),
        price_version: Some(snapshot.reservation.price_version),
    }
}

/// The charge over the held shares: over payers by largest remainder of their held amounts; inside
/// a payer, promotional credit is spent first (decision 3). One amount per share, in share order.
fn charge_shares(snapshot: &HeldSnapshot, charge: u64) -> Result<Vec<u64>, SettleRefusal> {
    let mut charged = vec![0_u64; snapshot.shares.len()];
    if charge == 0 {
        return Ok(charged);
    }
    let mut by_allocation: BTreeMap<AllocationId, Vec<usize>> = BTreeMap::new();
    for (index, share) in snapshot.shares.iter().enumerate() {
        by_allocation
            .entry(share.allocation)
            .or_default()
            .push(index);
    }
    let weights: Vec<(AllocationId, u64)> = by_allocation
        .iter()
        .map(|(allocation, indexes)| {
            (
                *allocation,
                indexes
                    .iter()
                    .map(|&i| snapshot.shares[i].held_micro_dollars)
                    .sum(),
            )
        })
        .collect();
    let per_payer = split_exactly(charge, &weights).map_err(|_| SettleRefusal::Arithmetic)?;
    for (allocation, payer_charge) in per_payer {
        let mut indexes = by_allocation.get(&allocation).cloned().unwrap_or_default();
        indexes.sort_by_key(|&i| {
            (
                snapshot.shares[i].class == LotClass::Purchased,
                snapshot.shares[i].lot,
            )
        });
        let mut remaining = payer_charge;
        for index in indexes {
            let take = remaining.min(snapshot.shares[index].held_micro_dollars);
            charged[index] = take;
            remaining -= take;
        }
    }
    Ok(charged)
}

/// What charging the shares posts, and the split of the charge by lot class.
struct ShareLines {
    lines: Lines,
    resolved: Vec<(usize, Resolved, ReturnTarget)>,
    purchased_charge: u64,
    promotional_charge: u64,
}

/// For each share: the reserved debit, the revenue or offset credit of what it was charged, and the
/// return of what it was not.
fn charge_lines(
    snapshot: &HeldSnapshot,
    keys: &Keys,
    charged: &[u64],
) -> Result<ShareLines, SettleRefusal> {
    let mut out = ShareLines {
        lines: Lines::default(),
        resolved: Vec::new(),
        purchased_charge: 0,
        promotional_charge: 0,
    };
    for (index, share) in snapshot.shares.iter().enumerate() {
        let taken = charged[index];
        let released = share.held_micro_dollars - taken;
        out.lines.debit(
            keys.reserved(share.class, share.allocation, share.lot),
            share.held_micro_dollars,
        )?;
        match share.class {
            LotClass::Purchased => {
                out.lines.credit(keys.revenue_ai_usage(), taken)?;
                out.purchased_charge += taken;
            }
            LotClass::Promotional => {
                let source = snapshot
                    .lot(share.lot)?
                    .promo_source
                    .as_deref()
                    .ok_or(SettleRefusal::MissingRow)?;
                out.lines.credit(keys.promotional_offset(source), taken)?;
                out.promotional_charge += taken;
            }
        }
        let target = post_return(&mut out.lines, keys, snapshot, share, released)?;
        out.resolved.push((
            index,
            Resolved {
                charged: taken,
                released,
            },
            target,
        ));
    }
    Ok(out)
}

/// The supplier cost, split between the purchased and promotional cost lines by largest remainder
/// over the charge's parts (rule 4.6), ties to purchased. A zero charge books it all as purchased.
fn split_supplier_cost(
    supplier_cost: u64,
    purchased_charge: u64,
    promotional_charge: u64,
) -> Result<(u64, u64), SettleRefusal> {
    if promotional_charge == 0 {
        return Ok((supplier_cost, 0));
    }
    if purchased_charge == 0 {
        return Ok((0, supplier_cost));
    }
    let split = split_exactly(
        supplier_cost,
        &[(0_u8, purchased_charge), (1, promotional_charge)],
    )
    .map_err(|_| SettleRefusal::Arithmetic)?;
    Ok((split[0].1, split[1].1))
}

/// The usage units as a JSON object; a category is a tariff key, escaped all the same.
fn units_json(units: &[(String, u64)]) -> String {
    let members: Vec<String> = units
        .iter()
        .map(|(category, quantity)| {
            let escaped = category.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\":{quantity}")
        })
        .collect();
    format!("{{{}}}", members.join(","))
}

/// The amounts a settlement posts.
struct Charge {
    retail: u64,
    supplier: u64,
    supplier_purchased: u64,
    supplier_promotional: u64,
}

/// The usage event, the shares, the attempt, the settlement fact and the session's running total.
fn settlement_rows(
    plan: &mut FinancePlan,
    snapshot: &HeldSnapshot,
    request: &SettleRequest,
    charge: &Charge,
    resolved: &[(usize, Resolved, ReturnTarget)],
) -> Result<(), SettleRefusal> {
    plan.changes.push(RowChange::insert(
        ChangeKind::UsageEventInsert,
        Row::new()
            .bytes("usage_event_id", request.usage_event.as_bytes())
            .bytes("attempt_id", snapshot.attempt.attempt.as_bytes())
            .bytes("session_id", snapshot.session.session.as_bytes())
            .text("category", "ai_usage")
            .text("quantity_text", "usage")
            .json("units", &units_json(&request.actual_units))
            .uint("retail_micro_dollars", charge.retail)
            .uint("supplier_micro_dollars", charge.supplier)
            .uint(
                "supplier_purchased_micro_dollars",
                charge.supplier_purchased,
            )
            .uint(
                "supplier_promotional_micro_dollars",
                charge.supplier_promotional,
            )
            .bytes("price_version_id", snapshot.price_version.id.as_bytes())
            .bytes("entry_id", request.entry_id.as_bytes())
            .int("settled_at_ms", request.now_ms),
    ));
    share_changes(plan, snapshot, resolved, Closing::Settled)?;
    if charge.retail > 0 {
        plan.changes.push(RowChange::update(
            ChangeKind::SessionChargedTotalUpdate,
            Row::new()
                .bytes("session_id", snapshot.session.session.as_bytes())
                .uint(
                    "charged_total_micro_dollars",
                    snapshot
                        .session
                        .charged_total_micro_dollars
                        .checked_add(charge.retail)
                        .ok_or(SettleRefusal::Arithmetic)?,
                ),
            snapshot.session.revision,
        ));
    }
    let mut attempt = Row::new()
        .bytes("attempt_id", snapshot.attempt.attempt.as_bytes())
        .text("customer_state", "charged")
        .text("supplier_state", "cost_recorded")
        .text("supplier_outcome", "billed")
        .text("usage_source", request.usage_source.label())
        .uint("supplier_cost_micro_dollars", charge.supplier)
        .text("usability", "usable")
        .int("settled_at_ms", request.now_ms);
    if snapshot.attempt.dispatch != DispatchState::ResultKnown {
        attempt = attempt
            .text("dispatch_state", "result_known")
            .int("result_at_ms", request.now_ms);
    }
    plan.changes.push(RowChange::update(
        ChangeKind::ProviderAttemptUpdate,
        attempt,
        snapshot.attempt.revision,
    ));
    plan.changes.push(RowChange::insert(
        ChangeKind::SettlementFactInsert,
        Row::new()
            .bytes("attempt_id", snapshot.attempt.attempt.as_bytes())
            .text("source", "runtime")
            .text("outcome", "usable")
            .int("recorded_at_ms", request.now_ms),
    ));
    complete_timers(plan, snapshot);
    Ok(())
}

/// 16.5: charges the actual units against the held shares and releases the surplus. A billed result
/// whose retail rounds to zero settles with no revenue line: the hold is released in full and the
/// supplier cost is booked, never left stranded.
pub fn plan_settlement(
    snapshot: &HeldSnapshot,
    request: &SettleRequest,
) -> Result<FinancePlan, SettleRefusal> {
    if !snapshot.reservation.held {
        return Err(SettleRefusal::Stale);
    }
    if snapshot.attempt.dispatch.is_unsent()
        || snapshot.attempt.dispatch == DispatchState::FailedUnknown
    {
        return Err(SettleRefusal::NotDispatched);
    }
    let version = &snapshot.price_version;
    let (provider, model) = (
        snapshot.attempt.provider.as_str(),
        snapshot.attempt.model.as_str(),
    );
    let retail = version.retail_charge(
        provider,
        model,
        &request.actual_units,
        snapshot.reservation.retail_maximum_micro_dollars,
    )?;
    let supplier = version.supplier_cost(provider, model, &request.actual_units)?;

    let keys = Keys::new(snapshot.session.currency);
    let charged = charge_shares(snapshot, retail)?;
    let mut posted = charge_lines(snapshot, &keys, &charged)?;
    let (supplier_purchased, supplier_promotional) =
        split_supplier_cost(supplier, posted.purchased_charge, posted.promotional_charge)?;
    posted
        .lines
        .debit(keys.cost_of_services(provider, false), supplier_purchased)?;
    posted
        .lines
        .debit(keys.cost_of_services(provider, true), supplier_promotional)?;
    posted
        .lines
        .credit(keys.supplier_payable(provider), supplier)?;
    let exposure = snapshot.attempt.supplier_exposure_micro_dollars;
    posted
        .lines
        .debit(keys.exposure_offset(provider), exposure)?;
    posted
        .lines
        .credit(keys.exposure_open(provider), exposure)?;

    let mut plan = FinancePlan::new(request.event_id, EventKind::SettlementApplied);
    let charge = Charge {
        retail,
        supplier,
        supplier_purchased,
        supplier_promotional,
    };
    settlement_rows(&mut plan, snapshot, request, &charge, &posted.resolved)?;
    plan.entries.push(JournalEntry {
        entry_id: request.entry_id,
        kind: EntryKind::Settlement,
        business_key: business_key("settlement", &[snapshot.attempt.attempt.as_bytes()]),
        external_ref: None,
        stream: (StreamKind::Session, *snapshot.session.session.as_bytes()),
        effective_at_ms: request.now_ms,
        reason: EntryReason::Settled,
        actor: Actor::System,
        correlation: correlation(snapshot),
        lines: posted.lines.finish(),
    });
    plan.check().map_err(SettleRefusal::Plan)?;
    Ok(plan)
}

/// Why a hold is released.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseReason {
    /// The user cancelled, or nothing was dispatched: the hold and the exposure both close.
    NotDispatched,
    /// The customer-hold deadline passed with the supplier outcome unknown: the exposure stays.
    HoldDeadline,
}

#[derive(Clone, Copy, Debug)]
pub struct ReleaseRequest {
    pub event_id: EventId,
    pub entry_id: EntryId,
    pub reason: ReleaseReason,
    pub now_ms: i64,
}

/// The attempt and outcome rows of a release; the exposure closes only for work never sent.
fn release_attempt_rows(
    plan: &mut FinancePlan,
    snapshot: &HeldSnapshot,
    request: &ReleaseRequest,
    close_exposure: bool,
) {
    let mut attempt = Row::new()
        .bytes("attempt_id", snapshot.attempt.attempt.as_bytes())
        .text("customer_state", "released")
        .int("customer_released_at_ms", request.now_ms);
    if close_exposure {
        attempt = attempt
            .text("supplier_state", "exposure_released")
            .text("supplier_outcome", "not_billed");
        if matches!(
            snapshot.attempt.dispatch,
            DispatchState::Reserved | DispatchState::DispatchPrepared
        ) {
            attempt = attempt.text("dispatch_state", "cancelled_unsent");
        }
    }
    plan.changes.push(RowChange::update(
        ChangeKind::ProviderAttemptUpdate,
        attempt,
        snapshot.attempt.revision,
    ));
    if close_exposure && let Some(asset) = snapshot.attempt.media_asset {
        plan.changes.push(RowChange::insert(
            ChangeKind::AttemptOutcomeEventInsert,
            Row::new()
                .bytes("attempt_id", snapshot.attempt.attempt.as_bytes())
                .bytes("media_asset_id", &asset)
                .text("outcome", "cancelled_unsent")
                .text("reason", "not_dispatched")
                .int("recorded_at_ms", request.now_ms),
        ));
    }
}

/// 16.6: releases every held share to where `return_target` says.
pub fn plan_release(
    snapshot: &HeldSnapshot,
    request: &ReleaseRequest,
) -> Result<FinancePlan, SettleRefusal> {
    if !snapshot.reservation.held {
        return Err(SettleRefusal::Stale);
    }
    let unsent = snapshot.attempt.dispatch.is_unsent();
    match request.reason {
        ReleaseReason::NotDispatched if !unsent => return Err(SettleRefusal::CannotReleaseSent),
        ReleaseReason::HoldDeadline if unsent => return Err(SettleRefusal::NotDispatched),
        _ => {}
    }
    let keys = Keys::new(snapshot.session.currency);
    let provider = snapshot.attempt.provider.as_str();
    let mut lines = Lines::default();
    let mut resolved = Vec::new();
    for (index, share) in snapshot.shares.iter().enumerate() {
        lines.debit(
            keys.reserved(share.class, share.allocation, share.lot),
            share.held_micro_dollars,
        )?;
        let target = post_return(&mut lines, &keys, snapshot, share, share.held_micro_dollars)?;
        resolved.push((
            index,
            Resolved {
                charged: 0,
                released: share.held_micro_dollars,
            },
            target,
        ));
    }
    let close_exposure = request.reason == ReleaseReason::NotDispatched;
    if close_exposure {
        let exposure = snapshot.attempt.supplier_exposure_micro_dollars;
        lines.debit(keys.exposure_offset(provider), exposure)?;
        lines.credit(keys.exposure_open(provider), exposure)?;
    }
    let mut plan = FinancePlan::new(request.event_id, EventKind::ReleaseApplied);
    share_changes(&mut plan, snapshot, &resolved, Closing::Released)?;
    release_attempt_rows(&mut plan, snapshot, request, close_exposure);
    complete_timers(&mut plan, snapshot);
    plan.entries.push(JournalEntry {
        entry_id: request.entry_id,
        kind: EntryKind::Release,
        business_key: business_key(
            "release",
            &[
                snapshot.attempt.attempt.as_bytes(),
                if close_exposure { b"unsent" } else { b"hold" },
            ],
        ),
        external_ref: None,
        stream: (StreamKind::Session, *snapshot.session.session.as_bytes()),
        effective_at_ms: request.now_ms,
        reason: if close_exposure {
            EntryReason::ReleasedNotDispatched
        } else {
            EntryReason::ReleasedHoldDeadline
        },
        actor: Actor::System,
        correlation: correlation(snapshot),
        lines: lines.finish(),
    });
    plan.check().map_err(SettleRefusal::Plan)?;
    Ok(plan)
}

/// A charged share with its payer and allocation status, for credits.
#[derive(Clone, Debug)]
pub struct CreditShare {
    pub share: ShareRow,
    pub payer: PayerId,
    pub allocation_status: AllocationStatus,
}

#[derive(Clone, Debug)]
pub struct CreditSnapshot {
    pub session: SessionState,
    pub usage: UsageRow,
    pub attempt: AttemptRow,
    pub shares: Vec<CreditShare>,
    pub lots: Vec<LotRow>,
}

/// Why an operator credits a charge; each is a closed code in the sealed journal (22.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditReason {
    TechnicalFailure,
    ContentFailure,
    BillingError,
    DuplicateCharge,
    Goodwill,
}

impl CreditReason {
    pub fn entry_reason(self) -> EntryReason {
        match self {
            Self::TechnicalFailure => EntryReason::CreditTechnicalFailure,
            Self::ContentFailure => EntryReason::CreditContentFailure,
            Self::BillingError => EntryReason::CreditBillingError,
            Self::DuplicateCharge => EntryReason::CreditDuplicateCharge,
            Self::Goodwill => EntryReason::CreditGoodwill,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CreditRequest {
    pub event_id: EventId,
    pub entry_id: EntryId,
    pub operation: OperationKey,
    pub amount_micro_dollars: u64,
    pub reason: CreditReason,
    pub actor_id: [u8; 16],
    pub case_ref: String,
    pub approver_id: Option<[u8; 16]>,
    pub now_ms: i64,
}

/// One share's part of a credit, keyed by its allocation and lot.
type CreditPart = ((AllocationId, LotId), u64);

/// The credit over the charged shares by what each can still give back.
fn credit_parts(
    snapshot: &CreditSnapshot,
    amount: u64,
    remaining: u64,
) -> Result<Vec<CreditPart>, SettleRefusal> {
    let weights: Vec<((AllocationId, LotId), u64)> = snapshot
        .shares
        .iter()
        .map(|entry| {
            (
                (entry.share.allocation, entry.share.lot),
                entry
                    .share
                    .charged_micro_dollars
                    .saturating_sub(entry.share.reversed_micro_dollars),
            )
        })
        .collect();
    split_exactly(amount, &weights).map_err(|_| SettleRefusal::OverReversal {
        remaining,
        requested: amount,
    })
}

/// Debits the revenue or offset and credits the original lot's key, for each share's part.
fn credit_lines(
    snapshot: &CreditSnapshot,
    parts: &[CreditPart],
    plan: &mut FinancePlan,
) -> Result<Lines, SettleRefusal> {
    let keys = Keys::new(snapshot.session.currency);
    let mut lines = Lines::default();
    let mut lot_credit: BTreeMap<LotId, u64> = BTreeMap::new();
    for entry in &snapshot.shares {
        let part = parts
            .iter()
            .find(|((allocation, lot), _)| {
                *allocation == entry.share.allocation && *lot == entry.share.lot
            })
            .map_or(0, |(_, part)| *part);
        if part == 0 {
            continue;
        }
        let lot = snapshot
            .lots
            .iter()
            .find(|row| row.lot == entry.share.lot)
            .ok_or(SettleRefusal::MissingRow)?;
        match entry.share.class {
            LotClass::Purchased => lines.debit(keys.revenue_ai_usage(), part)?,
            LotClass::Promotional => {
                let source = lot
                    .promo_source
                    .as_deref()
                    .ok_or(SettleRefusal::MissingRow)?;
                lines.debit(keys.promotional_offset(source), part)?;
            }
        }
        match credit_target(entry.share.class, lot.state) {
            ReturnTarget::Available => lines.credit(
                keys.available(entry.share.class, entry.payer, entry.share.lot),
                part,
            )?,
            ReturnTarget::Frozen => lines.credit(keys.frozen(entry.share.lot), part)?,
            _ => return Err(SettleRefusal::CreditTargetRefused),
        }
        plan.changes.push(RowChange::update(
            ChangeKind::ReservationShareUpdate,
            Row::new()
                .bytes("reservation_id", entry.share.reservation.as_bytes())
                .bytes("allocation_id", entry.share.allocation.as_bytes())
                .bytes("lot_id", entry.share.lot.as_bytes())
                .uint(
                    "reversed_micro_dollars",
                    entry.share.reversed_micro_dollars + part,
                ),
            entry.share.revision,
        ));
        *lot_credit.entry(entry.share.lot).or_default() += part;
    }
    for (lot, credited) in lot_credit {
        let row = snapshot
            .lots
            .iter()
            .find(|row| row.lot == lot)
            .ok_or(SettleRefusal::MissingRow)?;
        plan.changes.push(RowChange::update(
            ChangeKind::FundingLotUpdate,
            Row::new().bytes("lot_id", lot.as_bytes()).uint(
                "consumed_micro_dollars",
                row.consumed_micro_dollars.saturating_sub(credited),
            ),
            row.revision,
        ));
    }
    Ok(lines)
}

/// 12.5, 16.8: returns up to the remaining reversible amount of a charge to its original lots,
/// split over the shares by what each can still give back. Goodwill is a separate flow.
pub fn plan_credit(
    snapshot: &CreditSnapshot,
    request: &CreditRequest,
) -> Result<FinancePlan, SettleRefusal> {
    if request.amount_micro_dollars == 0 {
        return Err(SettleRefusal::NothingToCredit);
    }
    let remaining = snapshot
        .usage
        .retail_micro_dollars
        .saturating_sub(snapshot.usage.reversed_micro_dollars);
    if request.amount_micro_dollars > remaining {
        return Err(SettleRefusal::OverReversal {
            remaining,
            requested: request.amount_micro_dollars,
        });
    }
    let parts = credit_parts(snapshot, request.amount_micro_dollars, remaining)?;
    let mut plan = FinancePlan::new(request.event_id, EventKind::CreditApplied);
    let lines = credit_lines(snapshot, &parts, &mut plan)?;
    let reason = request.reason.entry_reason();
    plan.changes.push(RowChange::update(
        ChangeKind::UsageEventUpdate,
        Row::new()
            .bytes("usage_event_id", snapshot.usage.usage_event.as_bytes())
            .uint(
                "reversed_micro_dollars",
                snapshot.usage.reversed_micro_dollars + request.amount_micro_dollars,
            ),
        snapshot.usage.revision,
    ));
    plan.changes.push(RowChange::insert(
        ChangeKind::UsageReversalInsert,
        Row::new()
            .bytes("usage_event_id", snapshot.usage.usage_event.as_bytes())
            .bytes("operation_id", request.operation.as_bytes())
            .uint("micro_dollars", request.amount_micro_dollars)
            .text("reason", reason.label())
            .bytes("entry_id", request.entry_id.as_bytes()),
    ));
    plan.entries.push(JournalEntry {
        entry_id: request.entry_id,
        kind: EntryKind::CustomerCredit,
        business_key: business_key(
            "customer_credit",
            &[
                snapshot.usage.usage_event.as_bytes(),
                request.operation.as_bytes(),
            ],
        ),
        external_ref: None,
        stream: (StreamKind::Session, *snapshot.session.session.as_bytes()),
        effective_at_ms: request.now_ms,
        reason,
        actor: Actor::Operator {
            actor_id: request.actor_id,
            case_ref: request.case_ref.clone(),
            approver_id: request.approver_id,
        },
        correlation: Correlation {
            campaign: snapshot.attempt.campaign,
            session: Some(snapshot.session.session),
            attempt: Some(snapshot.attempt.attempt),
            reservation: Some(snapshot.attempt.reservation),
            price_version: Some(snapshot.usage.price_version),
        },
        lines: lines.finish(),
    });
    plan.check().map_err(SettleRefusal::Plan)?;
    Ok(plan)
}
