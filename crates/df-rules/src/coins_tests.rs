use crate::coins::{CoinError, Coins, Price, smallest_tender};
use crate::local_journey::{SRD_521_BASELINE, selling_value_cp};
use crate::srd_equipment_prices::{SRD_521_EQUIPMENT_PRICES, price_cp};

const VALUES: [u64; 5] = [1, 10, 50, 100, 1000];

/// SplitMix64: a deterministic generator for the property loops, so no external crate is needed.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_1111);
        z ^ (z >> 31)
    }

    /// A value in `0..bound`. `bound` must be non-zero.
    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}

fn purse(cp: u64, sp: u64, ep: u64, gp: u64, pp: u64) -> Coins {
    Coins::new(cp, sp, ep, gp, pp).expect("test purse fits in u64")
}

fn price(copper: u64) -> Price {
    Price::from_copper(copper)
}

fn copper_of(stock: [u64; 5]) -> u64 {
    stock
        .iter()
        .zip(VALUES)
        .map(|(count, value)| count * value)
        .sum()
}

/// Every payment of `owed` from `stock`, found by trying all counts: the least copper paid at or
/// above `owed`, then the fewest coins. Only for small stocks.
fn exhaustive_tender(
    stock: &[u64; 5],
    index: usize,
    paid: u64,
    coins: u64,
    owed: u64,
    best: &mut Option<(u64, u64)>,
) {
    if index == VALUES.len() {
        if paid >= owed && best.is_none_or(|current| (paid, coins) < current) {
            *best = Some((paid, coins));
        }
        return;
    }
    for count in 0..=stock[index] {
        exhaustive_tender(
            stock,
            index + 1,
            paid + count * VALUES[index],
            coins + count,
            owed,
            best,
        );
    }
}

// Conversions and construction.

#[test]
fn each_denomination_is_worth_its_srd_copper_value() {
    let cases = [
        (purse(1, 0, 0, 0, 0), 1),
        (purse(0, 1, 0, 0, 0), 10),
        (purse(0, 0, 1, 0, 0), 50),
        (purse(0, 0, 0, 1, 0), 100),
        (purse(0, 0, 0, 0, 1), 1000),
    ];
    for (coins, value) in cases {
        assert_eq!(coins.total_copper(), Some(value));
        assert_eq!(Coins::from_copper(value), coins);
    }
}

#[test]
fn srd_exchange_rates_hold_between_denominations() {
    // SRD p.89: 2 ep = 1 gp, 10 gp = 1 pp, 5 sp = 1 ep, 10 sp = 1 gp.
    assert_eq!(
        purse(0, 0, 2, 0, 0).total_copper(),
        purse(0, 0, 0, 1, 0).total_copper()
    );
    assert_eq!(
        purse(0, 0, 0, 10, 0).total_copper(),
        purse(0, 0, 0, 0, 1).total_copper()
    );
    assert_eq!(
        purse(0, 5, 0, 0, 0).total_copper(),
        purse(0, 0, 1, 0, 0).total_copper()
    );
    assert_eq!(
        purse(0, 10, 0, 0, 0).total_copper(),
        purse(0, 0, 0, 1, 0).total_copper()
    );
}

#[test]
fn from_copper_splits_into_the_fewest_coins() {
    assert_eq!(Coins::from_copper(0), Coins::default());
    assert_eq!(Coins::from_copper(60), purse(0, 1, 1, 0, 0));
    assert_eq!(Coins::from_copper(1999), purse(9, 4, 1, 9, 1));
    assert_eq!(Coins::from_copper(3750), purse(0, 0, 1, 7, 3));
}

#[test]
fn from_copper_round_trips_the_maximum_total() {
    let max = Coins::from_copper(u64::MAX);
    assert_eq!(max.total_copper(), Some(u64::MAX));
    assert_eq!(max.cp(), 5);
    assert_eq!(max.sp(), 1);
    assert_eq!(max.ep(), 0);
    assert_eq!(max.gp(), 6);
    assert_eq!(max.pp(), u64::MAX / 1000);
}

#[test]
fn new_accepts_a_total_of_exactly_u64_max_and_refuses_one_more() {
    let max = Coins::from_copper(u64::MAX);
    assert_eq!(
        Coins::new(max.cp(), max.sp(), max.ep(), max.gp(), max.pp()),
        Ok(max)
    );
    assert_eq!(
        Coins::new(max.cp() + 1, max.sp(), max.ep(), max.gp(), max.pp()),
        Err(CoinError::Overflow)
    );
}

#[test]
fn new_refuses_a_platinum_count_that_overflows_when_valued() {
    assert_eq!(Coins::new(0, 0, 0, 0, u64::MAX), Err(CoinError::Overflow));
    assert_eq!(
        Coins::new(u64::MAX, 0, 0, 0, 0).map(|coins| coins.cp()),
        Ok(u64::MAX)
    );
    assert_eq!(Coins::new(u64::MAX, 1, 0, 0, 0), Err(CoinError::Overflow));
}

#[test]
fn zero_purse_has_no_value_and_affords_only_free_things() {
    let empty = Coins::default();
    assert_eq!(empty.total_copper(), Some(0));
    assert!(empty.can_afford(price(0)));
    assert!(!empty.can_afford(price(1)));
}

// Addition.

#[test]
fn checked_add_keeps_each_denomination_as_given() {
    let sum = purse(10, 1, 0, 0, 0)
        .checked_add(purse(0, 0, 0, 2, 0))
        .expect("fits");
    assert_eq!(sum, purse(10, 1, 0, 2, 0));
    assert_eq!(sum.total_copper(), Some(220));
}

#[test]
fn checked_add_refuses_a_total_past_u64() {
    let max = Coins::from_copper(u64::MAX);
    assert_eq!(
        max.checked_add(Coins::from_copper(1)),
        Err(CoinError::Overflow)
    );
    assert_eq!(
        Coins::new(u64::MAX, 0, 0, 0, 0)
            .expect("fits")
            .checked_add(purse(0, 0, 0, 0, 0)),
        Ok(Coins::new(u64::MAX, 0, 0, 0, 0).expect("fits"))
    );
}

#[test]
fn checked_add_refuses_a_single_count_past_u64() {
    assert_eq!(
        purse(u64::MAX, 0, 0, 0, 0).checked_add(purse(1, 0, 0, 0, 0)),
        Err(CoinError::Overflow)
    );
}

// Affordability.

#[test]
fn can_afford_compares_the_whole_copper_total() {
    let coins = purse(0, 0, 0, 1, 0);
    assert!(coins.can_afford(price(100)));
    assert!(!coins.can_afford(price(101)));
    assert!(coins.can_afford(price(0)));
}

// Subtraction: exact payment, change, shortfall, zero, limits.

#[test]
fn exact_payment_leaves_nothing_and_needs_no_change() {
    let after = purse(0, 1, 0, 1, 0).subtract(price(110)).expect("exact");
    assert_eq!(after, Coins::default());
}

#[test]
fn exact_payment_uses_mixed_coins_when_they_match() {
    // 100 cp is one electrum plus five silver; the purse has exactly those.
    let after = purse(0, 5, 1, 0, 0).subtract(price(100)).expect("exact");
    assert_eq!(after, Coins::default());
}

#[test]
fn payment_needing_change_returns_the_change_in_coins() {
    // A platinum piece pays 300 cp; 700 cp comes back as 7 gp.
    let after = purse(0, 0, 0, 0, 1).subtract(price(300)).expect("covered");
    assert_eq!(after, purse(0, 0, 0, 7, 0));
}

#[test]
fn an_inexact_payment_takes_the_smallest_sufficient_coins() {
    // 60 cp from one gold and five silver: silver cannot make 60 exactly, so the gold piece
    // (100 cp) is handed over and 40 cp comes back as four silver. The remaining five silver
    // are kept as they were.
    let after = purse(0, 5, 0, 1, 0).subtract(price(60)).expect("covered");
    assert_eq!(after, purse(0, 9, 0, 0, 0));
    assert_eq!(after.total_copper(), Some(90));
}

#[test]
fn equal_amounts_are_paid_with_the_fewest_coins() {
    // One gold (100 cp, 1 coin) and two electrum (100 cp, 2 coins) both cover 60 cp with 100 cp.
    // The gold piece is handed over, so the two electrum stay and 40 cp returns as four silver.
    let after = purse(0, 0, 2, 1, 0).subtract(price(60)).expect("covered");
    assert_eq!(after, purse(0, 4, 2, 0, 0));
}

#[test]
fn a_platinum_piece_pays_a_small_price_with_change_in_gold_and_silver() {
    let after = purse(0, 0, 0, 0, 1).subtract(price(1)).expect("covered");
    assert_eq!(after, purse(9, 4, 1, 9, 0));
    assert_eq!(after.total_copper(), Some(999));
}

#[test]
fn insufficient_funds_fail_with_both_amounts_and_change_nothing() {
    let purse_before = purse(0, 0, 0, 1, 0);
    assert_eq!(
        purse_before.subtract(price(101)),
        Err(CoinError::Insufficient {
            cost_cp: 101,
            purse_cp: 100,
        })
    );
    assert_eq!(purse_before, purse(0, 0, 0, 1, 0));
}

#[test]
fn a_zero_cost_returns_the_purse_unchanged_even_when_not_in_lowest_terms() {
    let odd = purse(12, 0, 0, 0, 0);
    assert_eq!(odd.subtract(price(0)), Ok(odd));
}

#[test]
fn an_exact_payment_keeps_the_rest_of_the_purse_as_it_was() {
    let after = purse(12, 0, 0, 0, 0).subtract(price(2)).expect("exact");
    assert_eq!(after, purse(10, 0, 0, 0, 0));
}

#[test]
fn paying_a_price_of_u64_max_from_a_purse_of_u64_max_leaves_nothing() {
    let max = Coins::from_copper(u64::MAX);
    assert_eq!(max.subtract(price(u64::MAX)), Ok(Coins::default()));
    let after = max.subtract(price(1)).expect("covered");
    assert_eq!(after.total_copper(), Some(u64::MAX - 1));
}

#[test]
fn a_large_platinum_stock_pays_a_small_price_without_overflow() {
    let big = purse(0, 0, 0, 0, 1_000_000_000_000);
    let after = big.subtract(price(1)).expect("covered");
    assert_eq!(after.total_copper(), Some(1_000_000_000_000_000 - 1));
}

// Tender search against an exhaustive search.

#[test]
fn smallest_tender_matches_an_exhaustive_search_on_small_purses() {
    let mut rng = Rng(7);
    for case in 0..6000 {
        let stock: [u64; 5] = core::array::from_fn(|_| rng.below(5));
        let total = copper_of(stock);
        let owed = rng.below(total + 60);
        let mut expected = None;
        exhaustive_tender(&stock, 0, 0, 0, owed, &mut expected);
        let found = smallest_tender(&stock, VALUES.len(), owed)
            .expect("small purses never overflow")
            .map(|tender| (tender.paid_cp, tender.coins));
        assert_eq!(found, expected, "case {case}: stock {stock:?}, owed {owed}");
    }
}

#[test]
fn tender_uses_no_more_of_any_denomination_than_the_purse_holds() {
    let mut rng = Rng(11);
    for _ in 0..4000 {
        let stock: [u64; 5] = core::array::from_fn(|_| rng.below(6));
        let owed = rng.below(copper_of(stock) + 1);
        let tender = smallest_tender(&stock, VALUES.len(), owed)
            .expect("small purses never overflow")
            .expect("owed is within the purse");
        for (handed, held) in tender.handed.iter().zip(stock) {
            assert!(handed <= &held, "stock {stock:?}, owed {owed}");
        }
        assert!(tender.paid_cp >= owed);
    }
}

// Conservation over many payments: purse before = purse after + cost, nothing lost.

#[test]
fn subtract_conserves_value_over_thousands_of_payments() {
    let mut rng = Rng(0x5EED);
    let mut paid = 0_u64;
    let mut refused = 0_u64;
    for _ in 0..6000 {
        let stock: [u64; 5] = core::array::from_fn(|_| match rng.below(3) {
            0 => rng.below(4),
            1 => rng.below(300),
            _ => rng.below(1_000_000_000_000),
        });
        let purse = Coins::new(stock[0], stock[1], stock[2], stock[3], stock[4])
            .expect("stocks here stay far below u64");
        let total = purse.total_copper().expect("fits");
        let cost = rng.below(total + total / 2 + 2);
        let outcome = purse.subtract(price(cost));
        if cost <= total {
            let after = outcome.expect("covered payment succeeds");
            assert_eq!(
                after.total_copper().expect("fits"),
                total - cost,
                "stock {stock:?}, cost {cost}"
            );
            paid += 1;
        } else {
            assert_eq!(
                outcome,
                Err(CoinError::Insufficient {
                    cost_cp: cost,
                    purse_cp: total,
                })
            );
            refused += 1;
        }
    }
    assert!(
        paid > 1000 && refused > 1000,
        "both branches exercised: {paid} paid, {refused} refused"
    );
}

#[test]
fn from_copper_is_canonical_and_exact_over_random_values() {
    let mut rng = Rng(0xC0FFEE);
    for _ in 0..6000 {
        let value = rng.next_u64() >> rng.below(64);
        let coins = Coins::from_copper(value);
        assert_eq!(coins.total_copper(), Some(value));
        assert!(coins.cp() < 10 && coins.sp() < 5 && coins.ep() < 2 && coins.gp() < 10);
    }
}

// The SRD price table and its consistency with the in-game sale rule.

#[test]
fn price_lookup_returns_the_srd_copper_price_for_each_priced_baseline_item() {
    let expected = [
        ("chain-mail", 7_500),
        ("greatsword", 5_000),
        ("flail", 1_000),
        ("javelin", 50),
        ("spear", 100),
        ("shortbow", 2_500),
        ("arrow", 5),
        ("dungeoneer-pack", 1_200),
        ("healers-kit", 500),
        ("quiver", 100),
        ("travelers-clothes", 200),
    ];
    for (id, copper) in expected {
        assert_eq!(price_cp(id), Some(price(copper)), "{id}");
    }
}

#[test]
fn price_lookup_returns_none_for_unknown_and_unpriced_ids() {
    for id in [
        "",
        "dragon-hoard",
        "Chain-Mail",
        "longsword",
        "gaming-set",
        "gold-piece",
    ] {
        assert_eq!(price_cp(id), None, "{id}");
    }
}

#[test]
fn every_baseline_item_is_priced_except_the_two_without_a_single_srd_price() {
    for (id, _) in SRD_521_BASELINE.equipment {
        let unpriced = matches!(*id, "gaming-set" | "gold-piece");
        assert_eq!(price_cp(id).is_none(), unpriced, "{id}");
    }
}

#[test]
fn the_price_table_has_no_duplicate_ids() {
    for (index, entry) in SRD_521_EQUIPMENT_PRICES.iter().enumerate() {
        assert!(
            SRD_521_EQUIPMENT_PRICES[index + 1..]
                .iter()
                .all(|other| other.id != entry.id),
            "{}",
            entry.id
        );
    }
}

#[test]
fn twenty_arrows_cost_one_gold_piece() {
    let arrow = price_cp("arrow").expect("arrow is priced").copper();
    assert_eq!(
        arrow * 20,
        purse(0, 0, 0, 1, 0).total_copper().expect("fits")
    );
}

#[test]
fn sale_value_is_consistent_with_selling_value_cp_for_every_priced_item() {
    for entry in SRD_521_EQUIPMENT_PRICES {
        let cost = entry.price.copper();
        let cost_cp = u32::try_from(cost).expect("SRD prices fit in u32");
        assert_eq!(
            u64::from(selling_value_cp(cost_cp, false)),
            cost / 2,
            "{}",
            entry.id
        );
        assert_eq!(
            u64::from(selling_value_cp(cost_cp, true)),
            cost,
            "{}",
            entry.id
        );
    }
    // Chain mail (75 gp, SRD p.92) sells for half: 37 gp 5 sp, which is 3 pp, 7 gp and 1 ep.
    let chain_sale = selling_value_cp(7_500, false);
    assert_eq!(
        Coins::from_copper(u64::from(chain_sale)),
        purse(0, 0, 1, 7, 3)
    );
    // A 5 cp arrow sells for 2 cp: the half of a copper piece rounds down (SRD p.89).
    assert_eq!(selling_value_cp(5, false), 2);
}

// Serde.

#[test]
fn coins_round_trip_through_json_keeping_each_denomination() {
    let coins = purse(3, 1, 2, 7, 4);
    let json = serde_json::to_string(&coins).expect("serializes");
    assert_eq!(json, r#"{"cp":3,"sp":1,"ep":2,"gp":7,"pp":4}"#);
    let back: Coins = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, coins);
}

#[test]
fn deserializing_a_purse_past_u64_is_refused() {
    let json = r#"{"cp":0,"sp":0,"ep":0,"gp":0,"pp":18446744073709551615}"#;
    assert!(serde_json::from_str::<Coins>(json).is_err());
}

#[test]
fn deserializing_a_purse_with_an_unknown_denomination_is_refused() {
    let json = r#"{"cp":0,"sp":0,"ep":0,"gp":0,"pp":0,"xp":1}"#;
    assert!(serde_json::from_str::<Coins>(json).is_err());
}

#[test]
fn price_round_trips_through_json() {
    let cost = price(7_500);
    let json = serde_json::to_string(&cost).expect("serializes");
    let back: Price = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, cost);
}

// Error text.

#[test]
fn coin_errors_read_in_plain_language() {
    let short = CoinError::Insufficient {
        cost_cp: 200,
        purse_cp: 150,
    }
    .to_string();
    assert!(
        short.contains("200 cp") && short.contains("150 cp"),
        "{short}"
    );
    let big = CoinError::Overflow.to_string();
    assert!(big.contains("more than a purse can hold"), "{big}");
}
