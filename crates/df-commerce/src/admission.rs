//! Paid admission (planning/finance-ledger-research.md 11.2, 9.3, 10, 16.4): the pure decision
//! that turns a request and the rows read under the admission locks into a reservation, its
//! shares, a provider attempt and the journal entry that holds the money.
//!
//! The decision refuses before it writes: an unpriced category, an incomplete maximum, a stale or
//! spent quote, a ceiling, an inactive payer. Refusals are never stored (11.2 step 2); only an
//! admitted operation has a receipt.

use std::collections::BTreeSet;

use crate::digest::business_key;
use crate::finance_ids::{
    AllocationId, AttemptId, CampaignKey, EffectId, EntryId, EventId, LotId, OperationKey, PayerId,
    QuoteId, ReservationId, SessionKey, TenantKey, hex,
};
use crate::funding::{DrawableLot, LotClass, LotState, plan_draw};
use crate::keys::Keys;
use crate::ledger::{
    Actor, Correlation, EntryKind, EntryReason, EntryRefusal, JournalEntry, Line, StreamKind,
};
use crate::plan::{ChangeKind, EventKind, FinancePlan, PlanRefusal, Row, RowChange};
use crate::pot::split_capped;
use crate::price_schedule::{PriceVersion, UnitsRefusal, units_cover_route};
use crate::returns::AllocationStatus;
use crate::snapshot::{AllocationState, QuoteRow, QuoteStatus, ReceiptRow, SessionState};

/// How a route's cost reaches the book (15.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Metering {
    Priced,
    /// Metered but not priced from the table: only company-paid work may use it.
    Unpriced,
    /// Not metered yet: refused in hosted paid mode (ruling (e)).
    Pending,
}

/// A provider route as the admission sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Route {
    pub provider: String,
    pub model: String,
    pub name: String,
    /// The categories the route declares; the maximum units must cover exactly these.
    pub declared_categories: Vec<String>,
    pub metering: Metering,
}

/// Essential work bypasses pacing; optional work may be deferred; premium needs a quote.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkClass {
    Essential,
    Optional,
    Premium,
}

impl WorkClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Essential => "essential",
            Self::Optional => "optional",
            Self::Premium => "premium",
        }
    }
}

/// An admission request from the runtime's finance endpoint.
#[derive(Clone, Debug)]
pub struct AdmissionRequest {
    pub event_id: EventId,
    pub entry_id: EntryId,
    pub reservation: ReservationId,
    pub attempt: AttemptId,
    pub effect: EffectId,
    pub operation: OperationKey,
    pub tenant: TenantKey,
    pub campaign: CampaignKey,
    pub session: SessionKey,
    pub route: Route,
    pub work_class: WorkClass,
    pub maximum_units: Vec<(String, u64)>,
    pub estimate_units: Vec<(String, u64)>,
    pub quote: Option<QuoteId>,
    pub request_digest: [u8; 32],
    /// The payers whose fresh `member_snapshot` is authorized (11.2 step 4); others are excluded
    /// from the shares.
    pub authorized_payers: BTreeSet<PayerId>,
    pub now_ms: i64,
    pub reserved_expiry_ms: i64,
}

/// What the admission read under its locks.
#[derive(Clone, Debug)]
pub struct AdmissionSnapshot {
    pub session: SessionState,
    pub price_version: PriceVersion,
    /// The allocations of the current epoch with their epoch weights.
    pub allocations: Vec<(AllocationState, u64)>,
    pub quote: Option<QuoteRow>,
    pub open_holds_micro_dollars: u64,
    pub charged_last_hour_micro_dollars: u64,
    pub receipt: Option<ReceiptRow>,
}

/// The ceilings of 10 that can be reached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ceiling {
    Hourly,
    Session,
}

/// Why an admission was refused. None of these holds anything or writes anything.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdmissionRefusal {
    SessionNotOpen,
    PriceVersionMismatch,
    RouteUnmetered,
    CategoryUnpriced(String),
    MaximumUnitsIncomplete(String),
    UndeclaredCategory(String),
    QuoteRequired,
    QuoteExpired,
    QuoteNotApproved,
    QuoteAlreadyUsed,
    QuoteMismatch,
    /// No eligible payer can hold the maximum.
    FundingRequired,
    CeilingReached(Ceiling),
    /// The same operation id with a different request.
    Conflict,
    NothingToAdmit,
    Arithmetic,
    Entry(EntryRefusal),
    Plan(PlanRefusal),
}

impl AdmissionRefusal {
    /// A later retry can differ (11.2): the caller may try again.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::FundingRequired | Self::CeilingReached(_) | Self::QuoteNotApproved
        )
    }
}

impl From<UnitsRefusal> for AdmissionRefusal {
    fn from(refusal: UnitsRefusal) -> Self {
        match refusal {
            UnitsRefusal::CategoryUnpriced(category) => Self::CategoryUnpriced(category),
            UnitsRefusal::MaximumUnitsIncomplete(category) => {
                Self::MaximumUnitsIncomplete(category)
            }
            UnitsRefusal::UndeclaredCategory(category) => Self::UndeclaredCategory(category),
            UnitsRefusal::Arithmetic(_) => Self::Arithmetic,
        }
    }
}

/// An admitted operation: the plan and what the caller is told.
#[derive(Clone, Debug)]
pub struct Admitted {
    pub plan: FinancePlan,
    pub receipt: ReceiptRow,
}

#[derive(Clone, Debug)]
pub enum AdmissionDecision {
    Admitted(Box<Admitted>),
    /// The operation was admitted before: the stored outcome (SAVE-02).
    AlreadyAdmitted(ReceiptRow),
}

/// Units as unambiguous bytes: sorted by category, each category length-prefixed.
fn encode_units(units: &[(String, u64)]) -> Vec<u8> {
    let mut sorted: Vec<&(String, u64)> = units.iter().collect();
    sorted.sort();
    let mut out = Vec::new();
    for (category, quantity) in sorted {
        out.extend_from_slice(
            &u32::try_from(category.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        out.extend_from_slice(category.as_bytes());
        out.extend_from_slice(&quantity.to_be_bytes());
    }
    out
}

/// The fingerprint of a request (11.2 step 2): it includes the attempt id.
pub fn fingerprint(request: &AdmissionRequest) -> [u8; 32] {
    let quote: &[u8] = request.quote.as_ref().map_or(&[], |id| id.as_bytes());
    business_key(
        "admission_fingerprint",
        &[
            request.attempt.as_bytes(),
            request.effect.as_bytes(),
            request.session.as_bytes(),
            request.route.provider.as_bytes(),
            request.route.model.as_bytes(),
            request.route.name.as_bytes(),
            request.work_class.label().as_bytes(),
            &encode_units(&request.maximum_units),
            &encode_units(&request.estimate_units),
            quote,
            &request.request_digest,
        ],
    )
}

fn check_quote(
    request: &AdmissionRequest,
    snapshot: &AdmissionSnapshot,
    retail_maximum: u64,
) -> Result<(), AdmissionRefusal> {
    let needed = retail_maximum > snapshot.session.per_request_threshold_micro_dollars
        || request.work_class == WorkClass::Premium;
    if request.quote.is_none() {
        return if needed {
            Err(AdmissionRefusal::QuoteRequired)
        } else {
            Ok(())
        };
    }
    let Some(quote) = &snapshot.quote else {
        return Err(AdmissionRefusal::QuoteMismatch);
    };
    if Some(quote.quote) != request.quote || quote.session != request.session {
        return Err(AdmissionRefusal::QuoteMismatch);
    }
    match quote.status {
        QuoteStatus::Used => return Err(AdmissionRefusal::QuoteAlreadyUsed),
        QuoteStatus::Approved => {}
        QuoteStatus::Open | QuoteStatus::Declined => {
            return Err(AdmissionRefusal::QuoteNotApproved);
        }
        QuoteStatus::Expired => return Err(AdmissionRefusal::QuoteExpired),
    }
    if request.now_ms >= quote.expires_at_ms {
        return Err(AdmissionRefusal::QuoteExpired);
    }
    if quote.request_digest != request.request_digest
        || quote.price_version != snapshot.session.price_version
        || quote.retail_maximum_micro_dollars < retail_maximum
    {
        return Err(AdmissionRefusal::QuoteMismatch);
    }
    Ok(())
}

/// The most the request can cost the table and the supplier.
struct Maximums {
    retail: u64,
    supplier: u64,
}

fn price_request(
    snapshot: &AdmissionSnapshot,
    request: &AdmissionRequest,
) -> Result<Maximums, AdmissionRefusal> {
    if !snapshot.session.open {
        return Err(AdmissionRefusal::SessionNotOpen);
    }
    if snapshot.price_version.id != snapshot.session.price_version {
        return Err(AdmissionRefusal::PriceVersionMismatch);
    }
    if request.route.metering != Metering::Priced {
        return Err(AdmissionRefusal::RouteUnmetered);
    }
    let declared: Vec<&str> = request
        .route
        .declared_categories
        .iter()
        .map(String::as_str)
        .collect();
    units_cover_route(&declared, &request.maximum_units)?;
    let version = &snapshot.price_version;
    let (provider, model) = (
        request.route.provider.as_str(),
        request.route.model.as_str(),
    );
    Ok(Maximums {
        retail: version.retail_maximum(provider, model, &request.maximum_units)?,
        supplier: version.supplier_maximum(provider, model, &request.maximum_units)?,
    })
}

fn check_ceilings(
    snapshot: &AdmissionSnapshot,
    retail_maximum: u64,
) -> Result<(), AdmissionRefusal> {
    let over = |current: u64, limit: u64| {
        current
            .checked_add(retail_maximum)
            .is_none_or(|total| total > limit)
    };
    if let Some(limit) = snapshot.session.session_ceiling_micro_dollars
        && over(
            snapshot
                .session
                .charged_total_micro_dollars
                .saturating_add(snapshot.open_holds_micro_dollars),
            limit,
        )
    {
        return Err(AdmissionRefusal::CeilingReached(Ceiling::Session));
    }
    if over(
        snapshot
            .charged_last_hour_micro_dollars
            .saturating_add(snapshot.open_holds_micro_dollars),
        snapshot.session.hourly_ceiling_micro_dollars,
    ) {
        return Err(AdmissionRefusal::CeilingReached(Ceiling::Hourly));
    }
    Ok(())
}

/// One lot's part of a reservation.
struct Hold {
    allocation: AllocationId,
    lot: LotId,
    class: LotClass,
    held: u64,
}

/// Splits the retail maximum over the eligible allocations (active allocation, active payer, an
/// authorized member: 11.2 step 4), then over each allocation's lots, promotional first.
fn hold_shares(
    snapshot: &AdmissionSnapshot,
    request: &AdmissionRequest,
    retail_maximum: u64,
) -> Result<Vec<Hold>, AdmissionRefusal> {
    let eligible: Vec<&(AllocationState, u64)> = snapshot
        .allocations
        .iter()
        .filter(|(allocation, weight)| {
            *weight > 0
                && allocation.status == AllocationStatus::Active
                && allocation.payer_active
                && request.authorized_payers.contains(&allocation.payer)
        })
        .collect();
    let capped: Vec<(AllocationId, u64, u64)> = eligible
        .iter()
        .map(|(allocation, weight)| {
            let held: u64 = allocation
                .lots
                .iter()
                .filter(|lot| lot.state == LotState::Active)
                .map(|lot| lot.allocated_micro_dollars)
                .sum();
            let hourly_remaining = allocation
                .approved_hourly_ceiling_micro_dollars
                .saturating_sub(allocation.charged_last_hour_micro_dollars)
                .saturating_sub(allocation.open_holds_micro_dollars);
            let session_remaining =
                allocation
                    .approved_session_ceiling_micro_dollars
                    .map_or(u64::MAX, |ceiling| {
                        ceiling
                            .saturating_sub(allocation.charged_micro_dollars)
                            .saturating_sub(allocation.open_holds_micro_dollars)
                    });
            (
                allocation.allocation,
                *weight,
                held.min(hourly_remaining).min(session_remaining),
            )
        })
        .collect();
    let shares = if retail_maximum == 0 {
        Vec::new()
    } else {
        split_capped(retail_maximum, &capped).map_err(|_| AdmissionRefusal::FundingRequired)?
    };
    let mut holds = Vec::new();
    for (allocation_id, share) in shares.into_iter().filter(|(_, share)| *share > 0) {
        let Some((allocation, _)) = eligible
            .iter()
            .find(|(allocation, _)| allocation.allocation == allocation_id)
        else {
            return Err(AdmissionRefusal::FundingRequired);
        };
        let drawable: Vec<DrawableLot> = allocation
            .lots
            .iter()
            .map(|lot| DrawableLot {
                lot: lot.lot,
                class: lot.class,
                state: lot.state,
                created_at_ms: lot.created_at_ms,
                available_micro_dollars: lot.allocated_micro_dollars,
            })
            .collect();
        let portions =
            plan_draw(share, &drawable).map_err(|_| AdmissionRefusal::FundingRequired)?;
        holds.extend(portions.into_iter().map(|(lot, class, held)| Hold {
            allocation: allocation_id,
            lot,
            class,
            held,
        }));
    }
    Ok(holds)
}

/// The lines of the reservation entry: allocated to reserved per hold, supplier exposure opened.
fn reservation_lines(
    snapshot: &AdmissionSnapshot,
    request: &AdmissionRequest,
    holds: &[Hold],
    supplier_maximum: u64,
) -> Result<Vec<Line>, AdmissionRefusal> {
    let keys = Keys::new(snapshot.session.currency);
    let entry = AdmissionRefusal::Entry;
    let mut lines = Vec::new();
    for hold in holds {
        lines.push(
            Line::debit(
                keys.allocated(hold.class, hold.allocation, hold.lot),
                hold.held,
            )
            .map_err(entry)?,
        );
        lines.push(
            Line::credit(
                keys.reserved(hold.class, hold.allocation, hold.lot),
                hold.held,
            )
            .map_err(entry)?,
        );
    }
    if supplier_maximum > 0 {
        let provider = request.route.provider.as_str();
        lines.push(Line::debit(keys.exposure_open(provider), supplier_maximum).map_err(entry)?);
        lines.push(Line::credit(keys.exposure_offset(provider), supplier_maximum).map_err(entry)?);
    }
    if lines.is_empty() {
        return Err(AdmissionRefusal::NothingToAdmit);
    }
    Ok(lines)
}

/// The rows of an admitted operation: reservation, shares, attempt, quote use, receipt, and the
/// timer that releases an attempt nobody sends.
fn admission_changes(
    snapshot: &AdmissionSnapshot,
    request: &AdmissionRequest,
    maximums: &Maximums,
    holds: &[Hold],
    receipt: &ReceiptRow,
    plan: &mut FinancePlan,
) {
    let (provider, model) = (
        request.route.provider.as_str(),
        request.route.model.as_str(),
    );
    plan.changes.push(RowChange::insert(
        ChangeKind::ReservationInsert,
        Row::new()
            .bytes("reservation_id", request.reservation.as_bytes())
            .bytes("session_id", request.session.as_bytes())
            .text("payer_kind", "table")
            .text("kind", "provider")
            .bytes("operation_id", request.operation.as_bytes())
            .opt_bytes(
                "quote_id",
                request.quote.as_ref().map(|id| id.as_bytes().as_slice()),
            )
            .bytes("price_version_id", snapshot.price_version.id.as_bytes())
            .uint("retail_maximum_micro_dollars", maximums.retail)
            .int("admitted_at_ms", request.now_ms)
            .text("state", "held"),
    ));
    for hold in holds {
        plan.changes.push(RowChange::insert(
            ChangeKind::ReservationShareInsert,
            Row::new()
                .bytes("reservation_id", request.reservation.as_bytes())
                .bytes("allocation_id", hold.allocation.as_bytes())
                .bytes("lot_id", hold.lot.as_bytes())
                .text("class", hold.class.label())
                .uint("held_micro_dollars", hold.held),
        ));
    }
    plan.changes.push(RowChange::insert(
        ChangeKind::ProviderAttemptInsert,
        Row::new()
            .bytes("attempt_id", request.attempt.as_bytes())
            .bytes("reservation_id", request.reservation.as_bytes())
            .bytes("effect_id", request.effect.as_bytes())
            .bytes("campaign_id", request.campaign.as_bytes())
            .bytes("tenant_id", request.tenant.as_bytes())
            .text("provider", provider)
            .text("model", model)
            .text("route", &request.route.name)
            .uint("supplier_maximum_micro_dollars", maximums.supplier)
            .uint("supplier_exposure_micro_dollars", maximums.supplier)
            .text("dispatch_state", "reserved")
            .text("customer_state", "held")
            .text("supplier_state", "exposure_open")
            .int("admitted_at_ms", request.now_ms),
    ));
    if let Some(quote) = &snapshot.quote
        && request.quote.is_some()
    {
        plan.changes.push(RowChange::update(
            ChangeKind::QuoteUpdate,
            Row::new()
                .bytes("quote_id", quote.quote.as_bytes())
                .text("state", "used")
                .bytes("used_by_reservation", request.reservation.as_bytes()),
            quote.revision,
        ));
    }
    plan.changes.push(RowChange::insert(
        ChangeKind::OperationReceiptInsert,
        Row::new()
            .text("namespace", "admission")
            .bytes("principal", request.campaign.as_bytes())
            .bytes("operation_id", request.operation.as_bytes())
            .bytes("fingerprint", &receipt.fingerprint)
            .json("outcome", &receipt_json(receipt))
            .int("created_at_ms", request.now_ms),
    ));
    plan.changes.push(RowChange::insert(
        ChangeKind::WorkItemUpsert,
        Row::new()
            .text("kind", "reserved_expiry")
            .bytes("ref_id", request.attempt.as_bytes())
            .int(
                "due_at_ms",
                request.now_ms.saturating_add(request.reserved_expiry_ms),
            ),
    ));
}

/// Decides one admission (11.2 steps 2-5). Table-paid work only; company-paid admission is
/// TODO(FIN-04).
pub fn decide_admission(
    snapshot: &AdmissionSnapshot,
    request: &AdmissionRequest,
) -> Result<AdmissionDecision, AdmissionRefusal> {
    let request_fingerprint = fingerprint(request);
    if let Some(receipt) = &snapshot.receipt {
        return if receipt.fingerprint == request_fingerprint {
            Ok(AdmissionDecision::AlreadyAdmitted(receipt.clone()))
        } else {
            Err(AdmissionRefusal::Conflict)
        };
    }
    let maximums = price_request(snapshot, request)?;
    check_quote(request, snapshot, maximums.retail)?;
    check_ceilings(snapshot, maximums.retail)?;
    let holds = hold_shares(snapshot, request, maximums.retail)?;
    let lines = reservation_lines(snapshot, request, &holds, maximums.supplier)?;

    let receipt = ReceiptRow {
        fingerprint: request_fingerprint,
        reservation: request.reservation,
        attempt: request.attempt,
        retail_maximum_micro_dollars: maximums.retail,
        supplier_maximum_micro_dollars: maximums.supplier,
        price_version: snapshot.price_version.id,
    };
    let mut plan = FinancePlan::new(request.event_id, EventKind::AdmissionAdmitted);
    admission_changes(snapshot, request, &maximums, &holds, &receipt, &mut plan);
    plan.entries.push(JournalEntry {
        entry_id: request.entry_id,
        kind: EntryKind::Reservation,
        business_key: business_key(
            "reservation",
            &[request.campaign.as_bytes(), request.operation.as_bytes()],
        ),
        external_ref: None,
        stream: (StreamKind::Session, *request.session.as_bytes()),
        effective_at_ms: request.now_ms,
        reason: EntryReason::Admitted,
        actor: Actor::System,
        correlation: Correlation {
            campaign: Some(request.campaign),
            session: Some(request.session),
            attempt: Some(request.attempt),
            reservation: Some(request.reservation),
            price_version: Some(snapshot.price_version.id),
        },
        lines,
    });
    plan.check().map_err(AdmissionRefusal::Plan)?;
    Ok(AdmissionDecision::Admitted(Box::new(Admitted {
        plan,
        receipt,
    })))
}

/// The receipt outcome stored as JSON text.
pub fn receipt_json(receipt: &ReceiptRow) -> String {
    format!(
        "{{\"reservation\":\"{}\",\"attempt\":\"{}\",\"retail_maximum\":{},\"supplier_maximum\":{},\"price_version\":\"{}\"}}",
        receipt.reservation.hex(),
        receipt.attempt.hex(),
        receipt.retail_maximum_micro_dollars,
        receipt.supplier_maximum_micro_dollars,
        hex(receipt.price_version.as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(maximum_units: Vec<(String, u64)>) -> AdmissionRequest {
        AdmissionRequest {
            event_id: EventId::from_array([1; 16]),
            entry_id: EntryId::from_array([2; 16]),
            reservation: ReservationId::from_array([3; 16]),
            attempt: AttemptId::from_array([4; 16]),
            effect: EffectId::from_array([5; 16]),
            operation: OperationKey::from_array([6; 16]),
            tenant: TenantKey::from_array([7; 16]),
            campaign: CampaignKey::from_array([8; 16]),
            session: SessionKey::from_array([9; 16]),
            route: Route {
                provider: "p".into(),
                model: "m".into(),
                name: "n".into(),
                declared_categories: vec![],
                metering: Metering::Priced,
            },
            work_class: WorkClass::Essential,
            maximum_units,
            estimate_units: Vec::new(),
            quote: None,
            request_digest: [0; 32],
            authorized_payers: BTreeSet::new(),
            now_ms: 0,
            reserved_expiry_ms: 0,
        }
    }

    #[test]
    fn two_different_unit_lists_never_share_a_fingerprint_even_when_a_category_holds_the_separators()
     {
        let split = request(vec![("a".into(), 1), ("b".into(), 2)]);
        let joined = request(vec![("a=1;b".into(), 2)]);
        assert_ne!(fingerprint(&split), fingerprint(&joined));
    }

    #[test]
    fn the_order_the_categories_arrive_in_does_not_change_the_fingerprint() {
        let one = request(vec![("a".into(), 1), ("b".into(), 2)]);
        let other = request(vec![("b".into(), 2), ("a".into(), 1)]);
        assert_eq!(fingerprint(&one), fingerprint(&other));
    }

    #[test]
    fn an_absent_quote_and_a_present_one_fingerprint_differently() {
        let mut quoted = request(vec![("a".into(), 1)]);
        quoted.quote = Some(QuoteId::from_array([0; 16]));
        assert_ne!(
            fingerprint(&request(vec![("a".into(), 1)])),
            fingerprint(&quoted)
        );
    }
}
