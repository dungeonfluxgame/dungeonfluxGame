use df_types::{Currency, Money, MoneyError};

const RENEWAL_GRACE_S: u64 = 72 * 60 * 60;

/// Supplied subscription policy state, independent of gameplay already admitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    PendingInitial,
    Active,
    Grace,
    ReadOnly,
    NoAccess,
    Restricted,
}

/// Caller-bound snapshot for one subscription; no ID or tenant authority is issued.
/// Times are authoritative UTC seconds. Object revision is the native owner's
/// monotonic latest-object refresh revision, never webhook delivery ordering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubscriptionState {
    pub currency: Currency,
    pub revision: u64,
    pub object_revision: u64,
    pub access: Access,
    pub paid_through: u64,
    pub grace_until: Option<u64>,
}

/// Caller assertion about observation provenance, not a verification mechanism.
/// A signature-verified webhook alone still requires authoritative object refresh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationAuthority {
    VerifiedLatest,
    WebhookOnly,
    Unverified,
}

/// Supplied settled invoice and selected supplier allowance in exact micro-units.
/// The native owner binds subscription, invoice, period and selected price version,
/// and checks the unique allowance identity in the same transaction as publication.
/// Overlapping upgrade/proration periods require separate qualified policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaidInvoice {
    pub period_start: u64,
    pub paid_through: u64,
    pub selected_allowance: Money,
    pub allowance_already_recorded: bool,
}

/// Known invoice outcomes; Unknown cannot create access or allowance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaymentOutcome {
    Settled(PaidInvoice),
    InitialFailed,
    RenewalFailed,
    Unknown,
}

/// One caller-bound latest-object observation with expected local state revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubscriptionObservation {
    pub authority: ObservationAuthority,
    pub expected_revision: u64,
    pub object_revision: u64,
    pub outcome: PaymentOutcome,
}

/// Refusal produces no candidate, credit, grant or state effects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionRefusal {
    Unverified,
    NeedsAuthoritativeRefresh,
    StaleRevision,
    StaleObject,
    UnknownPayment,
    WrongState,
    InvalidPeriod,
    DuplicateAllowance,
    Money(MoneyError),
    RevisionOverflow,
    TimeOverflow,
}

/// A proposal only. `allowance` is the exact once-only candidate allowance addition,
/// not cash, earned revenue or a published entitlement. Failure does not mutate inputs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubscriptionTransition {
    pub before_revision: u64,
    pub next: SubscriptionState,
    pub allowance: Option<Money>,
}

/// Proposes initial settlement, renewal settlement/failure or initial failure.
///
/// Verification and local/object revision checks precede policy. Grace ends exactly
/// 72 hours after paid-through and repeated failure cannot extend it. Restricted and
/// NoAccess cannot be reactivated through this bounded policy. All updates are local
/// values; the native transaction owner must revalidate before committing.
pub fn observe_subscription(
    state: SubscriptionState,
    observation: SubscriptionObservation,
    now: u64,
) -> Result<SubscriptionTransition, SubscriptionRefusal> {
    match observation.authority {
        ObservationAuthority::Unverified => return Err(SubscriptionRefusal::Unverified),
        ObservationAuthority::WebhookOnly => {
            return Err(SubscriptionRefusal::NeedsAuthoritativeRefresh);
        }
        ObservationAuthority::VerifiedLatest => {}
    }
    if observation.expected_revision != state.revision {
        return Err(SubscriptionRefusal::StaleRevision);
    }
    if observation.object_revision <= state.object_revision {
        return Err(SubscriptionRefusal::StaleObject);
    }
    let mut next = state;
    let allowance = match observation.outcome {
        PaymentOutcome::Unknown => return Err(SubscriptionRefusal::UnknownPayment),
        PaymentOutcome::Settled(invoice) => {
            if matches!(state.access, Access::NoAccess | Access::Restricted) {
                return Err(SubscriptionRefusal::WrongState);
            }
            if invoice.selected_allowance.currency() != state.currency {
                return Err(SubscriptionRefusal::Money(MoneyError::CurrencyMismatch));
            }
            if invoice.period_start >= invoice.paid_through
                || invoice.period_start > now
                || invoice.paid_through <= now
                || invoice.paid_through <= state.paid_through
                || (state.access != Access::PendingInitial
                    && invoice.period_start < state.paid_through)
            {
                return Err(SubscriptionRefusal::InvalidPeriod);
            }
            if invoice.allowance_already_recorded {
                return Err(SubscriptionRefusal::DuplicateAllowance);
            }
            next.access = Access::Active;
            next.paid_through = invoice.paid_through;
            next.grace_until = None;
            Some(invoice.selected_allowance)
        }
        PaymentOutcome::InitialFailed => {
            if state.access != Access::PendingInitial {
                return Err(SubscriptionRefusal::WrongState);
            }
            next.access = Access::NoAccess;
            None
        }
        PaymentOutcome::RenewalFailed => {
            if !matches!(
                state.access,
                Access::Active | Access::Grace | Access::ReadOnly
            ) {
                return Err(SubscriptionRefusal::WrongState);
            }
            if now < state.paid_through {
                return Err(SubscriptionRefusal::InvalidPeriod);
            }
            let grace_until = state
                .paid_through
                .checked_add(RENEWAL_GRACE_S)
                .ok_or(SubscriptionRefusal::TimeOverflow)?;
            next.access = if now < grace_until {
                Access::Grace
            } else {
                Access::ReadOnly
            };
            next.grace_until = Some(grace_until);
            None
        }
    };
    next.revision = state
        .revision
        .checked_add(1)
        .ok_or(SubscriptionRefusal::RevisionOverflow)?;
    next.object_revision = observation.object_revision;
    Ok(SubscriptionTransition {
        before_revision: state.revision,
        next,
        allowance,
    })
}

/// Computes bounded current access without publishing a grant or refreshing allowance.
/// Active ends at paid-through. Malformed, missing, or extended grace deadlines yield
/// ReadOnly; only the exact 72-hour boundary can represent Grace. Grace never permits
/// new optional paid video; even Active still needs separate rights/consent/admission.
pub fn effective_access(state: SubscriptionState, now: u64) -> Access {
    match state.access {
        Access::Active if now >= state.paid_through => Access::ReadOnly,
        Access::Grace => {
            let grace_deadline_s = state.paid_through.checked_add(RENEWAL_GRACE_S);
            match (grace_deadline_s, state.grace_until) {
                (Some(expected), Some(actual_deadline_s))
                    if expected == actual_deadline_s
                        && now >= state.paid_through
                        && now < actual_deadline_s =>
                {
                    Access::Grace
                }
                _ => Access::ReadOnly,
            }
        }
        access => access,
    }
}
