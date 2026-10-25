use df_types::{Currency, LiabilityRate, Money, MoneyError, OperationId, Usage};

use crate::ObservationAuthority;

/// Canonical counter order for caller-bound current periods in one currency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpendScope {
    PlatformDay,
    PlatformMonth,
    SupplierAccount,
    TenantPayer,
    Campaign,
    Job,
}

const SPEND_SCOPES: [SpendScope; 6] = [
    SpendScope::PlatformDay,
    SpendScope::PlatformMonth,
    SpendScope::SupplierAccount,
    SpendScope::TenantPayer,
    SpendScope::Campaign,
    SpendScope::Job,
];

/// Exact current exposure, including every reserved and Unknown worst-case maximum.
/// Expiry, cancellation, lease loss or restore cannot subtract Unknown exposure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpendCounter {
    pub used: Money,
    pub limit: Money,
}

/// Supplied transaction snapshot, not a ledger or independent budget authority.
/// Rows are ordered platform day, platform month, supplier/account, tenant/payer,
/// campaign, job. Both platform period caps are mandatory, with no time-based reset.
/// The owner binds all rows to authenticated scope and locks them in this order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpendSnapshot {
    pub currency: Currency,
    pub revision: u64,
    pub counters: [SpendCounter; 6],
}

/// Supplied current consent in exact micro-units; no price or grant is selected here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpendConsent {
    pub revision: u64,
    pub maximum: Money,
}

/// Durable lookup status, supplied for the exact requested operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpendOperationStatus {
    Unseen,
    Recorded,
    Unknown,
}

/// Unknown never becomes Unseen merely because of expiry, cancellation or restore.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpendOperationObservation {
    pub operation: OperationId,
    pub status: SpendOperationStatus,
}

/// The quote owner supplies a conservatively computed maximum, never an estimate.
/// Native admission separately binds quote/price/source/provider/attempt and consent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpendRequest {
    pub operation: OperationId,
    pub expected_revision: u64,
    pub expected_consent_revision: u64,
    pub maximum_supplier_liability: Money,
}

/// Refusal produces no reservation, counter changes, customer charge or dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpendRefusal {
    StaleRevision,
    StaleConsent,
    WrongOperationObservation,
    DuplicateOperation,
    UnknownOperation,
    Money(MoneyError),
    CustomerConsentExceeded,
    BudgetExceeded(SpendScope),
    RevisionOverflow,
}

impl From<MoneyError> for SpendRefusal {
    fn from(error: MoneyError) -> Self {
        Self::Money(error)
    }
}

/// Candidate only, never a reservation, dispatch permit, charge or commit receipt.
/// Its owner rechecks revisions and uniqueness and atomically commits every counter
/// with reservation/accepted-intent linkage. Protected journal confirmation precedes
/// egress. Rights, entitlement, expiry, units, concurrency and storage remain required.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpendProposal {
    pub operation: OperationId,
    pub before_revision: u64,
    pub consent_revision: u64,
    pub consented_maximum: Money,
    pub maximum_supplier_liability: Money,
    pub next: SpendSnapshot,
}

/// Computes all six exposure increases in stable order without mutating input.
/// Money tags are checked before any arithmetic; no currency conversion is attempted.
/// Unknown operation liability cannot be resent or released through this function.
/// Shared exact `LiabilityRate::liability` can compute a rounded-up quoted maximum;
/// default rates, customer prices and invoice settlement are outside this boundary.
pub fn propose_spend(
    snapshot: &SpendSnapshot,
    consent: SpendConsent,
    request: SpendRequest,
    observation: SpendOperationObservation,
) -> Result<SpendProposal, SpendRefusal> {
    if request.expected_revision != snapshot.revision {
        return Err(SpendRefusal::StaleRevision);
    }
    if request.expected_consent_revision != consent.revision {
        return Err(SpendRefusal::StaleConsent);
    }
    if observation.operation != request.operation {
        return Err(SpendRefusal::WrongOperationObservation);
    }
    match observation.status {
        SpendOperationStatus::Recorded => return Err(SpendRefusal::DuplicateOperation),
        SpendOperationStatus::Unknown => return Err(SpendRefusal::UnknownOperation),
        SpendOperationStatus::Unseen => {}
    }
    let maximum_supplier_liability = request.maximum_supplier_liability;
    if maximum_supplier_liability.currency() != snapshot.currency
        || consent.maximum.currency() != snapshot.currency
        || snapshot.counters.iter().any(|counter| {
            counter.used.currency() != snapshot.currency
                || counter.limit.currency() != snapshot.currency
        })
    {
        return Err(MoneyError::CurrencyMismatch.into());
    }
    if maximum_supplier_liability.micros() > consent.maximum.micros() {
        return Err(SpendRefusal::CustomerConsentExceeded);
    }

    let mut next = *snapshot;
    next.revision = snapshot
        .revision
        .checked_add(1)
        .ok_or(SpendRefusal::RevisionOverflow)?;
    for (scope, counter) in SPEND_SCOPES.into_iter().zip(next.counters.iter_mut()) {
        let proposed_used = counter.used.checked_add(maximum_supplier_liability)?;
        if proposed_used.micros() > counter.limit.micros() {
            return Err(SpendRefusal::BudgetExceeded(scope));
        }
        counter.used = proposed_used;
    }
    Ok(SpendProposal {
        operation: request.operation,
        before_revision: snapshot.revision,
        consent_revision: consent.revision,
        consented_maximum: consent.maximum,
        maximum_supplier_liability,
        next,
    })
}

/// Supplied durable status of the exact reservation. The native owner binds the
/// operation to tenant, supplier, quote and accepted intent before using it here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpendReservationStatus {
    Dispatching,
    UnknownLiability,
    SettledKnown,
    UnsentCanceled,
    Released,
}

/// Caller-bound reservation, not a second ledger or proof of a provider send.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpendReservation {
    pub operation: OperationId,
    pub maximum_supplier_liability: Money,
    pub status: SpendReservationStatus,
}

/// Unknown usage retains the full reservation until same-key reconciliation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpendSettlementOutcome {
    Unknown,
    Known {
        actual_usage: Usage,
        billed_waste: Money,
        liability_rate: LiabilityRate,
    },
}

/// The native owner verifies invoice/usage, rate and operation provenance before
/// supplying this observation. This assertion is not a verification mechanism.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpendSettlementObservation {
    pub authority: ObservationAuthority,
    pub operation: OperationId,
    pub expected_revision: u64,
    pub outcome: SpendSettlementOutcome,
}

/// Refusal leaves all caller-owned state and the full reservation unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpendSettlementRefusal {
    Unverified,
    NeedsAuthoritativeRefresh,
    StaleRevision,
    WrongOperation,
    AlreadySettled,
    NotDispatched,
    UnknownLiability,
    Money(MoneyError),
    ReservationExceedsExposure(SpendScope),
    RevisionOverflow,
}

impl From<MoneyError> for SpendSettlementRefusal {
    fn from(error: MoneyError) -> Self {
        Self::Money(error)
    }
}

/// Candidate only. A native transaction rechecks operation/revision uniqueness,
/// appends exact ledger and incident entries, and marks the reservation final once.
/// Customer exposure never rises above its consented maximum on supplier overrun.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpendSettlementProposal {
    pub operation: OperationId,
    pub before_revision: u64,
    pub maximum_supplier_liability: Money,
    pub actual_supplier_liability: Money,
    pub known_unused_release: Money,
    pub platform_loss: Money,
    pub supplier_admissions_blocked: bool,
    pub next: SpendSnapshot,
}

/// Settles verified actual usage plus billed waste against one held maximum.
/// Known unused exposure releases from all six counters. Overrun is an explicit
/// platform loss: platform and supplier counters record it, customer counters do
/// not grow, and the native owner must block affected supplier admissions.
pub fn propose_spend_settlement(
    snapshot: &SpendSnapshot,
    reservation: SpendReservation,
    observation: SpendSettlementObservation,
) -> Result<SpendSettlementProposal, SpendSettlementRefusal> {
    match observation.authority {
        ObservationAuthority::Unverified => return Err(SpendSettlementRefusal::Unverified),
        ObservationAuthority::WebhookOnly => {
            return Err(SpendSettlementRefusal::NeedsAuthoritativeRefresh);
        }
        ObservationAuthority::VerifiedLatest => {}
    }
    if observation.expected_revision != snapshot.revision {
        return Err(SpendSettlementRefusal::StaleRevision);
    }
    if observation.operation != reservation.operation {
        return Err(SpendSettlementRefusal::WrongOperation);
    }
    match reservation.status {
        SpendReservationStatus::SettledKnown => {
            return Err(SpendSettlementRefusal::AlreadySettled);
        }
        SpendReservationStatus::UnsentCanceled | SpendReservationStatus::Released => {
            return Err(SpendSettlementRefusal::NotDispatched);
        }
        SpendReservationStatus::Dispatching | SpendReservationStatus::UnknownLiability => {}
    }
    let SpendSettlementOutcome::Known {
        actual_usage,
        billed_waste,
        liability_rate,
    } = observation.outcome
    else {
        return Err(SpendSettlementRefusal::UnknownLiability);
    };
    let maximum_supplier_liability = reservation.maximum_supplier_liability;
    if maximum_supplier_liability.currency() != snapshot.currency
        || billed_waste.currency() != snapshot.currency
        || liability_rate.currency() != snapshot.currency
        || snapshot.counters.iter().any(|counter| {
            counter.used.currency() != snapshot.currency
                || counter.limit.currency() != snapshot.currency
        })
    {
        return Err(MoneyError::CurrencyMismatch.into());
    }
    let actual_supplier_liability = liability_rate
        .liability(actual_usage)?
        .checked_add(billed_waste)?;
    let mut next = *snapshot;
    next.revision = snapshot
        .revision
        .checked_add(1)
        .ok_or(SpendSettlementRefusal::RevisionOverflow)?;
    let known_unused_release =
        if actual_supplier_liability.micros() <= maximum_supplier_liability.micros() {
            maximum_supplier_liability.checked_sub(actual_supplier_liability)?
        } else {
            Money::new(snapshot.currency, 0)
        };
    let platform_loss = if actual_supplier_liability.micros() > maximum_supplier_liability.micros()
    {
        actual_supplier_liability.checked_sub(maximum_supplier_liability)?
    } else {
        Money::new(snapshot.currency, 0)
    };
    for (index, (scope, counter)) in SPEND_SCOPES
        .into_iter()
        .zip(next.counters.iter_mut())
        .enumerate()
    {
        if counter.used.micros() < maximum_supplier_liability.micros() {
            return Err(SpendSettlementRefusal::ReservationExceedsExposure(scope));
        }
        counter.used = counter.used.checked_sub(known_unused_release)?;
        if index < 3 {
            counter.used = counter.used.checked_add(platform_loss)?;
        }
    }
    Ok(SpendSettlementProposal {
        operation: reservation.operation,
        before_revision: snapshot.revision,
        maximum_supplier_liability,
        actual_supplier_liability,
        known_unused_release,
        platform_loss,
        supplier_admissions_blocked: platform_loss.micros() > 0,
        next,
    })
}
