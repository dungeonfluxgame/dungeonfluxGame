/// Asserts the plan's section 5.4 allocation contract for splitting one charge by weights.
///
/// `shares[i]` is payer `i`'s part of `charge` (integer micro-units) for authorization
/// `weights[i]`. The contract: the shares sum exactly to the charge, and each share is its exact
/// proportion rounded down or up (so a payer with no authorization pays nothing). The tie-breaker
/// is the owner's choice but must be deterministic.
///
/// # Panics
///
/// Panics with the offending case when the contract is broken, or when `weights` sum to zero,
/// which cannot be allocated at all.
pub fn assert_allocation_contract(charge: u128, weights: &[u128], shares: &[u128]) {
    let case = format!("charge={charge} weights={weights:?} shares={shares:?}");
    assert_eq!(weights.len(), shares.len(), "one share per payer: {case}");
    let total_weight: u128 = weights.iter().sum();
    assert!(total_weight > 0, "weights must authorize something: {case}");
    assert_eq!(
        shares.iter().sum::<u128>(),
        charge,
        "shares must sum exactly to the charge: {case}"
    );
    for (weight, share) in weights.iter().zip(shares) {
        let scaled = charge
            .checked_mul(*weight)
            .expect("the checked cases stay within u128");
        let floor = scaled / total_weight;
        let ceiling = floor + u128::from(!scaled.is_multiple_of(total_weight));
        assert!(
            (floor..=ceiling).contains(share),
            "a share must be its exact proportion rounded down or up: {case}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A labelled reference of the plan's rule (5.4), used only to show that the checker accepts
    /// correct splits and rejects the naive roundings. It is not the production allocator.
    fn reference_largest_remainder(charge: u128, weights: &[u128]) -> Vec<u128> {
        let total: u128 = weights.iter().sum();
        let mut shares: Vec<u128> = weights.iter().map(|w| charge * w / total).collect();
        let mut leftover = charge - shares.iter().sum::<u128>();
        let mut by_remainder: Vec<usize> = (0..weights.len()).collect();
        by_remainder.sort_by_key(|&i| (std::cmp::Reverse(charge * weights[i] % total), i));
        for index in by_remainder {
            if leftover == 0 {
                break;
            }
            if weights[index] > 0 {
                shares[index] += 1;
                leftover -= 1;
            }
        }
        shares
    }

    #[test]
    fn the_checker_accepts_the_plans_three_contributor_split_of_eight_dollars() {
        let weights = [20_000_000, 10_000_000, 10_000_000];

        let shares = reference_largest_remainder(8_000_000, &weights);

        assert_eq!(shares, [4_000_000, 2_000_000, 2_000_000]);
        assert_allocation_contract(8_000_000, &weights, &shares);
    }

    #[test]
    fn the_checker_accepts_a_correct_split_of_every_small_charge_and_weight_mix() {
        for charge in 0..=60_u128 {
            for a in 0..=6_u128 {
                for b in 0..=6_u128 {
                    for c in 0..=6_u128 {
                        let weights = [a, b, c, 3];
                        let shares = reference_largest_remainder(charge, &weights);
                        assert_allocation_contract(charge, &weights, &shares);
                    }
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "shares must sum exactly to the charge")]
    fn the_checker_rejects_rounding_each_share_to_nearest_because_it_drops_the_charge() {
        let shares: Vec<u128> = [1_u128, 1, 1].iter().map(|w| (w * 2 + 1) / 6).collect();

        assert_allocation_contract(1, &[1, 1, 1], &shares);
    }

    #[test]
    #[should_panic(expected = "shares must sum exactly to the charge")]
    fn the_checker_rejects_rounding_each_share_up_because_it_raises_the_table_total() {
        let shares: Vec<u128> = [1_u128, 1, 1].iter().map(|w| w.div_ceil(3)).collect();

        assert_allocation_contract(1, &[1, 1, 1], &shares);
    }

    #[test]
    #[should_panic(expected = "exact proportion rounded down or up")]
    fn the_checker_rejects_a_charge_to_a_payer_with_no_authorization() {
        assert_allocation_contract(2, &[1, 0], &[1, 1]);
    }
}
