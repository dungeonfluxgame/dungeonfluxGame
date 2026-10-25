//! Exact nonnegative money and usage values, without quote or settlement authority.

/// A value validation or checked arithmetic rejection, without supplied payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoneyError {
    InvalidCurrencyTag,
    CurrencyMismatch,
    UnitMismatch,
    Overflow,
    Underflow,
    ZeroDenominator,
}

/// Exactly three uppercase ASCII letters; syntax does not establish currency support.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Currency([u8; 3]);

impl Currency {
    /// Preserves a three-letter uppercase ASCII tag or returns `InvalidCurrencyTag`.
    pub fn parse(value: &str) -> Result<Self, MoneyError> {
        let bytes: [u8; 3] = value
            .as_bytes()
            .try_into()
            .map_err(|_| MoneyError::InvalidCurrencyTag)?;
        if !bytes.iter().all(u8::is_ascii_uppercase) {
            return Err(MoneyError::InvalidCurrencyTag);
        }
        Ok(Self(bytes))
    }

    /// Returns the exact validated currency tag.
    pub fn as_str(&self) -> &str {
        // The private constructor admits only ASCII, so these bytes are valid UTF-8.
        std::str::from_utf8(&self.0).expect("validated ASCII currency tag")
    }
}

/// Nonnegative micro-units with an explicit currency; 1,000,000 is one whole unit.
/// This arithmetic scale does not specify a gateway or display precision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Money {
    currency: Currency,
    micros: u128,
}

impl Money {
    /// Constructs an exact amount, including zero and `u128::MAX`.
    pub fn new(currency: Currency, micros: u128) -> Self {
        Self { currency, micros }
    }

    pub fn currency(self) -> Currency {
        self.currency
    }

    /// Returns the exact nonnegative count of currency micro-units.
    pub fn micros(self) -> u128 {
        self.micros
    }

    /// Rejects `CurrencyMismatch` before checking for `Overflow`; never converts.
    pub fn checked_add(self, other: Self) -> Result<Self, MoneyError> {
        if self.currency != other.currency {
            return Err(MoneyError::CurrencyMismatch);
        }
        Ok(Self::new(
            self.currency,
            self.micros
                .checked_add(other.micros)
                .ok_or(MoneyError::Overflow)?,
        ))
    }

    /// Rejects `CurrencyMismatch` before checking for a negative result (`Underflow`).
    pub fn checked_sub(self, other: Self) -> Result<Self, MoneyError> {
        if self.currency != other.currency {
            return Err(MoneyError::CurrencyMismatch);
        }
        Ok(Self::new(
            self.currency,
            self.micros
                .checked_sub(other.micros)
                .ok_or(MoneyError::Underflow)?,
        ))
    }
}

/// Closed quantity kinds; provider-specific billing semantics remain caller-owned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageUnit {
    Token,
    Character,
    Byte,
    AudioMillisecond,
    VideoMillisecond,
    Image,
}

/// An exact nonnegative integer quantity with its explicit usage kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Usage {
    quantity: u128,
    unit: UsageUnit,
}

impl Usage {
    /// Constructs a quantity, including zero and `u128::MAX`; negatives are unrepresentable.
    pub fn new(quantity: u128, unit: UsageUnit) -> Self {
        Self { quantity, unit }
    }

    pub fn quantity(self) -> u128 {
        self.quantity
    }

    pub fn unit(self) -> UsageUnit {
        self.unit
    }

    /// Rejects `UnitMismatch` before checking for `Overflow`.
    pub fn checked_add(self, other: Self) -> Result<Self, MoneyError> {
        if self.unit != other.unit {
            return Err(MoneyError::UnitMismatch);
        }
        Ok(Self::new(
            self.quantity
                .checked_add(other.quantity)
                .ok_or(MoneyError::Overflow)?,
            self.unit,
        ))
    }
}

/// An exact rational micro-unit rate; quote identity/version and authority are external.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiabilityRate {
    currency: Currency,
    unit: UsageUnit,
    numerator_micros: u128,
    denominator_usage_units: u128,
}

impl LiabilityRate {
    /// Constructs a rate, allowing zero numerator but rejecting `ZeroDenominator`.
    pub fn new(
        currency: Currency,
        unit: UsageUnit,
        numerator_micros: u128,
        denominator_usage_units: u128,
    ) -> Result<Self, MoneyError> {
        if denominator_usage_units == 0 {
            return Err(MoneyError::ZeroDenominator);
        }
        Ok(Self {
            currency,
            unit,
            numerator_micros,
            denominator_usage_units,
        })
    }

    pub fn currency(self) -> Currency {
        self.currency
    }

    pub fn unit(self) -> UsageUnit {
        self.unit
    }

    pub fn numerator_micros(self) -> u128 {
        self.numerator_micros
    }

    /// Returns the positive denominator in usage units.
    pub fn denominator_usage_units(self) -> u128 {
        self.denominator_usage_units
    }

    /// Rounds fractional liability upward to a whole currency micro-unit.
    ///
    /// Rejects `UnitMismatch` first, then `Overflow` if the intermediate product
    /// exceeds `u128`, even when an unbounded rational quotient would fit.
    pub fn liability(self, usage: Usage) -> Result<Money, MoneyError> {
        if self.unit != usage.unit {
            return Err(MoneyError::UnitMismatch);
        }
        let product = usage
            .quantity
            .checked_mul(self.numerator_micros)
            .ok_or(MoneyError::Overflow)?;
        let quotient = product / self.denominator_usage_units;
        let micros = if product.is_multiple_of(self.denominator_usage_units) {
            quotient
        } else {
            // A remainder implies denominator >= 2 and quotient < product, so
            // increment overflow is unreachable after the checked product.
            quotient.checked_add(1).ok_or(MoneyError::Overflow)?
        };
        Ok(Money::new(self.currency, micros))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_tags_preserve_exact_uppercase_ascii_and_refuse_other_inputs() {
        for tag in ["USD", "EUR", "ZZZ"] {
            let currency = Currency::parse(tag).unwrap();
            assert_eq!(currency.as_str(), tag);
            for micros in [0, 1_000_000, u128::MAX] {
                let amount = Money::new(currency, micros);
                assert_eq!(amount.currency(), currency);
                assert_eq!(amount.currency().as_str(), tag);
                assert_eq!(amount.micros(), micros);
            }
        }
        for tag in [
            "",
            "US",
            "USDD",
            "usd",
            "Usd",
            " USD",
            "US ",
            "U\tD",
            "éA",
            "ＵＳＤ",
        ] {
            assert_eq!(
                Currency::parse(tag),
                Err(MoneyError::InvalidCurrencyTag),
                "{tag:?}"
            );
        }
    }

    #[test]
    fn money_add_sub_preserve_currency_and_refuse_mismatch_before_arithmetic() {
        let usd = Currency::parse("USD").unwrap();
        let eur = Currency::parse("EUR").unwrap();
        for currency in [usd, eur] {
            let zero = Money::new(currency, 0);
            let two = Money::new(currency, 2);
            let three = Money::new(currency, 3);
            let maximum = Money::new(currency, u128::MAX);
            assert_eq!(two.checked_add(three), Ok(Money::new(currency, 5)));
            assert_eq!(three.checked_sub(two), Ok(Money::new(currency, 1)));
            assert_eq!(two.checked_sub(two), Ok(zero));
            assert_eq!(zero.checked_add(zero), Ok(zero));
            assert_eq!(zero.checked_sub(zero), Ok(zero));
            assert_eq!(maximum.checked_add(zero), Ok(maximum));
            assert_eq!(maximum.checked_sub(zero), Ok(maximum));
            assert_eq!(
                Money::new(currency, u128::MAX - 1).checked_add(Money::new(currency, 1)),
                Ok(maximum)
            );
            assert_eq!(
                maximum.checked_add(Money::new(currency, 1)),
                Err(MoneyError::Overflow)
            );
            assert_eq!(two.checked_sub(three), Err(MoneyError::Underflow));
        }
        for (left, right) in [(usd, eur), (eur, usd)] {
            assert_eq!(
                Money::new(left, 1).checked_add(Money::new(right, 1)),
                Err(MoneyError::CurrencyMismatch)
            );
            assert_eq!(
                Money::new(left, u128::MAX).checked_add(Money::new(right, 1)),
                Err(MoneyError::CurrencyMismatch)
            );
            assert_eq!(
                Money::new(left, 1).checked_sub(Money::new(right, 0)),
                Err(MoneyError::CurrencyMismatch)
            );
            assert_eq!(
                Money::new(left, 0).checked_sub(Money::new(right, 1)),
                Err(MoneyError::CurrencyMismatch)
            );
        }
    }

    #[test]
    fn money_usage_preserves_every_unit_and_refuses_mismatch_before_overflow() {
        let units = [
            UsageUnit::Token,
            UsageUnit::Character,
            UsageUnit::Byte,
            UsageUnit::AudioMillisecond,
            UsageUnit::VideoMillisecond,
            UsageUnit::Image,
        ];
        for unit in units {
            for quantity in [0, u128::MAX] {
                let usage = Usage::new(quantity, unit);
                assert_eq!(usage.quantity(), quantity);
                assert_eq!(usage.unit(), unit);
                assert_eq!(usage.checked_add(Usage::new(0, unit)), Ok(usage));
            }
            assert_eq!(
                Usage::new(2, unit).checked_add(Usage::new(3, unit)),
                Ok(Usage::new(5, unit))
            );
            assert_eq!(
                Usage::new(u128::MAX - 1, unit).checked_add(Usage::new(1, unit)),
                Ok(Usage::new(u128::MAX, unit))
            );
            assert_eq!(
                Usage::new(u128::MAX, unit).checked_add(Usage::new(1, unit)),
                Err(MoneyError::Overflow)
            );
            for other in units {
                if other != unit {
                    assert_eq!(
                        Usage::new(1, unit).checked_add(Usage::new(1, other)),
                        Err(MoneyError::UnitMismatch)
                    );
                    assert_eq!(
                        Usage::new(u128::MAX, unit).checked_add(Usage::new(1, other)),
                        Err(MoneyError::UnitMismatch)
                    );
                }
            }
        }
    }

    #[test]
    fn money_liability_rounds_up_exactly_and_preserves_rate_accessors() {
        let usd = Currency::parse("USD").unwrap();
        let eur = Currency::parse("EUR").unwrap();
        for currency in [usd, eur] {
            for unit in [
                UsageUnit::Token,
                UsageUnit::Character,
                UsageUnit::Byte,
                UsageUnit::AudioMillisecond,
                UsageUnit::VideoMillisecond,
                UsageUnit::Image,
            ] {
                for (quantity, numerator, denominator, expected) in [
                    (5, 2, 3, 4),
                    (6, 2, 3, 4),
                    (1, 1, 2, 1),
                    (0, u128::MAX, 3, 0),
                    (u128::MAX, 0, 1, 0),
                    (u128::MAX, 1, 1, u128::MAX),
                    (1, u128::MAX, 2, u128::MAX / 2 + 1),
                ] {
                    let rate = LiabilityRate::new(currency, unit, numerator, denominator).unwrap();
                    assert_eq!(rate.currency(), currency);
                    assert_eq!(rate.unit(), unit);
                    assert_eq!(rate.numerator_micros(), numerator);
                    assert_eq!(rate.denominator_usage_units(), denominator);
                    let liability = rate.liability(Usage::new(quantity, unit)).unwrap();
                    assert_eq!(liability.currency().as_str(), currency.as_str());
                    assert_eq!(liability.micros(), expected);
                    assert_eq!(liability, Money::new(currency, expected));
                }
            }
        }
    }

    #[test]
    fn money_rate_refuses_zero_denominator_unit_mismatch_and_intermediate_overflow() {
        let usd = Currency::parse("USD").unwrap();
        for numerator in [0, 1, u128::MAX] {
            // Construction refuses the invalid rate before any zero usage could be applied.
            assert_eq!(
                LiabilityRate::new(usd, UsageUnit::Token, numerator, 0),
                Err(MoneyError::ZeroDenominator)
            );
        }
        let units = [
            UsageUnit::Token,
            UsageUnit::Character,
            UsageUnit::Byte,
            UsageUnit::AudioMillisecond,
            UsageUnit::VideoMillisecond,
            UsageUnit::Image,
        ];
        for unit in units {
            let rate = LiabilityRate::new(usd, unit, 2, 3).unwrap();
            // The mathematical quotient fits, but the bounded intermediate product does not.
            assert_eq!(
                rate.liability(Usage::new(u128::MAX, unit)),
                Err(MoneyError::Overflow)
            );
            let maximum_rate = LiabilityRate::new(usd, unit, u128::MAX, u128::MAX).unwrap();
            assert_eq!(
                maximum_rate.liability(Usage::new(2, unit)),
                Err(MoneyError::Overflow)
            );
            for other in units {
                if other != unit {
                    assert_eq!(
                        rate.liability(Usage::new(1, other)),
                        Err(MoneyError::UnitMismatch)
                    );
                    assert_eq!(
                        rate.liability(Usage::new(u128::MAX, other)),
                        Err(MoneyError::UnitMismatch)
                    );
                    assert_eq!(
                        rate.liability(Usage::new(0, other)),
                        Err(MoneyError::UnitMismatch)
                    );
                }
            }
        }
    }
}
