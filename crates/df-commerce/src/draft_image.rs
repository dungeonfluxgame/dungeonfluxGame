//! Creator-account portrait funding policy (owner decision 2026-10-28).
//! Authentication supplies the account separately; no seat, campaign, or session is a payer.

use df_types::AccountId;
use serde::{Deserialize, Serialize};

use crate::digest::business_key;
use crate::finance_ids::PriceVersionId;
use crate::price_schedule::{PriceVersion, UnitsRefusal};

mod consent;
mod stages;
pub use consent::{Consent, Exposure, Quote};
pub use stages::{
    JobConsent, JobScope, MAXIMUM_STAGE_ROUTES, PaidStageKind, StageBinding, StageCall,
    StagePublication, StageRoute,
};

#[cfg(test)]
mod tests;

/// Maximum distinct unit categories in a consented quote.
pub const MAXIMUM_UNIT_CATEGORIES: usize = 32;
/// Upper bound on one pinned provider, model, route, or unit-category label.
pub const MAXIMUM_LABEL_BYTES: usize = 120;

macro_rules! funding_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
        pub struct $name(pub [u8; 16]);
    };
}
funding_id!(ConsentId);
funding_id!(DraftId);
funding_id!(JobId);
funding_id!(StageId);
funding_id!(AssetId);
funding_id!(PublicationId);

/// All values the creator explicitly consents to. New terms require a new consent id.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    #[serde(with = "account_serde")]
    pub account: AccountId,
    pub draft: DraftId,
    pub draft_revision: u64,
    pub job: JobId,
    pub source_digest: [u8; 32],
    pub config_digest: [u8; 32],
    pub provider: String,
    pub model: String,
    pub route: String,
    pub price_version: [u8; 16],
    pub maximum_units: Vec<(String, u64)>,
}

/// One immutable quote and admission mapping inside the consented job.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub scope: Scope,
    pub stage: StageId,
    pub asset: AssetId,
    pub publication: PublicationId,
    pub operation: [u8; 16],
    pub attempt: [u8; 16],
    pub effect: [u8; 16],
    pub payload_digest: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    WrongAccount,
    InvalidConsent,
    ConsentRevoked,
    ConsentExpired,
    ConsentRevisionMismatch,
    ConsentCapExceeded,
    InsufficientFunds,
    QuoteMismatch,
    QuoteExpired,
    InvalidBinding,
    InvalidUnits,
    PriceVersionMismatch,
    Unpriced(UnitsRefusal),
}

impl Scope {
    /// Bounded, canonical categories prevent duplicate units from changing a quote's meaning.
    pub fn validate(&self, authenticated_account: AccountId) -> Result<(), Refusal> {
        if self.account != authenticated_account {
            return Err(Refusal::WrongAccount);
        }
        if self.draft.0 == [0; 16]
            || self.job.0 == [0; 16]
            || self.draft_revision == 0
            || self.provider.is_empty()
            || self.provider.len() > MAXIMUM_LABEL_BYTES
            || self.model.is_empty()
            || self.model.len() > MAXIMUM_LABEL_BYTES
            || self.route.is_empty()
            || self.route.len() > MAXIMUM_LABEL_BYTES
            || self.price_version == [0; 16]
        {
            return Err(Refusal::InvalidBinding);
        }
        if self.maximum_units.is_empty()
            || self.maximum_units.len() > MAXIMUM_UNIT_CATEGORIES
            || self.maximum_units.iter().any(|(category, units)| {
                category.is_empty() || category.len() > MAXIMUM_LABEL_BYTES || *units == 0
            })
            || self
                .maximum_units
                .windows(2)
                .any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err(Refusal::InvalidUnits);
        }
        Ok(())
    }

    pub fn digest(&self) -> [u8; 32] {
        let mut parts: Vec<Vec<u8>> = vec![
            self.account.as_bytes().to_vec(),
            self.draft.0.to_vec(),
            self.draft_revision.to_be_bytes().to_vec(),
            self.job.0.to_vec(),
            self.source_digest.to_vec(),
            self.config_digest.to_vec(),
            self.provider.as_bytes().to_vec(),
            self.model.as_bytes().to_vec(),
            self.route.as_bytes().to_vec(),
            self.price_version.to_vec(),
        ];
        for (category, quantity) in &self.maximum_units {
            parts.push(category.as_bytes().to_vec());
            parts.push(quantity.to_be_bytes().to_vec());
        }
        let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
        business_key("df-commerce/draft-image-consent-v1", &refs)
    }
}

impl Binding {
    pub fn validate(&self, account: AccountId) -> Result<(), Refusal> {
        self.scope.validate(account)?;
        if [
            self.stage.0,
            self.asset.0,
            self.publication.0,
            self.operation,
            self.attempt,
            self.effect,
        ]
        .contains(&[0; 16])
        {
            return Err(Refusal::InvalidBinding);
        }
        Ok(())
    }

    pub fn digest(&self) -> [u8; 32] {
        business_key(
            "df-commerce/draft-image-binding-v1",
            &[
                &self.scope.digest(),
                &self.stage.0,
                &self.asset.0,
                &self.publication.0,
                &self.operation,
                &self.attempt,
                &self.effect,
                &self.payload_digest,
            ],
        )
    }
}

/// Pinned supplier and retail bounds, calculated by the existing price-version authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Maximums {
    pub retail_micro_dollars: u64,
    pub supplier_micro_dollars: u64,
}

pub fn price_maximum(scope: &Scope, version: &PriceVersion) -> Result<Maximums, Refusal> {
    if version.id != PriceVersionId::from_array(scope.price_version) {
        return Err(Refusal::PriceVersionMismatch);
    }
    let supplier = version
        .supplier_exact(&scope.provider, &scope.model, &scope.maximum_units)
        .map_err(Refusal::Unpriced)?
        .round_up()
        .map_err(|error| Refusal::Unpriced(error.into()))?;
    let retail = version
        .retail_exact(&scope.provider, &scope.model, &scope.maximum_units)
        .map_err(Refusal::Unpriced)?
        .round_up()
        .map_err(|error| Refusal::Unpriced(error.into()))?;
    Ok(Maximums {
        retail_micro_dollars: retail,
        supplier_micro_dollars: supplier,
    })
}

mod account_serde {
    use df_types::AccountId;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(account: &AccountId, serializer: S) -> Result<S::Ok, S::Error> {
        account.as_bytes().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<AccountId, D::Error> {
        let bytes = <[u8; 16]>::deserialize(deserializer)?;
        AccountId::from_bytes(&bytes).map_err(|_| serde::de::Error::custom("invalid account"))
    }
}
