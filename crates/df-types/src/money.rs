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
#[path = "money_tests.rs"]
mod money_tests;
