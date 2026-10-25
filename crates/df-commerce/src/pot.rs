//! The exact split of an amount over weights (planning/finance-ledger-research.md 8.3): the one
//! production function every test of the pot split calls.
//!
//! `split_exactly` gives each weight `floor(amount * w / W)` and then hands the remainder out one
//! micro-dollar at a time to the largest fractional remainders, ties to the lowest key. The shares
//! sum to the amount exactly, and no share is a full micro-dollar above its exact value.
//! `split_capped` water-fills at exhausted payers: a capped share never exceeds its cap and the
//! uncovered part is split among the others by the same rule, repeatedly.

use std::cmp::Reverse;

/// Why an amount could not be split.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplitRefusal {
    /// No weights, or every weight is zero.
    NoWeight,
    /// The caps together cannot cover the amount.
    CapsTooSmall,
    Overflow,
}

/// Splits `amount` over `weights`, returning each key with its share, in the input order.
/// Keys only break ties (lowest first); they must be distinct.
pub fn split_exactly<K: Copy + Ord>(
    amount: u64,
    weights: &[(K, u64)],
) -> Result<Vec<(K, u64)>, SplitRefusal> {
    let total: u128 = weights.iter().map(|(_, weight)| u128::from(*weight)).sum();
    if total == 0 {
        return Err(SplitRefusal::NoWeight);
    }
    let amount = u128::from(amount);
    let mut shares: Vec<(K, u128, u128)> = weights
        .iter()
        .map(|(key, weight)| {
            let product = amount * u128::from(*weight);
            (*key, product / total, product % total)
        })
        .collect();
    let assigned: u128 = shares.iter().map(|(_, share, _)| *share).sum();
    let mut remaining = amount - assigned;
    let mut order: Vec<usize> = (0..shares.len()).collect();
    order.sort_by_key(|&index| (Reverse(shares[index].2), shares[index].0));
    for index in order {
        if remaining == 0 {
            break;
        }
        // A zero weight has a zero remainder, so it is only reached when every positive
        // remainder has been served, which cannot leave anything over.
        shares[index].1 += 1;
        remaining -= 1;
    }
    shares
        .into_iter()
        .map(|(key, share, _)| {
            u64::try_from(share)
                .map(|share| (key, share))
                .map_err(|_| SplitRefusal::Overflow)
        })
        .collect()
}

/// Splits `amount` over `(key, weight, cap)` so that no share exceeds its cap: water-filling.
/// Capped payers keep their cap; the rest is split among the uncapped ones by weight, and the
/// pass repeats until nobody exceeds a cap.
pub fn split_capped<K: Copy + Ord>(
    amount: u64,
    weights: &[(K, u64, u64)],
) -> Result<Vec<(K, u64)>, SplitRefusal> {
    let capacity: u128 = weights.iter().map(|(_, _, cap)| u128::from(*cap)).sum();
    if capacity < u128::from(amount) {
        return Err(SplitRefusal::CapsTooSmall);
    }
    let mut fixed: Vec<Option<u64>> = vec![None; weights.len()];
    loop {
        let fixed_total: u64 = fixed.iter().flatten().sum();
        let open: Vec<(usize, K, u64)> = weights
            .iter()
            .enumerate()
            .filter(|(index, _)| fixed[*index].is_none())
            .map(|(index, (key, weight, _))| (index, *key, *weight))
            .collect();
        let rest = amount.saturating_sub(fixed_total);
        if open.is_empty() {
            break;
        }
        let split = split_exactly(
            rest,
            &open
                .iter()
                .map(|(_, key, weight)| (*key, *weight))
                .collect::<Vec<_>>(),
        )?;
        let mut over = false;
        for ((index, _, _), (_, share)) in open.iter().zip(&split) {
            let cap = weights[*index].2;
            if *share > cap {
                fixed[*index] = Some(cap);
                over = true;
            }
        }
        if !over {
            let mut out = Vec::with_capacity(weights.len());
            let mut shares = split.into_iter();
            for (index, (key, _, _)) in weights.iter().enumerate() {
                match fixed[index] {
                    Some(cap) => out.push((*key, cap)),
                    None => {
                        let (_, share) = shares.next().ok_or(SplitRefusal::NoWeight)?;
                        out.push((*key, share));
                    }
                }
            }
            return Ok(out);
        }
    }
    Err(SplitRefusal::CapsTooSmall)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plans_three_contributor_example_splits_eight_dollars_four_two_two() {
        let shares = split_exactly(8_000_000, &[(1_u8, 20), (2, 10), (3, 10)]).expect("split");
        assert_eq!(shares, vec![(1, 4_000_000), (2, 2_000_000), (3, 2_000_000)]);
    }

    #[test]
    fn a_thousand_one_micro_charges_across_three_equal_payers_lose_nothing() {
        let mut totals = [0_u64; 3];
        for _ in 0..1001 {
            let shares = split_exactly(1, &[(0_u8, 1), (1, 1), (2, 1)]).expect("split");
            for (key, share) in shares {
                totals[usize::from(key)] += share;
            }
        }
        assert_eq!(totals.iter().sum::<u64>(), 1001);
    }

    #[test]
    fn ties_go_to_the_lowest_key() {
        let shares = split_exactly(1, &[(7_u8, 1), (3, 1), (5, 1)]).expect("split");
        assert_eq!(shares, vec![(7, 0), (3, 1), (5, 0)]);
    }

    #[test]
    fn a_capped_payer_keeps_its_cap_and_the_others_cover_the_rest() {
        let shares = split_capped(
            900_000,
            &[(1_u8, 1, 100_000), (2, 1, 1_000_000), (3, 1, 1_000_000)],
        )
        .expect("split");
        assert_eq!(shares, vec![(1, 100_000), (2, 400_000), (3, 400_000)]);
    }

    #[test]
    fn caps_that_cannot_cover_the_amount_are_refused() {
        assert_eq!(
            split_capped(10, &[(1_u8, 1, 3), (2, 1, 3)]),
            Err(SplitRefusal::CapsTooSmall)
        );
    }

    #[test]
    fn no_weight_is_refused() {
        assert_eq!(split_exactly(5, &[(1_u8, 0)]), Err(SplitRefusal::NoWeight));
        assert_eq!(split_exactly::<u8>(5, &[]), Err(SplitRefusal::NoWeight));
    }

    #[test]
    fn splits_sum_exactly_and_never_raise_a_share_a_full_micro_dollar_over_its_exact_value() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..2000 {
            let count = usize::try_from(next() % 6 + 1).expect("small");
            let weights: Vec<(u8, u64)> = (0..count)
                .map(|i| (u8::try_from(i).expect("small"), next() % 50 + 1))
                .collect();
            let amount = next() % 10_000_000;
            let total: u128 = weights.iter().map(|(_, w)| u128::from(*w)).sum();
            let shares = split_exactly(amount, &weights).expect("split");
            assert_eq!(shares.iter().map(|(_, s)| *s).sum::<u64>(), amount);
            for ((_, weight), (_, share)) in weights.iter().zip(&shares) {
                let exact = u128::from(amount) * u128::from(*weight);
                assert!(
                    u128::from(*share) * total < exact + total,
                    "share above exact + 1"
                );
                assert!(
                    u128::from(*share) * total + total > exact,
                    "share below exact - 1"
                );
            }
        }
    }
}
