//! Constructors for the account keys the flows post to, in one currency.

use crate::finance_ids::{AllocationId, LotId, PayerId};
use crate::funding::LotClass;
use crate::ledger::{AccountCode, AccountKey, CurrencyTag};

/// The keys of one currency.
#[derive(Clone, Copy, Debug)]
pub struct Keys {
    pub currency: CurrencyTag,
}

impl Keys {
    pub fn new(currency: CurrencyTag) -> Self {
        Self { currency }
    }

    fn key(&self, code: AccountCode) -> AccountKey {
        AccountKey::new(code, self.currency)
    }

    /// `purchased_available` or `promotional_available` of a payer's lot.
    pub fn available(&self, class: LotClass, payer: PayerId, lot: LotId) -> AccountKey {
        let code = match class {
            LotClass::Purchased => AccountCode::PurchasedAvailable,
            LotClass::Promotional => AccountCode::PromotionalAvailable,
        };
        self.key(code).payer(payer).lot(lot)
    }

    pub fn allocated(&self, class: LotClass, allocation: AllocationId, lot: LotId) -> AccountKey {
        let code = match class {
            LotClass::Purchased => AccountCode::PurchasedAllocated,
            LotClass::Promotional => AccountCode::PromotionalAllocated,
        };
        self.key(code).allocation(allocation).lot(lot)
    }

    pub fn reserved(&self, class: LotClass, allocation: AllocationId, lot: LotId) -> AccountKey {
        let code = match class {
            LotClass::Purchased => AccountCode::PurchasedReserved,
            LotClass::Promotional => AccountCode::PromotionalReserved,
        };
        self.key(code).allocation(allocation).lot(lot)
    }

    pub fn frozen(&self, lot: LotId) -> AccountKey {
        self.key(AccountCode::PurchasedFrozen).lot(lot)
    }

    pub fn clearing(&self, processor: &str) -> AccountKey {
        self.key(AccountCode::ProcessorClearing)
            .processor(processor)
    }

    pub fn chargeback_loss(&self, processor: &str) -> AccountKey {
        self.key(AccountCode::ChargebackLoss).processor(processor)
    }

    pub fn revenue_ai_usage(&self) -> AccountKey {
        self.key(AccountCode::RevenueAiUsage)
    }

    pub fn promotional_offset(&self, source: &str) -> AccountKey {
        self.key(AccountCode::PromotionalOffset)
            .promo_source(source)
    }

    pub fn supplier_payable(&self, provider: &str) -> AccountKey {
        self.key(AccountCode::SupplierPayable).provider(provider)
    }

    pub fn cost_of_services(&self, provider: &str, promotional: bool) -> AccountKey {
        let code = if promotional {
            AccountCode::CostOfServicesPromotional
        } else {
            AccountCode::CostOfServices
        };
        self.key(code).provider(provider)
    }

    pub fn refund_payable(&self, lot: LotId) -> AccountKey {
        self.key(AccountCode::RefundPayable).lot(lot)
    }

    pub fn dispute_withheld(&self, processor: &str) -> AccountKey {
        self.key(AccountCode::DisputeFundsWithheld)
            .processor(processor)
    }

    pub fn processing_fees(&self, processor: &str) -> AccountKey {
        self.key(AccountCode::ProcessingFees).processor(processor)
    }

    pub fn tax_payable(&self, jurisdiction: &str) -> AccountKey {
        self.key(AccountCode::TaxPayable).jurisdiction(jurisdiction)
    }

    pub fn absorbed(&self, provider: &str, loss_class: &str) -> AccountKey {
        self.key(AccountCode::AbsorbedSupplierCost)
            .provider(provider)
            .loss_class(loss_class)
    }

    pub fn exposure_open(&self, provider: &str) -> AccountKey {
        self.key(AccountCode::SupplierExposureOpen)
            .provider(provider)
    }

    pub fn exposure_offset(&self, provider: &str) -> AccountKey {
        self.key(AccountCode::SupplierExposureOffset)
            .provider(provider)
    }

    /// Every key of the global (lot-free) accounts a version's flows post to, for the keys a price
    /// version publication creates (9.1): revenue, offsets and the per-provider keys.
    pub fn publication_keys(&self, providers: &[&str]) -> Vec<AccountKey> {
        let mut keys = vec![
            self.key(AccountCode::RevenueAiUsage),
            self.key(AccountCode::RevenueTableTime),
            self.key(AccountCode::RefundsOfConsumedService),
            self.key(AccountCode::Rounding),
            self.key(AccountCode::BankOperating),
            self.key(AccountCode::OwnerEquity),
            self.promotional_offset("trial"),
            self.promotional_offset("goodwill"),
        ];
        for provider in providers {
            keys.push(self.supplier_payable(provider));
            keys.push(self.key(AccountCode::SupplierAccrued).provider(provider));
            keys.push(self.key(AccountCode::SupplierPrepaid).provider(provider));
            keys.push(self.cost_of_services(provider, false));
            keys.push(self.cost_of_services(provider, true));
            keys.push(self.exposure_open(provider));
            keys.push(self.exposure_offset(provider));
        }
        keys
    }
}
