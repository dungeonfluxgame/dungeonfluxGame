//! The chart of accounts, account keys, journal entries and their canonical hash bytes
//! (planning/finance-ledger-research.md 5, 17.1, 17.7).
//!
//! Three books: General is the ledger; Promo and Exposure are memoranda. Every entry balances in
//! each book and currency separately, so a line cannot cross from one book to another. Signed
//! amounts are micro-dollars: positive is a debit, negative a credit.

use std::collections::BTreeMap;

use crate::finance_ids::{
    AllocationId, AttemptId, CampaignKey, EntryId, EventId, LotId, PayerId, PriceVersionId,
    ReservationId, SessionKey, hex,
};

/// The three books of 5.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Book {
    General,
    Promo,
    Exposure,
}

impl Book {
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Promo => "promo",
            Self::Exposure => "exposure",
        }
    }
}

/// A dimension an account key may carry.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Dimension {
    Payer,
    Lot,
    Allocation,
    Provider,
    Processor,
    LossClass,
    Jurisdiction,
    PromoSource,
}

/// The closed chart of accounts (5).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AccountCode {
    BankOperating,
    ProcessorClearing,
    SupplierPrepaid,
    DisputeFundsWithheld,
    PurchasedAvailable,
    PurchasedAllocated,
    PurchasedReserved,
    PurchasedFrozen,
    SupplierPayable,
    SupplierAccrued,
    TaxPayable,
    RefundPayable,
    OwnerEquity,
    RevenueTableTime,
    RevenueAiUsage,
    RefundsOfConsumedService,
    PromotionalRevenue,
    CostOfServices,
    CostOfServicesPromotional,
    AbsorbedSupplierCost,
    PromotionalCreditExpense,
    ProcessingFees,
    ChargebackLoss,
    Rounding,
    PromotionalAvailable,
    PromotionalAllocated,
    PromotionalReserved,
    PromotionalOffset,
    SupplierExposureOpen,
    SupplierExposureOffset,
}

impl AccountCode {
    pub const ALL: [AccountCode; 30] = [
        Self::BankOperating,
        Self::ProcessorClearing,
        Self::SupplierPrepaid,
        Self::DisputeFundsWithheld,
        Self::PurchasedAvailable,
        Self::PurchasedAllocated,
        Self::PurchasedReserved,
        Self::PurchasedFrozen,
        Self::SupplierPayable,
        Self::SupplierAccrued,
        Self::TaxPayable,
        Self::RefundPayable,
        Self::OwnerEquity,
        Self::RevenueTableTime,
        Self::RevenueAiUsage,
        Self::RefundsOfConsumedService,
        Self::PromotionalRevenue,
        Self::CostOfServices,
        Self::CostOfServicesPromotional,
        Self::AbsorbedSupplierCost,
        Self::PromotionalCreditExpense,
        Self::ProcessingFees,
        Self::ChargebackLoss,
        Self::Rounding,
        Self::PromotionalAvailable,
        Self::PromotionalAllocated,
        Self::PromotionalReserved,
        Self::PromotionalOffset,
        Self::SupplierExposureOpen,
        Self::SupplierExposureOffset,
    ];

    pub fn number(self) -> u16 {
        match self {
            Self::BankOperating => 1000,
            Self::ProcessorClearing => 1100,
            Self::SupplierPrepaid => 1200,
            Self::DisputeFundsWithheld => 1300,
            Self::PurchasedAvailable => 2110,
            Self::PurchasedAllocated => 2120,
            Self::PurchasedReserved => 2130,
            Self::PurchasedFrozen => 2140,
            Self::SupplierPayable => 2300,
            Self::SupplierAccrued => 2310,
            Self::TaxPayable => 2400,
            Self::RefundPayable => 2500,
            Self::OwnerEquity => 3000,
            Self::RevenueTableTime => 4100,
            Self::RevenueAiUsage => 4110,
            Self::RefundsOfConsumedService => 4190,
            Self::PromotionalRevenue => 4200,
            Self::CostOfServices => 5100,
            Self::CostOfServicesPromotional => 5110,
            Self::AbsorbedSupplierCost => 5200,
            Self::PromotionalCreditExpense => 5300,
            Self::ProcessingFees => 5400,
            Self::ChargebackLoss => 5500,
            Self::Rounding => 5700,
            Self::PromotionalAvailable => 2210,
            Self::PromotionalAllocated => 2220,
            Self::PromotionalReserved => 2230,
            Self::PromotionalOffset => 3910,
            Self::SupplierExposureOpen => 9100,
            Self::SupplierExposureOffset => 9190,
        }
    }

    pub fn from_number(number: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|code| code.number() == number)
    }

    pub fn book(self) -> Book {
        match self.number() {
            2210 | 2220 | 2230 | 3910 => Book::Promo,
            9100 | 9190 => Book::Exposure,
            _ => Book::General,
        }
    }

    /// `1` for a debit-normal account, `-1` for a credit-normal one.
    pub fn normal_side(self) -> i64 {
        match self {
            Self::BankOperating
            | Self::ProcessorClearing
            | Self::SupplierPrepaid
            | Self::DisputeFundsWithheld
            | Self::RefundsOfConsumedService
            | Self::CostOfServices
            | Self::CostOfServicesPromotional
            | Self::AbsorbedSupplierCost
            | Self::PromotionalCreditExpense
            | Self::ProcessingFees
            | Self::ChargebackLoss
            | Self::Rounding
            | Self::PromotionalOffset
            | Self::SupplierExposureOpen => 1,
            _ => -1,
        }
    }

    /// Enforced accounts keep a balance row that can never go below zero (5).
    pub fn enforced(self) -> bool {
        matches!(
            self,
            Self::PurchasedAvailable
                | Self::PurchasedAllocated
                | Self::PurchasedReserved
                | Self::PurchasedFrozen
                | Self::RefundPayable
                | Self::PromotionalAvailable
                | Self::PromotionalAllocated
                | Self::PromotionalReserved
        )
    }

    /// The dimensions a key of this code must carry, and no others.
    pub fn dimensions(self) -> &'static [Dimension] {
        use Dimension::{Allocation, Jurisdiction, Lot, Payer, Processor, PromoSource, Provider};
        match self {
            Self::BankOperating
            | Self::OwnerEquity
            | Self::RevenueTableTime
            | Self::RevenueAiUsage
            | Self::RefundsOfConsumedService
            | Self::Rounding => &[],
            Self::ProcessorClearing
            | Self::DisputeFundsWithheld
            | Self::ProcessingFees
            | Self::ChargebackLoss => &[Processor],
            Self::SupplierPrepaid
            | Self::SupplierPayable
            | Self::SupplierAccrued
            | Self::CostOfServices
            | Self::CostOfServicesPromotional
            | Self::SupplierExposureOpen
            | Self::SupplierExposureOffset => &[Provider],
            Self::PurchasedAvailable | Self::PromotionalAvailable => &[Payer, Lot],
            Self::PurchasedAllocated
            | Self::PurchasedReserved
            | Self::PromotionalAllocated
            | Self::PromotionalReserved => &[Allocation, Lot],
            Self::PurchasedFrozen | Self::RefundPayable => &[Lot],
            Self::TaxPayable => &[Jurisdiction],
            Self::PromotionalRevenue | Self::PromotionalCreditExpense | Self::PromotionalOffset => {
                &[PromoSource]
            }
            Self::AbsorbedSupplierCost => &[Provider, Dimension::LossClass],
        }
    }
}

/// A three-letter currency tag.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CurrencyTag([u8; 3]);

impl CurrencyTag {
    pub const USD: CurrencyTag = CurrencyTag(*b"USD");

    pub fn parse(text: &str) -> Option<Self> {
        let bytes: [u8; 3] = text.as_bytes().try_into().ok()?;
        bytes
            .iter()
            .all(u8::is_ascii_uppercase)
            .then_some(Self(bytes))
    }

    pub fn as_str(&self) -> &str {
        // The constructors admit only ASCII uppercase letters.
        std::str::from_utf8(&self.0).unwrap_or("???")
    }
}

/// The dimension values of an account key.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Dimensions {
    pub payer: Option<PayerId>,
    pub lot: Option<LotId>,
    pub allocation: Option<AllocationId>,
    pub provider: Option<String>,
    pub processor: Option<String>,
    pub loss_class: Option<String>,
    pub jurisdiction: Option<String>,
    pub promo_source: Option<String>,
}

impl Dimensions {
    fn present(&self) -> Vec<Dimension> {
        let mut present = Vec::new();
        if self.payer.is_some() {
            present.push(Dimension::Payer);
        }
        if self.lot.is_some() {
            present.push(Dimension::Lot);
        }
        if self.allocation.is_some() {
            present.push(Dimension::Allocation);
        }
        if self.provider.is_some() {
            present.push(Dimension::Provider);
        }
        if self.processor.is_some() {
            present.push(Dimension::Processor);
        }
        if self.loss_class.is_some() {
            present.push(Dimension::LossClass);
        }
        if self.jurisdiction.is_some() {
            present.push(Dimension::Jurisdiction);
        }
        if self.promo_source.is_some() {
            present.push(Dimension::PromoSource);
        }
        present
    }
}

/// One account key: a code, a currency and exactly the dimensions the code names.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AccountKey {
    pub code: AccountCode,
    pub currency: CurrencyTag,
    pub dims: Dimensions,
}

/// Why an entry or key was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EntryRefusal {
    /// A key's dimensions are not the ones its code names.
    KeyShape(AccountCode),
    /// An entry needs two to sixty-four lines.
    LineCount(usize),
    ZeroAmount,
    /// Two lines of one entry share a key.
    DuplicateKey,
    /// A book and currency group does not sum to zero.
    Unbalanced {
        book: Book,
        currency: CurrencyTag,
        sum: i64,
    },
    /// The entry kind's allowlist has no such (code, side).
    NotAllowed {
        code: AccountCode,
        debit: bool,
    },
    AmountOverflow,
}

impl AccountKey {
    pub fn new(code: AccountCode, currency: CurrencyTag) -> Self {
        Self {
            code,
            currency,
            dims: Dimensions::default(),
        }
    }

    pub fn payer(mut self, payer: PayerId) -> Self {
        self.dims.payer = Some(payer);
        self
    }

    pub fn lot(mut self, lot: LotId) -> Self {
        self.dims.lot = Some(lot);
        self
    }

    pub fn allocation(mut self, allocation: AllocationId) -> Self {
        self.dims.allocation = Some(allocation);
        self
    }

    pub fn provider(mut self, provider: &str) -> Self {
        self.dims.provider = Some(provider.to_owned());
        self
    }

    pub fn processor(mut self, processor: &str) -> Self {
        self.dims.processor = Some(processor.to_owned());
        self
    }

    pub fn loss_class(mut self, loss_class: &str) -> Self {
        self.dims.loss_class = Some(loss_class.to_owned());
        self
    }

    pub fn jurisdiction(mut self, jurisdiction: &str) -> Self {
        self.dims.jurisdiction = Some(jurisdiction.to_owned());
        self
    }

    pub fn promo_source(mut self, promo_source: &str) -> Self {
        self.dims.promo_source = Some(promo_source.to_owned());
        self
    }

    /// The key carries exactly the dimensions its code names.
    pub fn validate_shape(&self) -> Result<(), EntryRefusal> {
        let mut required: Vec<Dimension> = self.code.dimensions().to_vec();
        required.sort();
        let mut present = self.dims.present();
        present.sort();
        if required == present {
            Ok(())
        } else {
            Err(EntryRefusal::KeyShape(self.code))
        }
    }
}

/// One signed line of an entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Line {
    pub key: AccountKey,
    /// Positive is a debit, negative a credit; never zero.
    pub amount_micro_dollars: i64,
}

impl Line {
    pub fn debit(key: AccountKey, amount_micro_dollars: u64) -> Result<Self, EntryRefusal> {
        let amount =
            i64::try_from(amount_micro_dollars).map_err(|_| EntryRefusal::AmountOverflow)?;
        Ok(Self {
            key,
            amount_micro_dollars: amount,
        })
    }

    pub fn credit(key: AccountKey, amount_micro_dollars: u64) -> Result<Self, EntryRefusal> {
        let amount =
            i64::try_from(amount_micro_dollars).map_err(|_| EntryRefusal::AmountOverflow)?;
        Ok(Self {
            key,
            amount_micro_dollars: -amount,
        })
    }
}

/// The kinds of journal entry (17.6); the allowlist lives in `entry_rules`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EntryKind {
    TrialGrant,
    GoodwillGrant,
    Purchase,
    ProcessingFee,
    Payout,
    Allocation,
    AllocationRelease,
    Reservation,
    MeterHold,
    Release,
    Settlement,
    MeterCharge,
    CompanyLoss,
    CustomerCredit,
    RefundRequested,
    RefundPaid,
    ManualRefundPayout,
    RefundReturnedToCredit,
    ExternalRefund,
    DisputeFreeze,
    DisputeWithheld,
    DisputeWon,
    DisputeLost,
    ChargebackRecovery,
    PromotionalExpiry,
    PromotionalForfeit,
    Rounding,
    SupplierExposureWriteOff,
    SupplierInvoiceMatch,
    SupplierPayment,
    SupplierPrepayment,
    ManualAdjustment,
    Reversal,
}

impl EntryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::TrialGrant => "trial_grant",
            Self::GoodwillGrant => "goodwill_grant",
            Self::Purchase => "purchase",
            Self::ProcessingFee => "processing_fee",
            Self::Payout => "payout",
            Self::Allocation => "allocation",
            Self::AllocationRelease => "allocation_release",
            Self::Reservation => "reservation",
            Self::MeterHold => "meter_hold",
            Self::Release => "release",
            Self::Settlement => "settlement",
            Self::MeterCharge => "meter_charge",
            Self::CompanyLoss => "company_loss",
            Self::CustomerCredit => "customer_credit",
            Self::RefundRequested => "refund_requested",
            Self::RefundPaid => "refund_paid",
            Self::ManualRefundPayout => "manual_refund_payout",
            Self::RefundReturnedToCredit => "refund_returned_to_credit",
            Self::ExternalRefund => "external_refund",
            Self::DisputeFreeze => "dispute_freeze",
            Self::DisputeWithheld => "dispute_withheld",
            Self::DisputeWon => "dispute_won",
            Self::DisputeLost => "dispute_lost",
            Self::ChargebackRecovery => "chargeback_recovery",
            Self::PromotionalExpiry => "promotional_expiry",
            Self::PromotionalForfeit => "promotional_forfeit",
            Self::Rounding => "rounding",
            Self::SupplierExposureWriteOff => "supplier_exposure_write_off",
            Self::SupplierInvoiceMatch => "supplier_invoice_match",
            Self::SupplierPayment => "supplier_payment",
            Self::SupplierPrepayment => "supplier_prepayment",
            Self::ManualAdjustment => "manual_adjustment",
            Self::Reversal => "reversal",
        }
    }
}

/// The lock anchor an entry's hash chain belongs to (17.7).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamKind {
    Payer,
    Session,
    Campaign,
    Account,
    Platform,
}

impl StreamKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Payer => "payer",
            Self::Session => "session",
            Self::Campaign => "campaign",
            Self::Account => "account",
            Self::Platform => "platform",
        }
    }
}

/// The kind of external reference an entry may carry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalRefKind {
    None,
    ProcessorPayment,
    ProcessorRefund,
    ProcessorDispute,
    ProcessorBalanceTxn,
    ProcessorPayout,
    SupplierInvoice,
    BankTransaction,
    SupportCase,
    Attempt,
    TrialGrant,
}

impl ExternalRefKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ProcessorPayment => "processor_payment",
            Self::ProcessorRefund => "processor_refund",
            Self::ProcessorDispute => "processor_dispute",
            Self::ProcessorBalanceTxn => "processor_balance_txn",
            Self::ProcessorPayout => "processor_payout",
            Self::SupplierInvoice => "supplier_invoice",
            Self::BankTransaction => "bank_transaction",
            Self::SupportCase => "support_case",
            Self::Attempt => "attempt",
            Self::TrialGrant => "trial_grant",
        }
    }
}

/// Why an entry was posted: a closed code, never free text, because the reason is sealed into the
/// journal with the entry (22.1). `finance.entry_reason` holds the same set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryReason {
    TrialGranted,
    GoodwillGranted,
    PurchaseConfirmed,
    Allocated,
    AllocationReleased,
    Admitted,
    Settled,
    ReleasedNotDispatched,
    ReleasedHoldDeadline,
    CompanyLoss,
    ProcessorFee,
    ProcessorPayout,
    RefundRequested,
    RefundPaid,
    DisputeOpened,
    DisputeResolved,
    ChargebackRecovered,
    PromotionalExpired,
    PromotionalForfeited,
    Rounding,
    SupplierExposureWrittenOff,
    SupplierInvoiceMatched,
    SupplierPaid,
    SupplierPrepaid,
    ManualAdjustment,
    Correction,
    CreditTechnicalFailure,
    CreditContentFailure,
    CreditBillingError,
    CreditDuplicateCharge,
    CreditGoodwill,
}

impl EntryReason {
    /// The code stored and sealed.
    pub fn label(self) -> &'static str {
        match self {
            Self::TrialGranted => "trial_granted",
            Self::GoodwillGranted => "goodwill_granted",
            Self::PurchaseConfirmed => "purchase_confirmed",
            Self::Allocated => "allocated",
            Self::AllocationReleased => "allocation_released",
            Self::Admitted => "admitted",
            Self::Settled => "settled",
            Self::ReleasedNotDispatched => "released_not_dispatched",
            Self::ReleasedHoldDeadline => "released_hold_deadline",
            Self::CompanyLoss => "company_loss",
            Self::ProcessorFee => "processor_fee",
            Self::ProcessorPayout => "processor_payout",
            Self::RefundRequested => "refund_requested",
            Self::RefundPaid => "refund_paid",
            Self::DisputeOpened => "dispute_opened",
            Self::DisputeResolved => "dispute_resolved",
            Self::ChargebackRecovered => "chargeback_recovered",
            Self::PromotionalExpired => "promotional_expired",
            Self::PromotionalForfeited => "promotional_forfeited",
            Self::Rounding => "rounding",
            Self::SupplierExposureWrittenOff => "supplier_exposure_written_off",
            Self::SupplierInvoiceMatched => "supplier_invoice_matched",
            Self::SupplierPaid => "supplier_paid",
            Self::SupplierPrepaid => "supplier_prepaid",
            Self::ManualAdjustment => "manual_adjustment",
            Self::Correction => "correction",
            Self::CreditTechnicalFailure => "credit_technical_failure",
            Self::CreditContentFailure => "credit_content_failure",
            Self::CreditBillingError => "credit_billing_error",
            Self::CreditDuplicateCharge => "credit_duplicate_charge",
            Self::CreditGoodwill => "credit_goodwill",
        }
    }

    /// Every code, in the order of the migration's seed.
    pub const ALL: [Self; 31] = [
        Self::TrialGranted,
        Self::GoodwillGranted,
        Self::PurchaseConfirmed,
        Self::Allocated,
        Self::AllocationReleased,
        Self::Admitted,
        Self::Settled,
        Self::ReleasedNotDispatched,
        Self::ReleasedHoldDeadline,
        Self::CompanyLoss,
        Self::ProcessorFee,
        Self::ProcessorPayout,
        Self::RefundRequested,
        Self::RefundPaid,
        Self::DisputeOpened,
        Self::DisputeResolved,
        Self::ChargebackRecovered,
        Self::PromotionalExpired,
        Self::PromotionalForfeited,
        Self::Rounding,
        Self::SupplierExposureWrittenOff,
        Self::SupplierInvoiceMatched,
        Self::SupplierPaid,
        Self::SupplierPrepaid,
        Self::ManualAdjustment,
        Self::Correction,
        Self::CreditTechnicalFailure,
        Self::CreditContentFailure,
        Self::CreditBillingError,
        Self::CreditDuplicateCharge,
        Self::CreditGoodwill,
    ];
}

/// Who posts an entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Actor {
    System,
    Operator {
        actor_id: [u8; 16],
        case_ref: String,
        approver_id: Option<[u8; 16]>,
    },
}

/// The ids that tie an entry to the objects it belongs to; all are hash-covered (17.7).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Correlation {
    pub campaign: Option<CampaignKey>,
    pub session: Option<SessionKey>,
    pub attempt: Option<AttemptId>,
    pub reservation: Option<ReservationId>,
    pub price_version: Option<PriceVersionId>,
}

/// One journal entry before it is posted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalEntry {
    pub entry_id: EntryId,
    pub kind: EntryKind,
    /// Domain-separated SHA-256 of the operation's idempotency fields (20); unique with the kind.
    pub business_key: [u8; 32],
    pub external_ref: Option<(ExternalRefKind, String)>,
    pub stream: (StreamKind, [u8; 16]),
    pub effective_at_ms: i64,
    pub reason: EntryReason,
    pub actor: Actor,
    pub correlation: Correlation,
    pub lines: Vec<Line>,
}

impl JournalEntry {
    /// Two to sixty-four non-zero lines, one per key, balanced in every book and currency.
    pub fn validate(&self) -> Result<(), EntryRefusal> {
        if !(2..=64).contains(&self.lines.len()) {
            return Err(EntryRefusal::LineCount(self.lines.len()));
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut sums: BTreeMap<(Book, CurrencyTag), i64> = BTreeMap::new();
        for line in &self.lines {
            line.key.validate_shape()?;
            if line.amount_micro_dollars == 0 {
                return Err(EntryRefusal::ZeroAmount);
            }
            if !seen.insert(&line.key) {
                return Err(EntryRefusal::DuplicateKey);
            }
            let sum = sums
                .entry((line.key.code.book(), line.key.currency))
                .or_default();
            *sum = sum
                .checked_add(line.amount_micro_dollars)
                .ok_or(EntryRefusal::AmountOverflow)?;
        }
        for ((book, currency), sum) in sums {
            if sum != 0 {
                return Err(EntryRefusal::Unbalanced {
                    book,
                    currency,
                    sum,
                });
            }
        }
        Ok(())
    }
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&u32::try_from(bytes.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    put_bytes(out, text.as_bytes());
}

fn put_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_opt_bytes(out: &mut Vec<u8>, value: Option<&[u8]>) {
    match value {
        None => out.push(0),
        Some(bytes) => {
            out.push(1);
            put_bytes(out, bytes);
        }
    }
}

fn put_opt_text(out: &mut Vec<u8>, value: Option<&str>) {
    put_opt_bytes(out, value.map(str::as_bytes));
}

fn put_opt_i64(out: &mut Vec<u8>, value: Option<i64>) {
    match value {
        None => out.push(0),
        Some(number) => {
            out.push(1);
            put_i64(out, number);
        }
    }
}

/// What the database adds to an entry when it posts it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PostingContext {
    pub event_id: EventId,
    pub stream_seq: i64,
    pub recorded_at_ms: i64,
    pub period_id: i64,
}

/// Version 1 canonical bytes of a posted entry (17.7 item 2), frozen: a later field is version 2
/// beside this function, never an edit of it. `finance.entry_canonical_v1` writes the same bytes.
pub fn canonical_v1(entry: &JournalEntry, context: PostingContext) -> Vec<u8> {
    let mut out = Vec::new();
    put_bytes(&mut out, entry.entry_id.as_bytes());
    put_bytes(&mut out, context.event_id.as_bytes());
    put_text(&mut out, entry.kind.label());
    put_bytes(&mut out, &entry.business_key);
    let (ref_kind, ref_value) = match &entry.external_ref {
        Some((kind, value)) => (*kind, Some(value.as_str())),
        None => (ExternalRefKind::None, None),
    };
    put_text(&mut out, ref_kind.label());
    put_opt_text(&mut out, ref_value);
    put_text(&mut out, entry.stream.0.label());
    put_bytes(&mut out, &entry.stream.1);
    put_i64(&mut out, context.stream_seq);
    put_i64(&mut out, context.recorded_at_ms);
    put_i64(&mut out, entry.effective_at_ms);
    put_i64(&mut out, context.period_id);
    put_opt_i64(&mut out, None); // corrects_period_id
    put_opt_bytes(&mut out, None); // reverses_entry_id
    let correlation = &entry.correlation;
    put_opt_bytes(
        &mut out,
        correlation
            .campaign
            .as_ref()
            .map(|id| id.as_bytes().as_slice()),
    );
    put_opt_bytes(
        &mut out,
        correlation
            .session
            .as_ref()
            .map(|id| id.as_bytes().as_slice()),
    );
    put_opt_bytes(
        &mut out,
        correlation
            .attempt
            .as_ref()
            .map(|id| id.as_bytes().as_slice()),
    );
    put_opt_bytes(
        &mut out,
        correlation
            .reservation
            .as_ref()
            .map(|id| id.as_bytes().as_slice()),
    );
    put_opt_bytes(
        &mut out,
        correlation
            .price_version
            .as_ref()
            .map(|id| id.as_bytes().as_slice()),
    );
    put_text(&mut out, entry.reason.label());
    let (actor_kind, actor_id, approver_id, case_ref) = match &entry.actor {
        Actor::System => ("system", None, None, None),
        Actor::Operator {
            actor_id,
            case_ref,
            approver_id,
        } => (
            "operator",
            Some(actor_id.as_slice()),
            approver_id.as_ref().map(|id| id.as_slice()),
            Some(case_ref.as_str()),
        ),
    };
    put_opt_text(&mut out, case_ref);
    put_text(&mut out, actor_kind);
    put_opt_bytes(&mut out, actor_id);
    put_opt_bytes(&mut out, approver_id);
    put_i64(
        &mut out,
        i64::try_from(entry.lines.len()).unwrap_or(i64::MAX),
    );
    for line in &entry.lines {
        let key = &line.key;
        put_i64(&mut out, i64::from(key.code.number()));
        put_text(&mut out, key.currency.as_str());
        put_opt_bytes(
            &mut out,
            key.dims.payer.as_ref().map(|id| id.as_bytes().as_slice()),
        );
        put_opt_bytes(
            &mut out,
            key.dims.lot.as_ref().map(|id| id.as_bytes().as_slice()),
        );
        put_opt_bytes(
            &mut out,
            key.dims
                .allocation
                .as_ref()
                .map(|id| id.as_bytes().as_slice()),
        );
        put_opt_text(&mut out, key.dims.provider.as_deref());
        put_opt_text(&mut out, key.dims.processor.as_deref());
        put_opt_text(&mut out, key.dims.loss_class.as_deref());
        put_opt_text(&mut out, key.dims.jurisdiction.as_deref());
        put_opt_text(&mut out, key.dims.promo_source.as_deref());
        put_i64(&mut out, line.amount_micro_dollars);
    }
    out
}

/// A one-line audit description of a key, for logs and refusals: ids as hex, no free text.
pub fn describe_key(key: &AccountKey) -> String {
    let mut text = format!("{}", key.code.number());
    if let Some(payer) = key.dims.payer {
        text.push_str(&format!(" payer={}", &hex(payer.as_bytes())[..8]));
    }
    if let Some(lot) = key.dims.lot {
        text.push_str(&format!(" lot={}", &hex(lot.as_bytes())[..8]));
    }
    if let Some(allocation) = key.dims.allocation {
        text.push_str(&format!(" allocation={}", &hex(allocation.as_bytes())[..8]));
    }
    text
}
