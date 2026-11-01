//! Pure consent checks. Durable callers must derive these observations under the
//! same PostgreSQL locks that post the existing account ledger reservation.

use df_types::AccountId;

use crate::finance_ids::QuoteId;
use crate::price_schedule::PriceVersion;

use super::{Binding, ConsentId, Maximums, Refusal, Scope, price_maximum};

/// Immutable creator-approved terms. Increasing a cap requires a fresh consent id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Consent {
    pub id: ConsentId,
    pub scope: Scope,
    pub cap_micro_dollars: u64,
    pub expires_at_ms: i64,
    pub revision: u64,
    pub revoked: bool,
}

/// A quote can only be constructed from the existing pinned price version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Quote {
    id: QuoteId,
    consent: ConsentId,
    consent_revision: u64,
    consent_digest: [u8; 32],
    binding: Binding,
    maximums: Maximums,
    expires_at_ms: i64,
}

/// Already charged plus still reserved money, read under the account and consent locks.
/// Unknown supplier outcomes remain in `held_micro_dollars` until reconciled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Exposure {
    pub charged_micro_dollars: u64,
    pub held_micro_dollars: u64,
    pub available_micro_dollars: u64,
}

impl Consent {
    pub fn digest(&self) -> [u8; 32] {
        crate::digest::business_key(
            "df-commerce/draft-image-terms-v1",
            &[
                &self.id.0,
                &self.scope.digest(),
                &self.cap_micro_dollars.to_be_bytes(),
                &self.expires_at_ms.to_be_bytes(),
                &self.revision.to_be_bytes(),
            ],
        )
    }

    pub fn validate(&self, account: AccountId, now_ms: i64) -> Result<(), Refusal> {
        self.scope.validate(account)?;
        if self.id.0 == [0; 16] || self.cap_micro_dollars == 0 || self.revision == 0 {
            return Err(Refusal::InvalidConsent);
        }
        if self.revoked {
            return Err(Refusal::ConsentRevoked);
        }
        if now_ms < 0 || self.expires_at_ms <= now_ms {
            return Err(Refusal::ConsentExpired);
        }
        Ok(())
    }
}

impl Quote {
    pub fn new(
        id: QuoteId,
        consent: &Consent,
        binding: Binding,
        version: &PriceVersion,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> Result<Self, Refusal> {
        consent.validate(binding.scope.account, now_ms)?;
        binding.validate(consent.scope.account)?;
        if binding.scope != consent.scope || id.as_bytes() == &[0; 16] {
            return Err(Refusal::QuoteMismatch);
        }
        if expires_at_ms <= now_ms || expires_at_ms > consent.expires_at_ms {
            return Err(Refusal::QuoteExpired);
        }
        let maximums = price_maximum(&binding.scope, version)?;
        if maximums.retail_micro_dollars == 0
            || maximums.retail_micro_dollars > consent.cap_micro_dollars
        {
            return Err(Refusal::ConsentCapExceeded);
        }
        Ok(Self {
            id,
            consent: consent.id,
            consent_revision: consent.revision,
            consent_digest: consent.digest(),
            binding,
            maximums,
            expires_at_ms,
        })
    }

    pub fn id(&self) -> QuoteId {
        self.id
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn maximums(&self) -> Maximums {
        self.maximums
    }
    pub fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }

    /// Pure admission preflight. This does not reserve funds or authorize a send.
    pub fn validate_admission(
        &self,
        account: AccountId,
        consent: &Consent,
        binding: &Binding,
        exposure: Exposure,
        now_ms: i64,
    ) -> Result<Maximums, Refusal> {
        consent.validate(account, now_ms)?;
        binding.validate(account)?;
        if self.consent != consent.id
            || self.consent_revision != consent.revision
            || self.consent_digest != consent.digest()
        {
            return Err(Refusal::ConsentRevisionMismatch);
        }
        if &self.binding != binding || binding.scope != consent.scope {
            return Err(Refusal::QuoteMismatch);
        }
        if self.expires_at_ms <= now_ms {
            return Err(Refusal::QuoteExpired);
        }
        let aggregate = exposure
            .charged_micro_dollars
            .checked_add(exposure.held_micro_dollars)
            .and_then(|used| used.checked_add(self.maximums.retail_micro_dollars))
            .ok_or(Refusal::ConsentCapExceeded)?;
        if aggregate > consent.cap_micro_dollars {
            return Err(Refusal::ConsentCapExceeded);
        }
        if exposure.available_micro_dollars < self.maximums.retail_micro_dollars {
            return Err(Refusal::InsufficientFunds);
        }
        Ok(self.maximums)
    }
}
