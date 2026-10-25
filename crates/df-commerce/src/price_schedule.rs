//! A published price version and the amounts priced from it
//! (planning/finance-ledger-research.md 4, 9.1, 9.2).
//!
//! The money core prices from units, never from a dollar amount: a supplier cost is the exact sum
//! of `units x pinned tariff`, rounded half up once; a retail maximum is the same sum times the
//! markup, rounded up; a retail charge is rounded down once and capped at its maximum. A category
//! with no tariff row refuses, it never prices at zero.

use crate::finance_ids::PriceVersionId;
use crate::ledger::CurrencyTag;
use crate::rounding::{Ratio, RoundingError};

/// One tariff row of a version. A context tier of zero is unbounded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TariffLine {
    pub provider: String,
    pub model: String,
    pub category: String,
    pub context_tier_max_tokens: u64,
    pub nano_usd: u64,
    pub per: u64,
}

/// An immutable published version (9.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceVersion {
    pub id: PriceVersionId,
    /// An opaque short code, never a rate or a date range (18.9).
    pub label: String,
    pub currency: CurrencyTag,
    pub platform_rate_micro_dollars_per_hour: u64,
    pub markup_numerator: u64,
    pub markup_denominator: u64,
    pub tariff: Vec<TariffLine>,
    pub proposed_pilot: bool,
}

/// Why units could not be priced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnitsRefusal {
    /// The pinned version has no row for this category (or the quantity exceeds every stored tier).
    CategoryUnpriced(String),
    /// The route declares a category the request's maximum units leave out.
    MaximumUnitsIncomplete(String),
    /// The request names a category the route did not declare.
    UndeclaredCategory(String),
    Arithmetic(RoundingError),
}

impl From<RoundingError> for UnitsRefusal {
    fn from(error: RoundingError) -> Self {
        Self::Arithmetic(error)
    }
}

/// Units as `(category, quantity)`.
pub type Units = [(String, u64)];

/// Which of the tiers that cover a quantity prices it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tier {
    /// A reservation covers any actual use: the highest rate among the covering tiers.
    Highest,
    /// An actual charge is the tier the actual units fall in: the narrowest covering tier.
    Lowest,
}

impl PriceVersion {
    /// The row that prices `quantity` of `category` under `tier`. A context tier of zero is
    /// unbounded, so it covers everything and ranks widest.
    fn line_for(
        &self,
        provider: &str,
        model: &str,
        category: &str,
        quantity: u64,
        tier: Tier,
    ) -> Option<&TariffLine> {
        let covering = self
            .tariff
            .iter()
            .filter(|line| {
                line.provider == provider && line.model == model && line.category == category
            })
            .filter(|line| {
                line.context_tier_max_tokens == 0 || quantity <= line.context_tier_max_tokens
            });
        match tier {
            Tier::Highest => covering.max_by_key(|line| {
                u128::from(line.nano_usd) * 1_000_000 / u128::from(line.per.max(1))
            }),
            Tier::Lowest => covering.min_by_key(|line| {
                if line.context_tier_max_tokens == 0 {
                    u64::MAX
                } else {
                    line.context_tier_max_tokens
                }
            }),
        }
    }

    /// The exact supplier price in micro-dollars of `units` under `tier`, summed before rounding.
    fn supplier_exact_at(
        &self,
        provider: &str,
        model: &str,
        units: &Units,
        tier: Tier,
    ) -> Result<Ratio, UnitsRefusal> {
        let mut total = Ratio {
            numerator: 0,
            denominator: 1,
        };
        for (category, quantity) in units {
            let line = self
                .line_for(provider, model, category, *quantity, tier)
                .ok_or_else(|| UnitsRefusal::CategoryUnpriced(category.clone()))?;
            let exact = Ratio {
                numerator: u128::from(*quantity) * u128::from(line.nano_usd),
                denominator: u128::from(line.per) * 1_000,
            }
            .reduced();
            total = total.checked_add(exact)?;
        }
        Ok(total)
    }

    /// The exact supplier price of a request's maximum, at the highest covering tier.
    pub fn supplier_exact(
        &self,
        provider: &str,
        model: &str,
        units: &Units,
    ) -> Result<Ratio, UnitsRefusal> {
        self.supplier_exact_at(provider, model, units, Tier::Highest)
    }

    fn retail_exact_at(
        &self,
        provider: &str,
        model: &str,
        units: &Units,
        tier: Tier,
    ) -> Result<Ratio, UnitsRefusal> {
        Ok(self
            .supplier_exact_at(provider, model, units, tier)?
            .checked_scale(
                u128::from(self.markup_numerator),
                u128::from(self.markup_denominator),
            )?)
    }

    /// The exact retail price of a request's maximum: the supplier price times the markup.
    pub fn retail_exact(
        &self,
        provider: &str,
        model: &str,
        units: &Units,
    ) -> Result<Ratio, UnitsRefusal> {
        self.retail_exact_at(provider, model, units, Tier::Highest)
    }

    /// Rule 4.1: the supplier maximum of a request, rounded up.
    pub fn supplier_maximum(
        &self,
        provider: &str,
        model: &str,
        units: &Units,
    ) -> Result<u64, UnitsRefusal> {
        Ok(self.supplier_exact(provider, model, units)?.round_up()?)
    }

    /// Rule 4.1: the retail maximum of a request, rounded up.
    pub fn retail_maximum(
        &self,
        provider: &str,
        model: &str,
        units: &Units,
    ) -> Result<u64, UnitsRefusal> {
        Ok(self.retail_exact(provider, model, units)?.round_up()?)
    }

    /// Rule 4.2: the retail charge of the actual units at the tier they fall in, rounded down once,
    /// capped at `maximum`.
    pub fn retail_charge(
        &self,
        provider: &str,
        model: &str,
        units: &Units,
        maximum: u64,
    ) -> Result<u64, UnitsRefusal> {
        Ok(self
            .retail_exact_at(provider, model, units, Tier::Lowest)?
            .round_down()?
            .min(maximum))
    }

    /// Rule 4.3: the supplier cost of the actual units at the tier they fall in, rounded half up once.
    pub fn supplier_cost(
        &self,
        provider: &str,
        model: &str,
        units: &Units,
    ) -> Result<u64, UnitsRefusal> {
        Ok(self
            .supplier_exact_at(provider, model, units, Tier::Lowest)?
            .round_half_up()?)
    }
}

/// `maximum_units` must cover every category the route declares and name no other (9.2).
pub fn units_cover_route(declared: &[&str], maximum_units: &Units) -> Result<(), UnitsRefusal> {
    for category in declared {
        if !maximum_units.iter().any(|(named, _)| named == category) {
            return Err(UnitsRefusal::MaximumUnitsIncomplete((*category).to_owned()));
        }
    }
    for (named, _) in maximum_units {
        if !declared.contains(&named.as_str()) {
            return Err(UnitsRefusal::UndeclaredCategory(named.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn version() -> PriceVersion {
        let line = |category: &str, nano_usd, per| TariffLine {
            provider: "openai".into(),
            model: "gpt-6-luna".into(),
            category: category.into(),
            context_tier_max_tokens: 0,
            nano_usd,
            per,
        };
        PriceVersion {
            id: PriceVersionId::from_array([7; 16]),
            label: "pv-test".into(),
            currency: CurrencyTag::USD,
            platform_rate_micro_dollars_per_hour: 2_000_000,
            markup_numerator: 3,
            markup_denominator: 2,
            tariff: vec![
                line("text_input", 100_000_000, 1_000_000),
                line("text_output", 500_000_000, 1_000_000),
            ],
            proposed_pilot: true,
        }
    }

    fn units(text_input: u64, text_output: u64) -> Vec<(String, u64)> {
        vec![
            ("text_input".into(), text_input),
            ("text_output".into(), text_output),
        ]
    }

    #[test]
    fn retail_maximum_rounds_up_and_charge_rounds_down_from_the_same_exact_sum() {
        let version = version();
        // 10 input tokens at 0.1 micro and 3 output at 0.5 micro: supplier 2.5, retail 3.75.
        let maximum = version
            .retail_maximum("openai", "gpt-6-luna", &units(10, 3))
            .expect("maximum");
        assert_eq!(maximum, 4);
        let charge = version
            .retail_charge("openai", "gpt-6-luna", &units(10, 3), maximum)
            .expect("charge");
        assert_eq!(charge, 3);
        let cost = version
            .supplier_cost("openai", "gpt-6-luna", &units(10, 3))
            .expect("cost");
        assert_eq!(cost, 3);
    }

    #[test]
    fn a_charge_never_exceeds_its_maximum() {
        let version = version();
        let charge = version
            .retail_charge("openai", "gpt-6-luna", &units(1_000_000, 1_000_000), 5)
            .expect("charge");
        assert_eq!(charge, 5);
    }

    #[test]
    fn a_category_with_no_tariff_row_is_refused_not_priced_at_zero() {
        let version = version();
        let refusal = version.retail_maximum("openai", "gpt-6-luna", &[("image_input".into(), 4)]);
        assert_eq!(
            refusal,
            Err(UnitsRefusal::CategoryUnpriced("image_input".into()))
        );
    }

    #[test]
    fn maximum_units_must_cover_every_declared_category_and_name_no_other() {
        assert_eq!(
            units_cover_route(&["text_input", "text_output"], &[("text_input".into(), 1)]),
            Err(UnitsRefusal::MaximumUnitsIncomplete("text_output".into()))
        );
        assert_eq!(
            units_cover_route(&["text_input"], &units(1, 1)),
            Err(UnitsRefusal::UndeclaredCategory("text_output".into()))
        );
        assert!(units_cover_route(&["text_input", "text_output"], &units(1, 1)).is_ok());
    }

    #[test]
    fn an_actual_charge_uses_the_narrowest_tier_it_falls_in_and_a_maximum_the_dearest() {
        let mut version = version();
        let wide = version.tariff[0].clone();
        version.tariff.push(TariffLine {
            context_tier_max_tokens: 128_000,
            nano_usd: wide.nano_usd / 2,
            ..wide
        });
        let units = [("text_input".to_owned(), 100_000)];
        // 100,000 tokens fall in the 128,000 tier at half the unbounded rate.
        let cost = version.supplier_cost("openai", "gpt-6-luna", &units);
        assert_eq!(cost, Ok(5_000));
        let maximum = version.supplier_maximum("openai", "gpt-6-luna", &units);
        assert_eq!(maximum, Ok(10_000));
    }

    #[test]
    fn a_quantity_above_every_stored_tier_is_unpriced() {
        let mut version = version();
        for line in &mut version.tariff {
            line.context_tier_max_tokens = 272_000;
        }
        assert!(
            version
                .retail_maximum("openai", "gpt-6-luna", &units(271_000, 0))
                .is_ok()
        );
        assert_eq!(
            version.retail_maximum("openai", "gpt-6-luna", &units(272_001, 0)),
            Err(UnitsRefusal::CategoryUnpriced("text_input".into()))
        );
    }
}
