use df_types::{Currency, Money, MoneyError, OperationId};

use crate::ObservationAuthority;

/// Caller-bound credit principal projection, not physical cash, profit or an issued wallet.
/// All values use one currency. Topup principal equals deferred + earned + refunded principal;
/// refund holds are a subset of deferred principal. Taxes, fees and supplier ledgers are separate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreditState {
    pub currency: Currency,
    pub revision: u64,
    pub topup_principal: Money,
    pub deferred_credit: Money,
    pub earned_credit: Money,
    pub refunded_principal: Money,
    pub refund_hold: Money,
}

/// An outstanding refund keeps its entire hold until conclusive same-key reconciliation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefundStatus {
    Pending,
    Unknown,
}

/// Supplied current obligation for the exact refund operation, not a gateway receipt.
/// Object revisions come from authoritative latest-object refresh, never webhook order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefundObligation {
    pub operation: OperationId,
    pub held: Money,
    pub object_revision: u64,
    pub status: RefundStatus,
}

/// Known failure means verified conclusive nonpayment, never timeout or missing evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefundOutcome {
    KnownSuccess,
    KnownFailure,
    Unknown,
}

/// Supplied eligible actions. Amounts, payment verification, refund eligibility and
/// consumption/reversal terms remain native-owned; no expiry or refund rate is selected here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditEvent {
    SettledTopup {
        amount: Money,
    },
    UnknownTopup,
    Consume {
        amount: Money,
    },
    ReverseConsumption {
        original_operation: OperationId,
        amount: Money,
    },
    HoldRefund {
        amount: Money,
    },
    ObserveRefund {
        refund_operation: OperationId,
        held: Money,
        object_revision: u64,
        outcome: RefundOutcome,
    },
}

/// Exact credit command and fingerprint for supplied durable lookup.
/// OperationId identifies this command, not a tenant, payment, invoice or ledger row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreditReceipt {
    pub operation: OperationId,
    pub event: CreditEvent,
}

/// Remaining reversible principal of the exact original consumption operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OriginalCharge {
    pub operation: OperationId,
    pub remaining_reversible: Money,
}

/// Native-supplied current facts. VerifiedLatest is an assertion, not verification.
/// Receipt absence requires complete trusted operation/source-payment lookup and journal
/// reconciliation. An omitted receipt after timeout/restore is not evidence of freshness.
/// The native owner binds tenant, source payment, original charge, refund, approved terms
/// and source uniqueness; a second command key cannot make one payment credit twice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreditObservation {
    pub authority: ObservationAuthority,
    pub expected_revision: u64,
    pub prior_receipt: Option<CreditReceipt>,
    pub refund: Option<RefundObligation>,
    pub reversible_charge: Option<OriginalCharge>,
}

/// Typed refusal produces no credit, refund, income, state mutation or external send.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditRefusal {
    Unverified,
    NeedsAuthoritativeRefresh,
    StaleRevision,
    AlreadyRecorded,
    ConflictingOperation,
    WrongReceipt,
    InvalidLedger,
    ZeroAmount,
    InsufficientAvailableCredit,
    MissingOriginalCharge,
    WrongOriginalCharge,
    ExceedsOriginalCharge,
    MissingRefund,
    WrongRefund,
    StaleRefundObservation,
    UnknownPayment,
    RevisionOverflow,
    Money(MoneyError),
}

impl From<MoneyError> for CreditRefusal {
    fn from(error: MoneyError) -> Self {
        Self::Money(error)
    }
}

/// One append-only semantic adjustment; this is not a canonical persisted LedgerEntry.
/// Successful/failed refunds retire the named outstanding obligation in the native commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditAdjustment {
    TopupDeferred {
        amount: Money,
    },
    ConsumptionEarned {
        amount: Money,
    },
    ConsumptionReversed {
        original_operation: OperationId,
        amount: Money,
    },
    RefundHeld(RefundObligation),
    RefundUnknown(RefundObligation),
    RefundPaid {
        operation: OperationId,
        amount: Money,
    },
    RefundFailed {
        operation: OperationId,
        released_hold: Money,
    },
}

/// Bounded candidate only. Native ownership atomically revalidates revision, receipt/source
/// uniqueness and exact obligation/original-charge facts, appends ledger adjustments and
/// confirms protected recovery journal publication before exposing credit or sending refunds.
/// This proposal never changes supplier spend, reservations or Unknown supplier liabilities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreditTransition {
    pub before_revision: u64,
    pub next: CreditState,
    pub receipt: CreditReceipt,
    pub adjustment: CreditAdjustment,
}

fn validate_state(state: CreditState) -> Result<(), CreditRefusal> {
    let zero = Money::new(state.currency, 0);
    for amount in [
        state.topup_principal,
        state.deferred_credit,
        state.earned_credit,
        state.refunded_principal,
        state.refund_hold,
    ] {
        zero.checked_add(amount)?;
    }
    let allocated = state
        .deferred_credit
        .checked_add(state.earned_credit)?
        .checked_add(state.refunded_principal)?;
    if allocated != state.topup_principal
        || state.refund_hold.micros() > state.deferred_credit.micros()
    {
        return Err(CreditRefusal::InvalidLedger);
    }
    Ok(())
}

fn require_amount(state: CreditState, amount: Money) -> Result<(), CreditRefusal> {
    Money::new(state.currency, 0).checked_add(amount)?;
    if amount.micros() == 0 {
        return Err(CreditRefusal::ZeroAmount);
    }
    Ok(())
}

fn require_available(state: CreditState, amount: Money) -> Result<(), CreditRefusal> {
    require_amount(state, amount)?;
    let available = state.deferred_credit.checked_sub(state.refund_hold)?;
    if amount.micros() > available.micros() {
        return Err(CreditRefusal::InsufficientAvailableCredit);
    }
    Ok(())
}

/// Proposes exact principal accounting from supplied verified current facts.
/// A topup remains fully deferred until verified consumption. Refund uncertainty keeps the
/// full held principal unavailable; only known reconciliation can pay or release that hold.
/// Exact receipt replay returns AlreadyRecorded for native lookup of its original result.
/// Arithmetic/state/order errors leave the input unchanged. No clock, provider, auth,
/// storage, telemetry SDK or implicit currency conversion is involved.
pub fn propose_credit_adjustment(
    state: CreditState,
    receipt: CreditReceipt,
    observation: CreditObservation,
) -> Result<CreditTransition, CreditRefusal> {
    match observation.authority {
        ObservationAuthority::Unverified => return Err(CreditRefusal::Unverified),
        ObservationAuthority::WebhookOnly => {
            return Err(CreditRefusal::NeedsAuthoritativeRefresh);
        }
        ObservationAuthority::VerifiedLatest => {}
    }
    if observation.expected_revision != state.revision {
        return Err(CreditRefusal::StaleRevision);
    }
    validate_state(state)?;
    if let Some(prior) = observation.prior_receipt {
        if prior.operation != receipt.operation {
            return Err(CreditRefusal::WrongReceipt);
        }
        return Err(if prior == receipt {
            CreditRefusal::AlreadyRecorded
        } else {
            CreditRefusal::ConflictingOperation
        });
    }
    let mut next_state = state;
    let adjustment = match receipt.event {
        CreditEvent::UnknownTopup => return Err(CreditRefusal::UnknownPayment),
        CreditEvent::SettledTopup { amount } => {
            require_amount(state, amount)?;
            next_state.topup_principal = state.topup_principal.checked_add(amount)?;
            next_state.deferred_credit = state.deferred_credit.checked_add(amount)?;
            CreditAdjustment::TopupDeferred { amount }
        }
        CreditEvent::Consume { amount } => {
            require_available(state, amount)?;
            next_state.deferred_credit = state.deferred_credit.checked_sub(amount)?;
            next_state.earned_credit = state.earned_credit.checked_add(amount)?;
            CreditAdjustment::ConsumptionEarned { amount }
        }
        CreditEvent::ReverseConsumption {
            original_operation,
            amount,
        } => {
            require_amount(state, amount)?;
            let original = observation
                .reversible_charge
                .ok_or(CreditRefusal::MissingOriginalCharge)?;
            if original.operation != original_operation {
                return Err(CreditRefusal::WrongOriginalCharge);
            }
            Money::new(state.currency, 0).checked_add(original.remaining_reversible)?;
            if amount.micros() > original.remaining_reversible.micros() {
                return Err(CreditRefusal::ExceedsOriginalCharge);
            }
            next_state.earned_credit = state.earned_credit.checked_sub(amount)?;
            next_state.deferred_credit = state.deferred_credit.checked_add(amount)?;
            CreditAdjustment::ConsumptionReversed {
                original_operation,
                amount,
            }
        }
        CreditEvent::HoldRefund { amount } => {
            require_available(state, amount)?;
            if observation.refund.is_some() {
                return Err(CreditRefusal::WrongRefund);
            }
            next_state.refund_hold = state.refund_hold.checked_add(amount)?;
            CreditAdjustment::RefundHeld(RefundObligation {
                operation: receipt.operation,
                held: amount,
                object_revision: 0,
                status: RefundStatus::Pending,
            })
        }
        CreditEvent::ObserveRefund {
            refund_operation,
            held,
            object_revision,
            outcome,
        } => {
            require_amount(state, held)?;
            let refund = observation.refund.ok_or(CreditRefusal::MissingRefund)?;
            require_amount(state, refund.held)?;
            if refund.operation != refund_operation
                || refund.held != held
                || held.micros() > state.refund_hold.micros()
            {
                return Err(CreditRefusal::WrongRefund);
            }
            if object_revision <= refund.object_revision {
                return Err(CreditRefusal::StaleRefundObservation);
            }
            match outcome {
                RefundOutcome::Unknown => CreditAdjustment::RefundUnknown(RefundObligation {
                    object_revision,
                    status: RefundStatus::Unknown,
                    ..refund
                }),
                RefundOutcome::KnownSuccess => {
                    next_state.refund_hold = state.refund_hold.checked_sub(held)?;
                    next_state.deferred_credit = state.deferred_credit.checked_sub(held)?;
                    next_state.refunded_principal = state.refunded_principal.checked_add(held)?;
                    CreditAdjustment::RefundPaid {
                        operation: refund_operation,
                        amount: held,
                    }
                }
                RefundOutcome::KnownFailure => {
                    next_state.refund_hold = state.refund_hold.checked_sub(held)?;
                    CreditAdjustment::RefundFailed {
                        operation: refund_operation,
                        released_hold: held,
                    }
                }
            }
        }
    };
    next_state.revision = state
        .revision
        .checked_add(1)
        .ok_or(CreditRefusal::RevisionOverflow)?;
    validate_state(next_state)?;
    Ok(CreditTransition {
        before_revision: state.revision,
        next: next_state,
        receipt,
        adjustment,
    })
}
