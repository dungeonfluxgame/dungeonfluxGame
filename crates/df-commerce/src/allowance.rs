use df_types::{Money, MoneyError, PaidInvoiceId, PriceVersion, SubscriptionId};

use crate::{
    Access, PaymentOutcome, SubscriptionObservation, SubscriptionRefusal, SubscriptionState,
    observe_subscription,
};

/// Caller-supplied half-open paid period in authoritative UTC seconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaidPeriod {
    pub start: u64,
    pub end: u64,
}

/// Exact identity of a candidate allowance event, not an issued or committed grant.
/// Native uniqueness covers this tuple and one initial/nonoverlapping grant per period.
/// Subscription, invoice, and price kinds cannot substitute for one another:
/// ```compile_fail
/// use df_commerce::{AllowanceGrantId, PaidPeriod};
/// use df_types::{PaidInvoiceId, PriceVersion, SubscriptionId};
/// let subscription = SubscriptionId::from_bytes(&[1; 16]).unwrap();
/// let invoice = PaidInvoiceId::from_bytes(&[2; 16]).unwrap();
/// let price = PriceVersion::from_bytes(&[3; 16]).unwrap();
/// let _ = AllowanceGrantId { subscription: invoice, invoice: subscription,
///     period: PaidPeriod { start: 1, end: 2 }, price_version: price };
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllowanceGrantId {
    pub subscription: SubscriptionId,
    pub invoice: PaidInvoiceId,
    pub period: PaidPeriod,
    pub price_version: PriceVersion,
}

/// Selected disclosed terms. The exact supplier ceiling is neither cash nor a price.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllowanceTerms {
    pub price_version: PriceVersion,
    pub supplier_allowance: Money,
}

/// A supplied version change effective at the current paid-through boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledDowngrade {
    pub effective_from: u64,
    pub terms: AllowanceTerms,
}

/// Caller-bound current state. Old ledger grants and reservations remain native-owned;
/// replacing `current_grant` never erases their usage or Reserved/Unknown exposure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllowanceState {
    pub subscription: SubscriptionId,
    pub revision: u64,
    pub subscription_state: SubscriptionState,
    pub current_terms: AllowanceTerms,
    pub current_grant: Option<AllowanceGrantId>,
    pub scheduled_downgrade: Option<ScheduledDowngrade>,
}

/// Request and verified lookup identity are checked separately to reject wrong receipts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllowanceRenewal {
    pub expected_revision: u64,
    pub grant: AllowanceGrantId,
}

/// Provenance remains the native owner's assertion; this policy performs no verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllowancePaymentObservation {
    pub grant: AllowanceGrantId,
    pub subscription: SubscriptionObservation,
}

/// Typed refusal with no state, allowance, or supplier-exposure mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllowanceRefusal {
    StaleRevision,
    WrongSubscription,
    WrongInvoiceObservation,
    WrongPeriodBasis,
    WrongPriceVersion,
    TermsMismatch,
    InvalidState,
    AlreadyScheduled,
    InvalidDowngrade,
    WrongState,
    RevisionOverflow,
    Money(MoneyError),
    Subscription(SubscriptionRefusal),
}

/// Exact proposed allowance addition; native commit and uniqueness issue the grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllowanceGrant {
    pub id: AllowanceGrantId,
    pub supplier_allowance: Money,
}

/// Native owners commit both revisions and grant uniqueness atomically with the ledger
/// and protected journal. Unknown commit is not success or permission to credit again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllowanceTransition {
    pub before_revision: u64,
    pub next: AllowanceState,
    pub grant: Option<AllowanceGrant>,
}

fn validate_state(state: &AllowanceState) -> Result<(), AllowanceRefusal> {
    let currency = state.subscription_state.currency;
    if state.current_terms.supplier_allowance.currency() != currency {
        return Err(AllowanceRefusal::Money(MoneyError::CurrencyMismatch));
    }
    if matches!(
        state.subscription_state.access,
        Access::PendingInitial | Access::NoAccess
    ) && state.current_grant.is_some()
    {
        return Err(AllowanceRefusal::InvalidState);
    }
    if let Some(grant) = state.current_grant {
        if grant.subscription != state.subscription
            || grant.price_version != state.current_terms.price_version
            || grant.period.start >= grant.period.end
            || grant.period.end != state.subscription_state.paid_through
        {
            return Err(AllowanceRefusal::InvalidState);
        }
    } else if state.subscription_state.paid_through != 0
        || matches!(
            state.subscription_state.access,
            Access::Active | Access::Grace | Access::ReadOnly
        )
    {
        return Err(AllowanceRefusal::InvalidState);
    }
    if let Some(change) = state.scheduled_downgrade {
        if change.terms.supplier_allowance.currency() != currency {
            return Err(AllowanceRefusal::Money(MoneyError::CurrencyMismatch));
        }
        if state.current_grant.is_none()
            || change.effective_from != state.subscription_state.paid_through
            || change.terms.price_version == state.current_terms.price_version
            || change.terms.supplier_allowance.micros()
                > state.current_terms.supplier_allowance.micros()
        {
            return Err(AllowanceRefusal::InvalidState);
        }
    }
    Ok(())
}

/// Selects disclosed effective terms, never a grant or reset. At an unpaid boundary
/// lowered terms still apply; payment uncertainty cannot manufacture renewed allowance.
pub fn effective_allowance_terms(
    state: &AllowanceState,
    now: u64,
) -> Result<AllowanceTerms, AllowanceRefusal> {
    validate_state(state)?;
    Ok(match state.scheduled_downgrade {
        Some(change) if now >= change.effective_from => change.terms,
        _ => state.current_terms,
    })
}

/// Schedules only a supplied non-increasing supplier ceiling for the next boundary.
/// Current payer authorization and disclosure/price qualification remain native-owned.
pub fn propose_allowance_downgrade(
    state: &AllowanceState,
    expected_revision: u64,
    terms: AllowanceTerms,
    now: u64,
) -> Result<AllowanceTransition, AllowanceRefusal> {
    if expected_revision != state.revision {
        return Err(AllowanceRefusal::StaleRevision);
    }
    validate_state(state)?;
    if state.subscription_state.access != Access::Active
        || now >= state.subscription_state.paid_through
        || state.current_grant.is_none()
    {
        return Err(AllowanceRefusal::WrongState);
    }
    if state.scheduled_downgrade.is_some() {
        return Err(AllowanceRefusal::AlreadyScheduled);
    }
    if terms.supplier_allowance.currency() != state.subscription_state.currency {
        return Err(AllowanceRefusal::Money(MoneyError::CurrencyMismatch));
    }
    if terms.price_version == state.current_terms.price_version
        || terms.supplier_allowance.micros() > state.current_terms.supplier_allowance.micros()
    {
        return Err(AllowanceRefusal::InvalidDowngrade);
    }
    let mut next_state = *state;
    next_state.revision = state
        .revision
        .checked_add(1)
        .ok_or(AllowanceRefusal::RevisionOverflow)?;
    next_state.scheduled_downgrade = Some(ScheduledDowngrade {
        effective_from: state.subscription_state.paid_through,
        terms,
    });
    Ok(AllowanceTransition {
        before_revision: state.revision,
        next: next_state,
        grant: None,
    })
}

/// Reuses canonical subscription observation policy. No timestamp or failure alone
/// grants allowance, no overlapping proration is supported, and no liability is released.
pub fn propose_allowance_renewal(
    state: &AllowanceState,
    request: AllowanceRenewal,
    observation: AllowancePaymentObservation,
    now: u64,
) -> Result<AllowanceTransition, AllowanceRefusal> {
    if request.expected_revision != state.revision {
        return Err(AllowanceRefusal::StaleRevision);
    }
    validate_state(state)?;
    if request.grant.subscription != state.subscription {
        return Err(AllowanceRefusal::WrongSubscription);
    }
    if request.grant != observation.grant {
        return Err(AllowanceRefusal::WrongInvoiceObservation);
    }
    let subscription =
        observe_subscription(state.subscription_state, observation.subscription, now)
            .map_err(AllowanceRefusal::Subscription)?;
    let mut next_state = *state;
    next_state.revision = state
        .revision
        .checked_add(1)
        .ok_or(AllowanceRefusal::RevisionOverflow)?;
    next_state.subscription_state = subscription.next;
    let grant = match observation.subscription.outcome {
        PaymentOutcome::Settled(invoice) => {
            if request.grant.period
                != (PaidPeriod {
                    start: invoice.period_start,
                    end: invoice.paid_through,
                })
            {
                return Err(AllowanceRefusal::WrongPeriodBasis);
            }
            let terms = effective_allowance_terms(state, invoice.period_start)?;
            if request.grant.price_version != terms.price_version {
                return Err(AllowanceRefusal::WrongPriceVersion);
            }
            if invoice.selected_allowance != terms.supplier_allowance {
                return Err(AllowanceRefusal::TermsMismatch);
            }
            next_state.current_terms = terms;
            next_state.current_grant = Some(request.grant);
            if state
                .scheduled_downgrade
                .is_some_and(|change| invoice.period_start >= change.effective_from)
            {
                next_state.scheduled_downgrade = None;
            }
            Some(AllowanceGrant {
                id: request.grant,
                supplier_allowance: terms.supplier_allowance,
            })
        }
        PaymentOutcome::InitialFailed | PaymentOutcome::RenewalFailed => None,
        PaymentOutcome::Unknown => {
            return Err(AllowanceRefusal::Subscription(
                SubscriptionRefusal::UnknownPayment,
            ));
        }
    };
    Ok(AllowanceTransition {
        before_revision: state.revision,
        next: next_state,
        grant,
    })
}
