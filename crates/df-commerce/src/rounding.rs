//! The rounding rules of planning/finance-ledger-research.md 4, as functions.
//!
//! All arithmetic is integer: a rate is a ratio, an amount is a whole number of micro-dollars, and
//! each function names its direction. Nothing here uses a float.

/// A rational amount of micro-dollars: `numerator / denominator`, in lowest terms or not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ratio {
    pub numerator: u128,
    pub denominator: u128,
}

/// A failed conversion back to whole micro-dollars.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoundingError {
    ZeroDenominator,
    Overflow,
}

impl Ratio {
    pub fn new(numerator: u128, denominator: u128) -> Result<Self, RoundingError> {
        if denominator == 0 {
            return Err(RoundingError::ZeroDenominator);
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    /// Rule 4.1: a retail maximum (a hold) rounds up.
    pub fn round_up(self) -> Result<u64, RoundingError> {
        let quotient = self.numerator / self.denominator;
        let rounded = quotient + u128::from(!self.numerator.is_multiple_of(self.denominator));
        u64::try_from(rounded).map_err(|_| RoundingError::Overflow)
    }

    /// Rule 4.2: a retail charge rounds down, once.
    pub fn round_down(self) -> Result<u64, RoundingError> {
        u64::try_from(self.numerator / self.denominator).map_err(|_| RoundingError::Overflow)
    }

    /// Rule 4.3: a supplier cost rounds half up, once.
    pub fn round_half_up(self) -> Result<u64, RoundingError> {
        let doubled = self
            .numerator
            .checked_mul(2)
            .ok_or(RoundingError::Overflow)?;
        let rounded = (doubled + self.denominator) / (self.denominator * 2);
        u64::try_from(rounded).map_err(|_| RoundingError::Overflow)
    }

    /// The exact sum of two ratios (the lines of one operation are summed before rounding).
    pub fn checked_add(self, other: Self) -> Result<Self, RoundingError> {
        let denominator = self
            .denominator
            .checked_mul(other.denominator)
            .ok_or(RoundingError::Overflow)?;
        let numerator = self
            .numerator
            .checked_mul(other.denominator)
            .and_then(|left| {
                other
                    .numerator
                    .checked_mul(self.denominator)
                    .and_then(|right| left.checked_add(right))
            })
            .ok_or(RoundingError::Overflow)?;
        Ok(Self {
            numerator,
            denominator,
        }
        .reduced())
    }

    /// The ratio in lowest terms, so a long sum of lines keeps small numbers.
    pub fn reduced(self) -> Self {
        let divisor = gcd(self.numerator, self.denominator).max(1);
        Self {
            numerator: self.numerator / divisor,
            denominator: self.denominator / divisor,
        }
    }

    /// This ratio times `numerator / denominator` (a markup).
    pub fn checked_scale(self, numerator: u128, denominator: u128) -> Result<Self, RoundingError> {
        if denominator == 0 {
            return Err(RoundingError::ZeroDenominator);
        }
        let product_numerator = self
            .numerator
            .checked_mul(numerator)
            .ok_or(RoundingError::Overflow)?;
        let product_denominator = self
            .denominator
            .checked_mul(denominator)
            .ok_or(RoundingError::Overflow)?;
        Ok(Self {
            numerator: product_numerator,
            denominator: product_denominator,
        }
        .reduced())
    }
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

/// Rule 4.4: the table-time charge, cumulative so remainders carry: the amount owed after
/// `priced_ms` of metered time at `rate_micro_dollars_per_hour` is
/// `floor(priced_ms * rate / 3_600_000)`.
pub fn cumulative_time_charge(
    priced_ms: u64,
    rate_micro_dollars_per_hour: u64,
) -> Result<u64, RoundingError> {
    Ratio::new(
        u128::from(priced_ms) * u128::from(rate_micro_dollars_per_hour),
        3_600_000,
    )?
    .round_down()
}

/// Whole cents of `micro_dollars`, rounded half up (rule 4.10).
pub fn cents_half_up(micro_dollars: i128) -> i128 {
    let magnitude = micro_dollars.unsigned_abs();
    let cents = i128::try_from((magnitude + 5_000) / 10_000).unwrap_or(i128::MAX);
    if micro_dollars < 0 { -cents } else { cents }
}

/// Rule 4.10: the day's cents of an account are `R(cumulative through the day) - R(cumulative
/// through the previous day)`, so the days of a month sum to the month's cents.
pub fn daily_cents(cumulative_micro_dollars_by_day: &[i128]) -> Vec<i128> {
    let mut previous = 0;
    cumulative_micro_dollars_by_day
        .iter()
        .map(|cumulative| {
            let rounded = cents_half_up(*cumulative);
            let day = rounded - previous;
            previous = rounded;
            day
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retail_maximums_round_up_and_charges_round_down() {
        let ratio = Ratio::new(100, 3).expect("ratio");
        assert_eq!(ratio.round_up(), Ok(34));
        assert_eq!(ratio.round_down(), Ok(33));
        assert_eq!(Ratio::new(99, 3).expect("ratio").round_up(), Ok(33));
    }

    #[test]
    fn supplier_cost_rounds_half_up_once() {
        assert_eq!(Ratio::new(5, 10).expect("ratio").round_half_up(), Ok(1));
        assert_eq!(Ratio::new(4, 10).expect("ratio").round_half_up(), Ok(0));
        assert_eq!(Ratio::new(15, 10).expect("ratio").round_half_up(), Ok(2));
    }

    #[test]
    fn a_zero_denominator_is_refused() {
        assert_eq!(Ratio::new(1, 0), Err(RoundingError::ZeroDenominator));
    }

    #[test]
    fn the_cumulative_time_meter_never_charges_more_than_the_exact_amount_for_any_split_of_time() {
        let rate = 2_000_000;
        for total_ms in [1_u64, 999, 60_000, 123_456, 3_599_999, 3_600_000, 7_000_001] {
            for piece in [1_u64, 7, 1000, 42_000, 60_000] {
                let mut charged = 0;
                let mut priced = 0;
                while priced < total_ms {
                    priced = (priced + piece).min(total_ms);
                    let owed = cumulative_time_charge(priced, rate).expect("charge");
                    charged = owed;
                }
                let exact = u128::from(total_ms) * u128::from(rate);
                assert!(
                    u128::from(charged) * 3_600_000 <= exact,
                    "charged above the exact amount"
                );
                assert!(
                    u128::from(charged + 1) * 3_600_000 > exact,
                    "charged more than one micro-dollar below"
                );
            }
        }
    }

    #[test]
    fn the_cents_of_each_day_sum_to_the_cents_of_the_month() {
        let cumulative = [3_333_i128, 9_999, 14_999, 25_001, 25_004];
        let days = daily_cents(&cumulative);
        assert_eq!(days.iter().sum::<i128>(), cents_half_up(25_004));
    }

    #[test]
    fn cents_round_half_up_in_both_signs() {
        assert_eq!(cents_half_up(5_000), 1);
        assert_eq!(cents_half_up(4_999), 0);
        assert_eq!(cents_half_up(-5_000), -1);
    }
}
