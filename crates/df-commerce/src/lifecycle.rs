//! Quotes and the attempt's dispatch arrows (planning/finance-ledger-research.md 9.3, 11.3): the
//! small flows around admission. A quote is stored, approved once and used once; an attempt moves
//! `Reserved -> DispatchPrepared -> Sent`, each step arming the deadline that would undo it.
//! TODO(FIN-06): the dispatcher-identity checks of 11.4 (`current_dispatcher()`, the worker match) are
//! O1's definers; the pure arrows are here.

use crate::admission::{Route, WorkClass};
use crate::finance_ids::{EventId, QuoteId, SessionKey};
use crate::plan::{ChangeKind, EventKind, FinancePlan, Row, RowChange};
use crate::price_schedule::{PriceVersion, UnitsRefusal, units_cover_route};
use crate::snapshot::{AttemptRow, DispatchState, QuoteRow, QuoteStatus};

/// Why a quote or an attempt step was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LifecycleRefusal {
    Units(UnitsRefusal),
    QuoteNotOpen,
    /// The quote belongs to another session than the one deciding on it.
    QuoteSessionMismatch,
    QuoteExpired,
    /// The attempt is not in the state the step starts from.
    WrongDispatchState(DispatchState),
}

impl From<UnitsRefusal> for LifecycleRefusal {
    fn from(refusal: UnitsRefusal) -> Self {
        Self::Units(refusal)
    }
}

#[derive(Clone, Debug)]
pub struct QuoteRequest {
    pub event_id: EventId,
    pub quote: QuoteId,
    pub session: SessionKey,
    pub route: Route,
    pub work_class: WorkClass,
    pub maximum_units: Vec<(String, u64)>,
    pub estimate_units: Vec<(String, u64)>,
    pub request_digest: [u8; 32],
    pub lifetime_ms: i64,
    pub now_ms: i64,
}

/// Stores a quote: the estimate and the maximum the reservation will hold (9.3).
pub fn plan_quote(
    version: &PriceVersion,
    request: &QuoteRequest,
) -> Result<FinancePlan, LifecycleRefusal> {
    let declared: Vec<&str> = request
        .route
        .declared_categories
        .iter()
        .map(String::as_str)
        .collect();
    units_cover_route(&declared, &request.maximum_units)?;
    let (provider, model) = (
        request.route.provider.as_str(),
        request.route.model.as_str(),
    );
    let maximum = version.retail_maximum(provider, model, &request.maximum_units)?;
    let estimate = version
        .retail_maximum(provider, model, &request.estimate_units)?
        .min(maximum);
    let supplier = version.supplier_maximum(provider, model, &request.maximum_units)?;
    let mut plan = FinancePlan::new(request.event_id, EventKind::QuoteCreated);
    plan.changes.push(RowChange::insert(
        ChangeKind::QuoteInsert,
        Row::new()
            .bytes("quote_id", request.quote.as_bytes())
            .bytes("session_id", request.session.as_bytes())
            .bytes("request_digest", &request.request_digest)
            .bytes("price_version_id", version.id.as_bytes())
            .text("work_class", request.work_class.label())
            .uint("retail_estimate_micro_dollars", estimate)
            .uint("retail_maximum_micro_dollars", maximum)
            .uint("supplier_maximum_micro_dollars", supplier)
            .int("created_at_ms", request.now_ms)
            .int(
                "expires_at_ms",
                request.now_ms.saturating_add(request.lifetime_ms),
            )
            .text("state", "open"),
    ));
    Ok(plan)
}

/// Approves or declines an open, unexpired quote of `session`.
pub fn plan_quote_decision(
    event_id: EventId,
    session: SessionKey,
    quote: &QuoteRow,
    approve: bool,
    now_ms: i64,
) -> Result<FinancePlan, LifecycleRefusal> {
    if quote.session != session {
        return Err(LifecycleRefusal::QuoteSessionMismatch);
    }
    if quote.status != QuoteStatus::Open {
        return Err(LifecycleRefusal::QuoteNotOpen);
    }
    if now_ms >= quote.expires_at_ms {
        return Err(LifecycleRefusal::QuoteExpired);
    }
    let mut plan = FinancePlan::new(event_id, EventKind::QuoteDecided);
    plan.changes.push(RowChange::update(
        ChangeKind::QuoteUpdate,
        Row::new()
            .bytes("quote_id", quote.quote.as_bytes())
            .text("state", if approve { "approved" } else { "declined" }),
        quote.revision,
    ));
    Ok(plan)
}

fn timer(kind: &'static str, attempt: &AttemptRow, due_at_ms: i64) -> RowChange {
    RowChange::insert(
        ChangeKind::WorkItemUpsert,
        Row::new()
            .text("kind", kind)
            .bytes("ref_id", attempt.attempt.as_bytes())
            .int("due_at_ms", due_at_ms),
    )
}

fn clear(kind: &'static str, attempt: &AttemptRow) -> RowChange {
    RowChange::insert(
        ChangeKind::WorkItemComplete,
        Row::new()
            .text("kind", kind)
            .bytes("ref_id", attempt.attempt.as_bytes()),
    )
}

/// The deadlines of 12.3 in milliseconds.
#[derive(Clone, Copy, Debug)]
pub struct Deadlines {
    pub prepared_expiry_ms: i64,
    pub sent_reconcile_ms: i64,
    pub customer_hold_ms: i64,
    pub unknown_resolution_ms: i64,
}

impl Default for Deadlines {
    fn default() -> Self {
        Self {
            prepared_expiry_ms: 120_000,
            sent_reconcile_ms: 300_000,
            customer_hold_ms: 900_000,
            unknown_resolution_ms: 86_400_000,
        }
    }
}

/// `Reserved -> DispatchPrepared`: the runtime claimed the effect (11.3).
pub fn plan_prepare_dispatch(
    event_id: EventId,
    attempt: &AttemptRow,
    deadlines: Deadlines,
    now_ms: i64,
) -> Result<FinancePlan, LifecycleRefusal> {
    if attempt.dispatch != DispatchState::Reserved {
        return Err(LifecycleRefusal::WrongDispatchState(attempt.dispatch));
    }
    let mut plan = FinancePlan::new(event_id, EventKind::AttemptPrepared);
    plan.changes.push(RowChange::update(
        ChangeKind::ProviderAttemptUpdate,
        Row::new()
            .bytes("attempt_id", attempt.attempt.as_bytes())
            .text("dispatch_state", "dispatch_prepared")
            .int("prepared_at_ms", now_ms),
        attempt.revision,
    ));
    plan.changes.push(clear("reserved_expiry", attempt));
    plan.changes.push(timer(
        "prepared_expiry",
        attempt,
        now_ms.saturating_add(deadlines.prepared_expiry_ms),
    ));
    Ok(plan)
}

/// `DispatchPrepared -> Sent`: committed before any byte leaves (O1 10.3). The hold clock starts
/// here (12.1).
pub fn plan_mark_sent(
    event_id: EventId,
    attempt: &AttemptRow,
    dispatcher_instance: [u8; 16],
    deadlines: Deadlines,
    now_ms: i64,
) -> Result<FinancePlan, LifecycleRefusal> {
    if attempt.dispatch != DispatchState::DispatchPrepared {
        return Err(LifecycleRefusal::WrongDispatchState(attempt.dispatch));
    }
    let mut plan = FinancePlan::new(event_id, EventKind::AttemptSent);
    plan.changes.push(RowChange::update(
        ChangeKind::ProviderAttemptUpdate,
        Row::new()
            .bytes("attempt_id", attempt.attempt.as_bytes())
            .text("dispatch_state", "sent")
            .bytes("dispatcher_instance_id", &dispatcher_instance)
            .int("sent_at_ms", now_ms),
        attempt.revision,
    ));
    plan.changes.push(clear("prepared_expiry", attempt));
    plan.changes.push(timer(
        "sent_reconcile",
        attempt,
        now_ms.saturating_add(deadlines.sent_reconcile_ms),
    ));
    plan.changes.push(timer(
        "customer_hold",
        attempt,
        now_ms.saturating_add(deadlines.customer_hold_ms),
    ));
    plan.changes.push(timer(
        "unknown_resolution",
        attempt,
        now_ms.saturating_add(deadlines.unknown_resolution_ms),
    ));
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finance_ids::{AttemptId, ReservationId};

    fn attempt(dispatch: DispatchState) -> AttemptRow {
        AttemptRow {
            attempt: AttemptId::from_array([1; 16]),
            reservation: ReservationId::from_array([2; 16]),
            provider: "openai".into(),
            model: "m".into(),
            supplier_exposure_micro_dollars: 5,
            dispatch,
            customer_held: true,
            supplier_open: true,
            media_asset: None,
            campaign: None,
            revision: 1,
        }
    }

    #[test]
    fn a_quote_is_decided_only_through_its_own_session() {
        let quote = QuoteRow {
            quote: QuoteId::from_array([1; 16]),
            session: SessionKey::from_array([2; 16]),
            request_digest: [0; 32],
            price_version: crate::finance_ids::PriceVersionId::from_array([3; 16]),
            retail_maximum_micro_dollars: 10,
            expires_at_ms: 100,
            status: QuoteStatus::Open,
            revision: 1,
        };
        let event = EventId::from_array([9; 16]);
        assert!(plan_quote_decision(event, quote.session, &quote, true, 0).is_ok());
        assert_eq!(
            plan_quote_decision(event, SessionKey::from_array([8; 16]), &quote, true, 0)
                .unwrap_err(),
            LifecycleRefusal::QuoteSessionMismatch
        );
    }

    #[test]
    fn an_attempt_moves_only_along_the_dispatch_arrows() {
        let event = EventId::from_array([9; 16]);
        assert!(
            plan_prepare_dispatch(
                event,
                &attempt(DispatchState::Reserved),
                Deadlines::default(),
                0
            )
            .is_ok()
        );
        assert_eq!(
            plan_prepare_dispatch(
                event,
                &attempt(DispatchState::Sent),
                Deadlines::default(),
                0
            )
            .unwrap_err(),
            LifecycleRefusal::WrongDispatchState(DispatchState::Sent)
        );
        assert!(
            plan_mark_sent(
                event,
                &attempt(DispatchState::DispatchPrepared),
                [3; 16],
                Deadlines::default(),
                0
            )
            .is_ok()
        );
        assert!(
            plan_mark_sent(
                event,
                &attempt(DispatchState::Reserved),
                [3; 16],
                Deadlines::default(),
                0
            )
            .is_err()
        );
    }
}
