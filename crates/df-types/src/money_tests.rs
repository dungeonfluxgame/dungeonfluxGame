use super::*;

const ALL_UNITS: [UsageUnit; 6] = [
    UsageUnit::Token,
    UsageUnit::Character,
    UsageUnit::Byte,
    UsageUnit::AudioMillisecond,
    UsageUnit::VideoMillisecond,
    UsageUnit::Image,
];

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
    for unit in ALL_UNITS {
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
        for other in ALL_UNITS {
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
        for unit in ALL_UNITS {
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
    for unit in ALL_UNITS {
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
        for other in ALL_UNITS {
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
