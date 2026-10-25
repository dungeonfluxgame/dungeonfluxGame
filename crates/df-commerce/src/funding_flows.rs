//! The flows that put credit into the system and move it into a session: publishing a price
//! version, opening session funding, the trial grant, a confirmed purchase, and an allocation
//! (planning/finance-ledger-research.md 7, 8, 9.1, 16.1-16.3). Each decides on a snapshot read
//! under the flow's locks and returns a [`FinancePlan`].

use crate::digest::business_key;
use crate::finance_ids::{
    AllocationId, CampaignKey, EntryId, EventId, LotId, OperationKey, PayerId, PriceVersionId,
    SessionKey, TenantKey,
};
use crate::funding::{
    DrawRefusal, DrawableLot, LotClass, LotState, TrialDecision, TrialPolicy, decide_trial,
    plan_draw,
};
use crate::keys::Keys;
use crate::ledger::{
    Actor, Correlation, CurrencyTag, EntryKind, EntryReason, EntryRefusal, ExternalRefKind,
    JournalEntry, Line, StreamKind,
};
use crate::plan::{ChangeKind, EventKind, FinancePlan, PlanRefusal, Row, RowChange};
use crate::price_schedule::PriceVersion;
use crate::snapshot::{AllocationState, SessionState};

/// Why a funding flow refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FundingRefusal {
    Trial(TrialDecision),
    /// The payment was already applied (PAY-02): nothing to do.
    AlreadyApplied,
    PayerRestricted,
    SessionNotOpen,
    NothingToAllocate,
    Draw(DrawRefusal),
    Entry(EntryRefusal),
    Plan(PlanRefusal),
    AmountOverflow,
}

impl From<EntryRefusal> for FundingRefusal {
    fn from(refusal: EntryRefusal) -> Self {
        Self::Entry(refusal)
    }
}

fn ensure_fits(amount: u64) -> Result<(), FundingRefusal> {
    i64::try_from(amount)
        .map(drop)
        .map_err(|_| FundingRefusal::AmountOverflow)
}

/// What a system entry carries besides its lines.
struct EntrySpec {
    entry_id: EntryId,
    kind: EntryKind,
    key: [u8; 32],
    stream: (StreamKind, [u8; 16]),
    external_ref: Option<(ExternalRefKind, String)>,
    reason: EntryReason,
    correlation: Correlation,
    now_ms: i64,
}

impl EntrySpec {
    fn entry(self, lines: Vec<Line>) -> JournalEntry {
        JournalEntry {
            entry_id: self.entry_id,
            kind: self.kind,
            business_key: self.key,
            external_ref: self.external_ref,
            stream: self.stream,
            effective_at_ms: self.now_ms,
            reason: self.reason,
            actor: Actor::System,
            correlation: self.correlation,
            lines,
        }
    }
}

/// Publishing a price version (9.1): the version row, its tariff rows and the account keys of
/// the revenue, offset and per-provider accounts its flows post to.
pub fn plan_publish_price_version(
    event_id: EventId,
    version: &PriceVersion,
    now_ms: i64,
) -> FinancePlan {
    let mut plan = FinancePlan::new(event_id, EventKind::PriceVersionPublished);
    plan.changes.push(RowChange::insert(
        ChangeKind::PriceVersionInsert,
        Row::new()
            .bytes("price_version_id", version.id.as_bytes())
            .text("label", &version.label)
            .int("effective_from_ms", now_ms)
            .text("currency", version.currency.as_str())
            .uint(
                "platform_rate_micro_dollars_per_hour",
                version.platform_rate_micro_dollars_per_hour,
            )
            .uint("markup_numerator", version.markup_numerator)
            .uint("markup_denominator", version.markup_denominator)
            .boolean("proposed_pilot", version.proposed_pilot)
            .text("status", "published")
            .int("published_at_ms", now_ms),
    ));
    for line in &version.tariff {
        plan.changes.push(RowChange::insert(
            ChangeKind::PriceVersionTariffInsert,
            Row::new()
                .bytes("price_version_id", version.id.as_bytes())
                .text("provider", &line.provider)
                .text("model", &line.model)
                .text("category", &line.category)
                .uint("context_tier_max_tokens", line.context_tier_max_tokens)
                .uint("rate_nano_dollars", line.nano_usd)
                .uint("per_units", line.per),
        ));
    }
    let mut providers: Vec<&str> = version
        .tariff
        .iter()
        .map(|line| line.provider.as_str())
        .collect();
    providers.sort_unstable();
    providers.dedup();
    for key in Keys::new(version.currency).publication_keys(&providers) {
        plan.changes.push(RowChange::key(&key));
    }
    plan
}

/// The session policy a table starts with.
#[derive(Clone, Copy, Debug)]
pub struct SessionPolicy {
    pub hourly_ceiling_micro_dollars: u64,
    pub session_ceiling_micro_dollars: Option<u64>,
    pub per_request_threshold_micro_dollars: u64,
}

/// A session's funding to open.
#[derive(Clone, Copy, Debug)]
pub struct OpenSession {
    pub event_id: EventId,
    pub session: SessionKey,
    pub tenant: TenantKey,
    pub campaign: CampaignKey,
    pub price_version: PriceVersionId,
    pub currency: CurrencyTag,
    pub policy: SessionPolicy,
    pub now_ms: i64,
}

/// Opening a session's funding pins the price version for its life (8.1).
pub fn plan_open_session(request: OpenSession) -> FinancePlan {
    let OpenSession {
        event_id,
        session,
        tenant,
        campaign,
        price_version,
        currency,
        policy,
        now_ms,
    } = request;
    let mut plan = FinancePlan::new(event_id, EventKind::SessionFundingOpened);
    plan.changes.push(RowChange::insert(
        ChangeKind::SessionFundingInsert,
        Row::new()
            .bytes("session_id", session.as_bytes())
            .bytes("tenant_id", tenant.as_bytes())
            .bytes("campaign_id", campaign.as_bytes())
            .bytes("price_version_id", price_version.as_bytes())
            .text("currency", currency.as_str())
            .text("status", "open")
            .uint(
                "hourly_ceiling_micro_dollars",
                policy.hourly_ceiling_micro_dollars,
            )
            .opt_int(
                "session_ceiling_micro_dollars",
                policy
                    .session_ceiling_micro_dollars
                    .map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
            )
            .uint(
                "per_request_threshold_micro_dollars",
                policy.per_request_threshold_micro_dollars,
            )
            .int("opened_at_ms", now_ms),
    ));
    plan.changes.push(RowChange::insert(
        ChangeKind::FundingEpochInsert,
        Row::new()
            .bytes("session_id", session.as_bytes())
            .int("epoch_seq", 1)
            .text("reason", "opened")
            .text("split_mode", "proportional")
            .int("created_at_ms", now_ms),
    ));
    plan
}

/// What the trial flow read.
#[derive(Clone, Copy, Debug)]
pub struct TrialSnapshot {
    pub payer_exists: bool,
    pub trial_by_account: bool,
    pub trial_by_email: bool,
}

/// The trial grant request: B passes the HMAC of the verified email, never the address.
#[derive(Clone, Copy, Debug)]
pub struct TrialRequest {
    pub event_id: EventId,
    pub lot: LotId,
    pub entry_id: EntryId,
    pub payer: PayerId,
    pub email_grant_key: [u8; 32],
    pub currency: CurrencyTag,
    pub policy: TrialPolicy,
    pub now_ms: i64,
}

/// 16.1: the trial grant. Posts to the Promo book only; promotional credit is never revenue.
pub fn plan_trial(
    snapshot: TrialSnapshot,
    request: TrialRequest,
) -> Result<FinancePlan, FundingRefusal> {
    let decision = decide_trial(
        request.policy,
        snapshot.trial_by_account,
        snapshot.trial_by_email,
    );
    if decision != TrialDecision::Grant {
        return Err(FundingRefusal::Trial(decision));
    }
    let keys = Keys::new(request.currency);
    let amount = request.policy.amount_micro_dollars;
    let mut plan = FinancePlan::new(request.event_id, EventKind::TrialGranted);
    if !snapshot.payer_exists {
        plan.changes
            .push(register_payer(request.payer, request.now_ms));
    }
    plan.changes.push(RowChange::insert(
        ChangeKind::FundingLotInsert,
        Row::new()
            .bytes("lot_id", request.lot.as_bytes())
            .bytes("payer_account_id", request.payer.as_bytes())
            .text("class", "promotional")
            .text("promo_source", "trial")
            .text("currency", request.currency.as_str())
            .uint("original_micro_dollars", amount)
            .text("state", "active")
            .int("created_at_ms", request.now_ms),
    ));
    let available = keys.available(LotClass::Promotional, request.payer, request.lot);
    let offset = keys.promotional_offset("trial");
    plan.changes.push(RowChange::key(&available));
    plan.changes.push(RowChange::key(&offset));
    plan.changes.push(RowChange::insert(
        ChangeKind::TrialGrantInsert,
        Row::new()
            .bytes("account_id", request.payer.as_bytes())
            .bytes("email_grant_key", &request.email_grant_key)
            .bytes("lot_id", request.lot.as_bytes())
            .int("granted_at_ms", request.now_ms),
    ));
    plan.entries.push(
        EntrySpec {
            entry_id: request.entry_id,
            kind: EntryKind::TrialGrant,
            key: business_key("trial_grant", &[request.payer.as_bytes()]),
            stream: (StreamKind::Payer, *request.payer.as_bytes()),
            external_ref: Some((ExternalRefKind::TrialGrant, request.payer.hex())),
            reason: EntryReason::TrialGranted,
            correlation: Correlation::default(),
            now_ms: request.now_ms,
        }
        .entry(vec![
            Line::debit(offset, amount)?,
            Line::credit(available, amount)?,
        ]),
    );
    plan.check().map_err(FundingRefusal::Plan)?;
    Ok(plan)
}

fn register_payer(payer: PayerId, now_ms: i64) -> RowChange {
    RowChange::insert(
        ChangeKind::PayerInsert,
        Row::new()
            .bytes("account_id", payer.as_bytes())
            .text("standing", "active")
            .int("created_at_ms", now_ms),
    )
}

/// What the purchase flow read.
#[derive(Clone, Copy, Debug)]
pub struct PurchaseSnapshot {
    pub payer_exists: bool,
    pub payment_state_exists: bool,
}

/// A confirmed payment, as B hands it over after re-reading the processor object (7.3).
#[derive(Clone, Debug)]
pub struct PurchaseRequest {
    pub event_id: EventId,
    pub lot: LotId,
    pub entry_id: EntryId,
    pub payer: PayerId,
    pub processor: String,
    pub payment_ref: String,
    pub charge_ref: Option<String>,
    pub currency: CurrencyTag,
    pub amount_micro_dollars: u64,
    pub now_ms: i64,
}

/// 16.2: a purchase confirmed by an authoritative processor state. A repeat of the same payment
/// reference changes nothing (PAY-02).
pub fn plan_purchase(
    snapshot: PurchaseSnapshot,
    request: &PurchaseRequest,
) -> Result<FinancePlan, FundingRefusal> {
    if snapshot.payment_state_exists {
        return Err(FundingRefusal::AlreadyApplied);
    }
    let keys = Keys::new(request.currency);
    let amount = request.amount_micro_dollars;
    let mut plan = FinancePlan::new(request.event_id, EventKind::PurchaseApplied);
    if !snapshot.payer_exists {
        plan.changes
            .push(register_payer(request.payer, request.now_ms));
    }
    plan.changes.push(RowChange::insert(
        ChangeKind::FundingLotInsert,
        Row::new()
            .bytes("lot_id", request.lot.as_bytes())
            .bytes("payer_account_id", request.payer.as_bytes())
            .text("class", "purchased")
            .text("currency", request.currency.as_str())
            .uint("original_micro_dollars", amount)
            .text("state", "active")
            .text("processor", &request.processor)
            .text("payment_ref", &request.payment_ref)
            .opt_text("charge_ref", request.charge_ref.as_deref())
            .int("created_at_ms", request.now_ms),
    ));
    plan.changes.push(RowChange::insert(
        ChangeKind::ProcessorPaymentStateInsert,
        Row::new()
            .text("processor", &request.processor)
            .text("payment_ref", &request.payment_ref)
            .bytes("lot_id", request.lot.as_bytes())
            .boolean("captured", true)
            .uint("captured_micro_dollars", amount)
            .int("applied_at_ms", request.now_ms),
    ));
    let available = keys.available(LotClass::Purchased, request.payer, request.lot);
    let clearing = keys.clearing(&request.processor);
    plan.changes.push(RowChange::key(&available));
    plan.changes.push(RowChange::key(&clearing));
    plan.entries.push(
        EntrySpec {
            entry_id: request.entry_id,
            kind: EntryKind::Purchase,
            key: business_key(
                "purchase",
                &[request.processor.as_bytes(), request.payment_ref.as_bytes()],
            ),
            stream: (StreamKind::Payer, *request.payer.as_bytes()),
            external_ref: Some((
                ExternalRefKind::ProcessorPayment,
                format!("{}|{}", request.processor, request.payment_ref),
            )),
            reason: EntryReason::PurchaseConfirmed,
            correlation: Correlation::default(),
            now_ms: request.now_ms,
        }
        .entry(vec![
            Line::debit(clearing, amount)?,
            Line::credit(available, amount)?,
        ]),
    );
    plan.check().map_err(FundingRefusal::Plan)?;
    Ok(plan)
}

/// One lot of the payer, with what is available and what this session's allocation holds of it.
#[derive(Clone, Copy, Debug)]
pub struct PayerLot {
    pub lot: LotId,
    pub class: LotClass,
    pub state: LotState,
    pub created_at_ms: i64,
    pub available_micro_dollars: u64,
}

/// The payer's existing allocation in the session, if any.
#[derive(Clone, Debug)]
pub struct ExistingAllocation {
    pub allocation: AllocationId,
    pub authorized_micro_dollars: u64,
    pub drawn_micro_dollars: u64,
    pub consent_revision: i64,
    pub revision: i64,
    pub lots_registered: Vec<LotId>,
}

/// Another allocation of the session (for the new epoch's weights).
#[derive(Clone, Copy, Debug)]
pub struct OtherAllocation {
    pub allocation: AllocationId,
    pub authorized_micro_dollars: u64,
    pub active: bool,
}

/// What the allocate flow read under its locks.
#[derive(Clone, Debug)]
pub struct AllocateSnapshot {
    pub payer_active: bool,
    pub session: SessionState,
    pub existing: Option<ExistingAllocation>,
    pub payer_lots: Vec<PayerLot>,
    pub other_allocations: Vec<OtherAllocation>,
}

/// The payer's consent to the session's price and policy (5.3).
#[derive(Clone, Copy, Debug)]
pub struct Consent {
    pub approved_hourly_ceiling_micro_dollars: u64,
    pub approved_session_ceiling_micro_dollars: Option<u64>,
    pub approval_threshold_micro_dollars: u64,
    pub terms_version: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct AllocateRequest {
    pub event_id: EventId,
    pub entry_id: EntryId,
    /// Used when the payer has no allocation in the session yet.
    pub new_allocation: AllocationId,
    pub operation: OperationKey,
    pub payer: PayerId,
    pub amount_micro_dollars: u64,
    pub consent: Consent,
    pub now_ms: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct WithdrawLot {
    pub lot: LotId,
    pub class: LotClass,
    pub allocated_micro_dollars: u64,
}

#[derive(Clone, Debug)]
pub struct WithdrawalSnapshot {
    pub session: SessionState,
    pub allocation: AllocationState,
    pub lots: Vec<WithdrawLot>,
    pub receipt_exists: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct WithdrawRequest {
    pub event_id: EventId,
    pub entry_id: EntryId,
    pub operation: OperationKey,
    pub now_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WithdrawalRefusal {
    SessionNotOpen,
    NothingToWithdraw,
    AmountOverflow,
    Entry(EntryRefusal),
    Plan(PlanRefusal),
}

/// Returns only the unreserved allocation balance to its payer (5.5).
pub fn plan_withdrawal(
    snapshot: &WithdrawalSnapshot,
    request: &WithdrawRequest,
) -> Result<Option<FinancePlan>, WithdrawalRefusal> {
    if snapshot.receipt_exists {
        return Ok(None);
    }
    if !snapshot.session.open {
        return Err(WithdrawalRefusal::SessionNotOpen);
    }
    let total = snapshot.lots.iter().try_fold(0_u64, |sum, lot| {
        sum.checked_add(lot.allocated_micro_dollars)
            .ok_or(WithdrawalRefusal::AmountOverflow)
    })?;
    if total == 0 {
        return Err(WithdrawalRefusal::NothingToWithdraw);
    }
    let returned = snapshot
        .allocation
        .returned_micro_dollars
        .checked_add(total)
        .ok_or(WithdrawalRefusal::AmountOverflow)?;
    let authorized = snapshot
        .allocation
        .authorized_micro_dollars
        .checked_sub(total)
        .ok_or(WithdrawalRefusal::AmountOverflow)?;
    let keys = Keys::new(snapshot.session.currency);
    let mut lines = Vec::with_capacity(snapshot.lots.len() * 2);
    let mut plan = FinancePlan::new(request.event_id, EventKind::ReleaseApplied);
    for lot in &snapshot.lots {
        lines.push(
            Line::debit(
                keys.allocated(lot.class, snapshot.allocation.allocation, lot.lot),
                lot.allocated_micro_dollars,
            )
            .map_err(WithdrawalRefusal::Entry)?,
        );
        lines.push(
            Line::credit(
                keys.available(lot.class, snapshot.allocation.payer, lot.lot),
                lot.allocated_micro_dollars,
            )
            .map_err(WithdrawalRefusal::Entry)?,
        );
    }
    plan.changes.push(RowChange::update(
        ChangeKind::AllocationUpdate,
        Row::new()
            .bytes("allocation_id", snapshot.allocation.allocation.as_bytes())
            .uint("authorized_micro_dollars", authorized)
            .uint("returned_micro_dollars", returned)
            .text("status", "withdrawn"),
        snapshot.allocation.revision,
    ));
    plan.changes.push(RowChange::insert(
        ChangeKind::OperationReceiptInsert,
        Row::new()
            .text("namespace", "withdrawal")
            .bytes("principal", snapshot.session.campaign.as_bytes())
            .bytes("operation_id", request.operation.as_bytes())
            .bytes(
                "fingerprint",
                &business_key("withdrawal", &[snapshot.allocation.allocation.as_bytes()]),
            )
            .json("outcome", "{\"withdrawn\":true}")
            .int("created_at_ms", request.now_ms),
    ));
    plan.entries.push(
        EntrySpec {
            entry_id: request.entry_id,
            kind: EntryKind::AllocationRelease,
            key: business_key(
                "allocation_withdrawal",
                &[
                    snapshot.allocation.allocation.as_bytes(),
                    request.operation.as_bytes(),
                ],
            ),
            stream: (StreamKind::Payer, *snapshot.allocation.payer.as_bytes()),
            external_ref: None,
            reason: EntryReason::AllocationReleased,
            correlation: Correlation {
                session: Some(snapshot.session.session),
                campaign: Some(snapshot.session.campaign),
                price_version: Some(snapshot.session.price_version),
                ..Correlation::default()
            },
            now_ms: request.now_ms,
        }
        .entry(lines),
    );
    plan.check().map_err(WithdrawalRefusal::Plan)?;
    Ok(Some(plan))
}

/// The allocation an allocate flow adds to, and what it is after the flow.
struct AllocationTarget {
    allocation: AllocationId,
    authorized_after: u64,
    consent_revision_after: i64,
}

fn allocation_target(
    snapshot: &AllocateSnapshot,
    request: &AllocateRequest,
    plan: &mut FinancePlan,
) -> Result<AllocationTarget, FundingRefusal> {
    let amount = request.amount_micro_dollars;
    let session = snapshot.session.session;
    let Some(existing) = &snapshot.existing else {
        plan.changes.push(RowChange::insert(
            ChangeKind::AllocationInsert,
            Row::new()
                .bytes("allocation_id", request.new_allocation.as_bytes())
                .bytes("session_id", session.as_bytes())
                .bytes("payer_account_id", request.payer.as_bytes())
                .uint("authorized_micro_dollars", amount)
                .uint("drawn_micro_dollars", amount)
                .text("status", "active")
                .int("consent_revision", 1)
                .int("created_at_ms", request.now_ms),
        ));
        return Ok(AllocationTarget {
            allocation: request.new_allocation,
            authorized_after: amount,
            consent_revision_after: 1,
        });
    };
    let authorized_after = existing
        .authorized_micro_dollars
        .checked_add(amount)
        .ok_or(FundingRefusal::AmountOverflow)?;
    let drawn_after = existing
        .drawn_micro_dollars
        .checked_add(amount)
        .ok_or(FundingRefusal::AmountOverflow)?;
    let consent_revision_after = existing.consent_revision + 1;
    plan.changes.push(RowChange::update(
        ChangeKind::AllocationUpdate,
        Row::new()
            .bytes("allocation_id", existing.allocation.as_bytes())
            .uint("authorized_micro_dollars", authorized_after)
            .uint("drawn_micro_dollars", drawn_after)
            .int("consent_revision", consent_revision_after),
        existing.revision,
    ));
    Ok(AllocationTarget {
        allocation: existing.allocation,
        authorized_after,
        consent_revision_after,
    })
}

/// The keys, lot registrations and lines of drawing `portions` from the payer's available keys into
/// the allocation's allocated keys.
fn draw_lines(
    snapshot: &AllocateSnapshot,
    request: &AllocateRequest,
    target: &AllocationTarget,
    portions: &[(LotId, LotClass, u64)],
    plan: &mut FinancePlan,
) -> Result<Vec<Line>, FundingRefusal> {
    let keys = Keys::new(snapshot.session.currency);
    let mut lines = Vec::new();
    for (rank, (lot, class, portion)) in portions.iter().enumerate() {
        let allocated = keys.allocated(*class, target.allocation, *lot);
        plan.changes.push(RowChange::key(&allocated));
        plan.changes.push(RowChange::key(&keys.reserved(
            *class,
            target.allocation,
            *lot,
        )));
        let registered = snapshot
            .existing
            .as_ref()
            .is_some_and(|existing| existing.lots_registered.contains(lot));
        if !registered {
            plan.changes.push(RowChange::insert(
                ChangeKind::AllocationLotInsert,
                Row::new()
                    .bytes("allocation_id", target.allocation.as_bytes())
                    .bytes("lot_id", lot.as_bytes())
                    .int("draw_rank", i64::try_from(rank).unwrap_or(0)),
            ));
        }
        lines.push(Line::debit(
            keys.available(*class, request.payer, *lot),
            *portion,
        )?);
        lines.push(Line::credit(allocated, *portion)?);
    }
    Ok(lines)
}

fn consent_change(
    snapshot: &AllocateSnapshot,
    request: &AllocateRequest,
    target: &AllocationTarget,
) -> RowChange {
    RowChange::insert(
        ChangeKind::AllocationConsentInsert,
        Row::new()
            .bytes("allocation_id", target.allocation.as_bytes())
            .int("consent_revision", target.consent_revision_after)
            .bytes(
                "price_version_id",
                snapshot.session.price_version.as_bytes(),
            )
            .uint(
                "approved_hourly_ceiling",
                request.consent.approved_hourly_ceiling_micro_dollars,
            )
            .opt_int(
                "approved_session_ceiling",
                request
                    .consent
                    .approved_session_ceiling_micro_dollars
                    .map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
            )
            .text("approved_split_mode", "proportional")
            .uint(
                "approval_threshold_micro_dollars",
                request.consent.approval_threshold_micro_dollars,
            )
            .int("consent_terms_version", request.consent.terms_version)
            .int("consented_at_ms", request.now_ms),
    )
}

/// A new epoch: every active allocation weighs its authorized amount (8.3).
fn epoch_changes(
    snapshot: &AllocateSnapshot,
    target: &AllocationTarget,
    now_ms: i64,
    plan: &mut FinancePlan,
) {
    let session = snapshot.session.session;
    let epoch = snapshot.session.current_epoch + 1;
    plan.changes.push(RowChange::update(
        ChangeKind::SessionFundingUpdate,
        Row::new()
            .bytes("session_id", session.as_bytes())
            .int("current_epoch", epoch),
        snapshot.session.revision,
    ));
    plan.changes.push(RowChange::insert(
        ChangeKind::FundingEpochInsert,
        Row::new()
            .bytes("session_id", session.as_bytes())
            .int("epoch_seq", epoch)
            .text("reason", "allocation_changed")
            .text("split_mode", "proportional")
            .int("created_at_ms", now_ms),
    ));
    let mut weights: Vec<(AllocationId, u64)> = snapshot
        .other_allocations
        .iter()
        .filter(|other| {
            other.active
                && other.allocation != target.allocation
                && other.authorized_micro_dollars > 0
        })
        .map(|other| (other.allocation, other.authorized_micro_dollars))
        .collect();
    weights.push((target.allocation, target.authorized_after));
    weights.sort();
    for (member, weight) in weights {
        plan.changes.push(RowChange::insert(
            ChangeKind::EpochShareInsert,
            Row::new()
                .bytes("session_id", session.as_bytes())
                .int("epoch_seq", epoch)
                .bytes("allocation_id", member.as_bytes())
                .uint("weight", weight),
        ));
    }
}

/// 16.3: moves credit from the payer's available keys into the session allocation, promotional
/// lots first, and starts a new epoch (8.3). It is not revenue and moves no ownership.
pub fn plan_allocation(
    snapshot: &AllocateSnapshot,
    request: &AllocateRequest,
) -> Result<FinancePlan, FundingRefusal> {
    if !snapshot.payer_active {
        return Err(FundingRefusal::PayerRestricted);
    }
    if !snapshot.session.open {
        return Err(FundingRefusal::SessionNotOpen);
    }
    if request.amount_micro_dollars == 0 {
        return Err(FundingRefusal::NothingToAllocate);
    }
    ensure_fits(request.amount_micro_dollars)?;
    let drawable: Vec<DrawableLot> = snapshot
        .payer_lots
        .iter()
        .map(|lot| DrawableLot {
            lot: lot.lot,
            class: lot.class,
            state: lot.state,
            created_at_ms: lot.created_at_ms,
            available_micro_dollars: lot.available_micro_dollars,
        })
        .collect();
    let portions =
        plan_draw(request.amount_micro_dollars, &drawable).map_err(FundingRefusal::Draw)?;
    let mut plan = FinancePlan::new(request.event_id, EventKind::AllocationMade);
    let target = allocation_target(snapshot, request, &mut plan)?;
    let lines = draw_lines(snapshot, request, &target, &portions, &mut plan)?;
    plan.changes
        .push(consent_change(snapshot, request, &target));
    epoch_changes(snapshot, &target, request.now_ms, &mut plan);
    let session = snapshot.session.session;
    plan.entries.push(
        EntrySpec {
            entry_id: request.entry_id,
            kind: EntryKind::Allocation,
            key: business_key(
                "allocation",
                &[request.payer.as_bytes(), request.operation.as_bytes()],
            ),
            stream: (StreamKind::Payer, *request.payer.as_bytes()),
            external_ref: None,
            reason: EntryReason::Allocated,
            correlation: Correlation {
                session: Some(session),
                campaign: Some(snapshot.session.campaign),
                price_version: Some(snapshot.session.price_version),
                ..Correlation::default()
            },
            now_ms: request.now_ms,
        }
        .entry(lines),
    );
    plan.check().map_err(FundingRefusal::Plan)?;
    Ok(plan)
}
