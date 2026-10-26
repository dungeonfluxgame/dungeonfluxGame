//! The finance plan: what a flow decides, as data the store applies (planning/finance-ledger-
//! research.md 17.6). A plan holds the journal entries and the typed row changes of one event;
//! the database boundary checks it again (allowlist, equations, compare-and-set), so a defect here
//! cannot mint credit.

use std::collections::BTreeMap;

use crate::finance_ids::{EventId, hex};
use crate::ledger::{AccountKey, EntryRefusal, JournalEntry};

/// A column value of a row image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Null,
    Bytes(Vec<u8>),
    Text(String),
    Int(i64),
    Bool(bool),
    /// A JSON document stored as text, for `jsonb` columns.
    Json(String),
}

/// A row image: column name to value.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Row(pub BTreeMap<&'static str, Value>);

impl Row {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bytes(mut self, column: &'static str, value: &[u8]) -> Self {
        self.0.insert(column, Value::Bytes(value.to_vec()));
        self
    }

    pub fn opt_bytes(mut self, column: &'static str, value: Option<&[u8]>) -> Self {
        self.0.insert(
            column,
            value.map_or(Value::Null, |bytes| Value::Bytes(bytes.to_vec())),
        );
        self
    }

    pub fn text(mut self, column: &'static str, value: &str) -> Self {
        self.0.insert(column, Value::Text(value.to_owned()));
        self
    }

    pub fn opt_text(mut self, column: &'static str, value: Option<&str>) -> Self {
        self.0.insert(
            column,
            value.map_or(Value::Null, |text| Value::Text(text.to_owned())),
        );
        self
    }

    pub fn int(mut self, column: &'static str, value: i64) -> Self {
        self.0.insert(column, Value::Int(value));
        self
    }

    pub fn uint(self, column: &'static str, value: u64) -> Self {
        self.int(column, i64::try_from(value).unwrap_or(i64::MAX))
    }

    pub fn opt_int(mut self, column: &'static str, value: Option<i64>) -> Self {
        self.0.insert(column, value.map_or(Value::Null, Value::Int));
        self
    }

    pub fn boolean(mut self, column: &'static str, value: bool) -> Self {
        self.0.insert(column, Value::Bool(value));
        self
    }

    pub fn json(mut self, column: &'static str, value: &str) -> Self {
        self.0.insert(column, Value::Json(value.to_owned()));
        self
    }

    pub fn get(&self, column: &str) -> Option<&Value> {
        self.0.get(column)
    }

    pub fn get_int(&self, column: &str) -> Option<i64> {
        match self.0.get(column) {
            Some(Value::Int(value)) => Some(*value),
            _ => None,
        }
    }
}

/// The closed kinds of event (17.6): every writer of a finance table is one of these.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    PayerRegistered,
    PriceVersionPublished,
    SessionFundingOpened,
    TrialGranted,
    PurchaseApplied,
    AllocationMade,
    AdmissionAdmitted,
    SettlementApplied,
    ReleaseApplied,
    CreditApplied,
    QuoteCreated,
    QuoteDecided,
    AttemptPrepared,
    AttemptSent,
    UsageRecorded,
    RefundRequested,
    RefundPaid,
    RefundUnknown,
    RefundFailed,
    RefundReturned,
    DisputeOpened,
    DisputeClosed,
    InvalidOutputSettled,
}

impl EventKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::PayerRegistered => "payer_registered",
            Self::PriceVersionPublished => "price_version_published",
            Self::SessionFundingOpened => "session_funding_opened",
            Self::TrialGranted => "trial_granted",
            Self::PurchaseApplied => "purchase_applied",
            Self::AllocationMade => "allocation_made",
            Self::AdmissionAdmitted => "admission_admitted",
            Self::SettlementApplied => "settlement_applied",
            Self::ReleaseApplied => "release_applied",
            Self::CreditApplied => "credit_applied",
            Self::QuoteCreated => "quote_created",
            Self::QuoteDecided => "quote_decided",
            Self::AttemptPrepared => "attempt_prepared",
            Self::AttemptSent => "attempt_sent",
            Self::UsageRecorded => "usage_recorded",
            Self::RefundRequested => "refund_requested",
            Self::RefundPaid => "refund_paid",
            Self::RefundUnknown => "refund_unknown",
            Self::RefundFailed => "refund_failed",
            Self::RefundReturned => "refund_returned",
            Self::DisputeOpened => "dispute_opened",
            Self::DisputeClosed => "dispute_closed",
            Self::InvalidOutputSettled => "invalid_output_settled",
        }
    }
}

/// The closed kinds of row change, mirroring `finance.change_kind`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    KeyCreate,
    PayerInsert,
    FundingLotInsert,
    FundingLotUpdate,
    ProcessorPaymentStateInsert,
    TrialGrantInsert,
    PriceVersionInsert,
    PriceVersionTariffInsert,
    SessionFundingInsert,
    SessionFundingUpdate,
    SessionChargedTotalUpdate,
    FundingEpochInsert,
    EpochShareInsert,
    AllocationInsert,
    AllocationUpdate,
    AllocationLotInsert,
    AllocationConsentInsert,
    QuoteInsert,
    QuoteUpdate,
    ReservationInsert,
    ReservationUpdate,
    ReservationShareInsert,
    ReservationShareUpdate,
    ProviderAttemptInsert,
    ProviderAttemptUpdate,
    UsageEventInsert,
    UsageEventUpdate,
    UsageReversalInsert,
    OperationReceiptInsert,
    SettlementFactInsert,
    AttemptOutcomeEventInsert,
    CompanyLossInsert,
    ForgoneRevenueInsert,
    MeterSegmentPricedInsert,
    WorkItemUpsert,
    WorkItemComplete,
    RefundInsert,
    RefundUpdate,
    RefundLotInsert,
    RefundLotUpdate,
    RefundOutboxInsert,
    RefundOutboxUpdate,
    DisputeInsert,
    DisputeUpdate,
    ProcessorPaymentStateUpdate,
    FundingLotRefundUpdate,
    FundingLotDisputeUpdate,
    ProviderAttemptOutcomeUpdate,
}

impl ChangeKind {
    /// Every kind, for the test that compares them with `finance.change_kind`.
    pub const ALL: [Self; 48] = [
        Self::KeyCreate,
        Self::PayerInsert,
        Self::FundingLotInsert,
        Self::FundingLotUpdate,
        Self::ProcessorPaymentStateInsert,
        Self::TrialGrantInsert,
        Self::PriceVersionInsert,
        Self::PriceVersionTariffInsert,
        Self::SessionFundingInsert,
        Self::SessionFundingUpdate,
        Self::SessionChargedTotalUpdate,
        Self::FundingEpochInsert,
        Self::EpochShareInsert,
        Self::AllocationInsert,
        Self::AllocationUpdate,
        Self::AllocationLotInsert,
        Self::AllocationConsentInsert,
        Self::QuoteInsert,
        Self::QuoteUpdate,
        Self::ReservationInsert,
        Self::ReservationUpdate,
        Self::ReservationShareInsert,
        Self::ReservationShareUpdate,
        Self::ProviderAttemptInsert,
        Self::ProviderAttemptUpdate,
        Self::UsageEventInsert,
        Self::UsageEventUpdate,
        Self::UsageReversalInsert,
        Self::OperationReceiptInsert,
        Self::SettlementFactInsert,
        Self::AttemptOutcomeEventInsert,
        Self::CompanyLossInsert,
        Self::ForgoneRevenueInsert,
        Self::MeterSegmentPricedInsert,
        Self::WorkItemUpsert,
        Self::WorkItemComplete,
        Self::RefundInsert,
        Self::RefundUpdate,
        Self::RefundLotInsert,
        Self::RefundLotUpdate,
        Self::RefundOutboxInsert,
        Self::RefundOutboxUpdate,
        Self::DisputeInsert,
        Self::DisputeUpdate,
        Self::ProcessorPaymentStateUpdate,
        Self::FundingLotRefundUpdate,
        Self::FundingLotDisputeUpdate,
        Self::ProviderAttemptOutcomeUpdate,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::KeyCreate => "key_create",
            Self::PayerInsert => "payer_insert",
            Self::FundingLotInsert => "funding_lot_insert",
            Self::FundingLotUpdate => "funding_lot_update",
            Self::ProcessorPaymentStateInsert => "processor_payment_state_insert",
            Self::TrialGrantInsert => "trial_grant_insert",
            Self::PriceVersionInsert => "price_version_insert",
            Self::PriceVersionTariffInsert => "price_version_tariff_insert",
            Self::SessionFundingInsert => "session_funding_insert",
            Self::SessionFundingUpdate => "session_funding_update",
            Self::SessionChargedTotalUpdate => "session_charged_total_update",
            Self::FundingEpochInsert => "funding_epoch_insert",
            Self::EpochShareInsert => "epoch_share_insert",
            Self::AllocationInsert => "allocation_insert",
            Self::AllocationUpdate => "allocation_update",
            Self::AllocationLotInsert => "allocation_lot_insert",
            Self::AllocationConsentInsert => "allocation_consent_insert",
            Self::QuoteInsert => "quote_insert",
            Self::QuoteUpdate => "quote_update",
            Self::ReservationInsert => "reservation_insert",
            Self::ReservationUpdate => "reservation_update",
            Self::ReservationShareInsert => "reservation_share_insert",
            Self::ReservationShareUpdate => "reservation_share_update",
            Self::ProviderAttemptInsert => "provider_attempt_insert",
            Self::ProviderAttemptUpdate => "provider_attempt_update",
            Self::UsageEventInsert => "usage_event_insert",
            Self::UsageEventUpdate => "usage_event_update",
            Self::UsageReversalInsert => "usage_reversal_insert",
            Self::OperationReceiptInsert => "operation_receipt_insert",
            Self::SettlementFactInsert => "settlement_fact_insert",
            Self::AttemptOutcomeEventInsert => "attempt_outcome_event_insert",
            Self::CompanyLossInsert => "company_loss_insert",
            Self::ForgoneRevenueInsert => "forgone_revenue_insert",
            Self::MeterSegmentPricedInsert => "meter_segment_priced_insert",
            Self::WorkItemUpsert => "work_item_upsert",
            Self::WorkItemComplete => "work_item_complete",
            Self::RefundInsert => "refund_insert",
            Self::RefundUpdate => "refund_update",
            Self::RefundLotInsert => "refund_lot_insert",
            Self::RefundLotUpdate => "refund_lot_update",
            Self::RefundOutboxInsert => "refund_outbox_insert",
            Self::RefundOutboxUpdate => "refund_outbox_update",
            Self::DisputeInsert => "dispute_insert",
            Self::DisputeUpdate => "dispute_update",
            Self::ProcessorPaymentStateUpdate => "processor_payment_state_update",
            Self::FundingLotRefundUpdate => "funding_lot_refund_update",
            Self::FundingLotDisputeUpdate => "funding_lot_dispute_update",
            Self::ProviderAttemptOutcomeUpdate => "provider_attempt_outcome_update",
        }
    }
}

/// One typed row change. An update carries the revision the plan was computed from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowChange {
    pub kind: ChangeKind,
    pub row: Row,
    pub expected_revision: Option<i64>,
}

impl RowChange {
    pub fn insert(kind: ChangeKind, row: Row) -> Self {
        Self {
            kind,
            row,
            expected_revision: None,
        }
    }

    pub fn update(kind: ChangeKind, row: Row, expected_revision: i64) -> Self {
        Self {
            kind,
            row,
            expected_revision: Some(expected_revision),
        }
    }

    /// Creates an account key (idempotent); the database trigger checks its class and currency.
    pub fn key(key: &AccountKey) -> Self {
        let dims = &key.dims;
        let row = Row::new()
            .int("code", i64::from(key.code.number()))
            .text("currency", key.currency.as_str())
            .opt_bytes(
                "payer",
                dims.payer.as_ref().map(|id| id.as_bytes().as_slice()),
            )
            .opt_bytes("lot", dims.lot.as_ref().map(|id| id.as_bytes().as_slice()))
            .opt_bytes(
                "allocation",
                dims.allocation.as_ref().map(|id| id.as_bytes().as_slice()),
            )
            .opt_text("provider", dims.provider.as_deref())
            .opt_text("processor", dims.processor.as_deref())
            .opt_text("loss_class", dims.loss_class.as_deref())
            .opt_text("jurisdiction", dims.jurisdiction.as_deref())
            .opt_text("promo_source", dims.promo_source.as_deref());
        Self::insert(ChangeKind::KeyCreate, row)
    }
}

/// Why a plan was refused before it reached the database.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanRefusal {
    Entry(usize, EntryRefusal),
    EntryCarryingEventWithoutEntries,
}

/// One event: entries and row changes, applied atomically by `finance.commit_event`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinancePlan {
    pub event_id: EventId,
    pub event_kind: EventKind,
    pub entries: Vec<JournalEntry>,
    pub changes: Vec<RowChange>,
}

impl FinancePlan {
    pub fn new(event_id: EventId, event_kind: EventKind) -> Self {
        Self {
            event_id,
            event_kind,
            entries: Vec::new(),
            changes: Vec::new(),
        }
    }

    /// Every entry is balanced and on its kind's allowlist.
    pub fn check(&self) -> Result<(), PlanRefusal> {
        for (index, entry) in self.entries.iter().enumerate() {
            entry
                .validate()
                .map_err(|refusal| PlanRefusal::Entry(index, refusal))?;
            crate::entry_rules::check_allowlist(entry)
                .map_err(|refusal| PlanRefusal::Entry(index, refusal))?;
        }
        Ok(())
    }

    /// A short audit description: ids as hex, counts only.
    pub fn describe(&self) -> String {
        format!(
            "event={} kind={} entries={} changes={}",
            &hex(self.event_id.as_bytes())[..8],
            self.event_kind.label(),
            self.entries.len(),
            self.changes.len()
        )
    }
}
