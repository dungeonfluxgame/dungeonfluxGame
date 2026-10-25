//! Bounded, pure commercial policy candidates using caller-supplied exact values.
//!
//! These functions do not verify payment observations, mint grants, reserve money,
//! authenticate tenants, commit a ledger, or authorize dispatch. Native owners must
//! bind the inputs to current trusted records and commit candidates atomically with
//! uniqueness, ledger, grant, accepted-intent and protected recovery-journal changes.
//! Subscription and spend inputs have separate revisions and policy assumptions;
//! neither candidate alone constitutes durable paid-work admission.

mod allowance;
mod credit;
mod ports;
mod spend;
mod subscription;

// The money core (planning/finance-ledger-research.md, F round 1): pure, no I/O, no clock.
pub mod admission;
pub mod digest;
pub mod entry_rules;
pub mod finance_ids;
pub mod funding;
pub mod funding_flows;
pub mod keys;
pub mod ledger;
pub mod lifecycle;
pub mod plan;
pub mod pot;
pub mod price_schedule;
pub mod returns;
pub mod rounding;
pub mod settlement;
pub mod snapshot;
pub mod tariff;
pub mod views;

pub use allowance::{
    AllowanceGrant, AllowanceGrantId, AllowancePaymentObservation, AllowanceRefusal,
    AllowanceRenewal, AllowanceState, AllowanceTerms, AllowanceTransition, PaidPeriod,
    ScheduledDowngrade, effective_allowance_terms, propose_allowance_downgrade,
    propose_allowance_renewal,
};

pub use credit::{
    CreditAdjustment, CreditEvent, CreditObservation, CreditReceipt, CreditRefusal, CreditState,
    CreditTransition, OriginalCharge, RefundObligation, RefundOutcome, RefundStatus,
    propose_credit_adjustment,
};

pub use ports::{
    CommerceCommit, CommerceCommitOutcome, CommerceMutation, CommerceReadOutcome,
    CommerceRepository, CommerceScope, GatewayMutation, GatewayObservation, GatewayOperation,
    GatewayOperationKind, GatewayOperationOutcome, GatewayPaymentObservation,
    GatewayReconciliation, PaymentGateway,
};

pub use spend::{
    SpendConsent, SpendCounter, SpendOperationObservation, SpendOperationStatus, SpendProposal,
    SpendRefusal, SpendRequest, SpendReservation, SpendReservationStatus, SpendScope,
    SpendSettlementObservation, SpendSettlementOutcome, SpendSettlementProposal,
    SpendSettlementRefusal, SpendSnapshot, propose_spend, propose_spend_settlement,
};
pub use subscription::{
    Access, ObservationAuthority, PaidInvoice, PaymentOutcome, SubscriptionObservation,
    SubscriptionRefusal, SubscriptionState, SubscriptionTransition, effective_access,
    observe_subscription,
};
