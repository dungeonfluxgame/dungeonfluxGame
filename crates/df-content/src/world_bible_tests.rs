//! Tests of the world bible: validation and the price math.
//!
//! TODO(DEV-12): a property test that `clamp_quote` stays inside the quote for every proposal.

use serde_json::{Value, json};

use crate::world_bible::{
    BibleError, BulkTier, Item, QuoteError, Stall, WorldBible, clamp_quote, quote,
};

fn good() -> Value {
    json!({
        "items": [
            {"id": "apple", "name": "apple", "unit": "each", "base_value_cp": 3},
            {"id": "pear", "name": "pear", "unit": "each", "base_value_cp": 4, "tags": ["fruit"]}
        ],
        "places": [{
            "id": "market", "name": "Market",
            "stalls": [{
                "keeper": "fruit_vendor", "sells": ["apple", "pear"],
                "bulk": [{"min_qty": 5, "discount_permille": 100}],
                "haggle_floor_permille": 850
            }]
        }],
        "voice_packs": [{"id": "market-trader", "description": "brisk"}]
    })
}

fn parse(value: &Value) -> Result<WorldBible, BibleError> {
    WorldBible::from_json_str(&value.to_string())
}

fn item(base: u64) -> Item {
    Item {
        id: "thing".to_owned(),
        name: "thing".to_owned(),
        unit: "each".to_owned(),
        base_value_cp: base,
        tags: Vec::new(),
    }
}

fn stall(bulk: &[(u32, u32)], floor: u32) -> Stall {
    Stall {
        keeper: "k".to_owned(),
        sells: vec!["thing".to_owned()],
        bulk: bulk
            .iter()
            .map(|&(min_qty, discount_permille)| BulkTier {
                min_qty,
                discount_permille,
                posted: false,
            })
            .collect(),
        haggle_floor_permille: floor,
    }
}

#[test]
fn a_good_bible_reads_and_an_empty_one_is_valid() {
    let bible = parse(&good()).unwrap();
    assert_eq!(bible.items.len(), 2);
    assert!(bible.place("market").is_some());
    assert!(bible.voice_pack("market-trader").is_some());
    assert_eq!(parse(&json!({})).unwrap(), WorldBible::default());
}

#[test]
fn text_that_is_not_a_bible_is_a_typed_error_with_a_position() {
    let Err(BibleError::NotABible { line, .. }) = WorldBible::from_json_str("{\n\"items\": [")
    else {
        panic!("not a bible");
    };
    assert!(line >= 2);
    let mut unknown = good();
    unknown["items"][0]["colour"] = json!("red");
    assert_eq!(parse(&unknown).unwrap_err().key(), "not_a_bible");
}

#[test]
fn coins_read_with_their_copper_value_and_a_repeated_or_worthless_one_is_refused() {
    let mut bible = good();
    bible["coins"] = json!([
        {"id": "gp", "name": "gold piece", "value_cp": 100},
        {"id": "cp", "name": "copper piece", "value_cp": 1}
    ]);
    assert_eq!(parse(&bible).unwrap().coins[0].value_cp, 100);
    let mut twice = bible.clone();
    twice["coins"][1]["id"] = json!("gp");
    assert_eq!(parse(&twice).unwrap_err().key(), "duplicate_id");
    let mut worthless = bible;
    worthless["coins"][0]["value_cp"] = json!(0);
    assert_eq!(parse(&worthless).unwrap_err().key(), "out_of_range");
}

#[test]
fn an_amount_is_said_in_the_largest_coins_that_fit_or_not_at_all_without_coins() {
    let mut bible = good();
    bible["coins"] = json!([
        {"id": "cp", "name": "copper", "value_cp": 1},
        {"id": "gp", "name": "gold", "value_cp": 100},
        {"id": "sp", "name": "silver", "value_cp": 10}
    ]);
    let bible = parse(&bible).unwrap();
    assert_eq!(
        bible.spoken_amount(60850).as_deref(),
        Some("608 gold and 5 silver")
    );
    assert_eq!(bible.spoken_amount(45000).as_deref(), Some("450 gold"));
    assert_eq!(bible.spoken_amount(7).as_deref(), Some("7 copper"));
    assert_eq!(bible.spoken_amount(0), None);
    assert_eq!(parse(&good()).unwrap().spoken_amount(450), None);
}

#[test]
fn ids_are_lowercase_letters_digits_underscore_and_hyphen_and_unique() {
    for bad in ["", "Apple", "a b", "a.b", &"x".repeat(65)] {
        let mut bible = good();
        bible["items"][0]["id"] = json!(bad);
        assert_eq!(parse(&bible).unwrap_err().key(), "bad_id", "{bad:?}");
    }
    let mut twice = good();
    twice["items"][1]["id"] = json!("apple");
    assert_eq!(parse(&twice).unwrap_err().key(), "duplicate_id");
    let mut places = good();
    let first = places["places"][0].clone();
    places["places"].as_array_mut().unwrap().push(first);
    assert_eq!(parse(&places).unwrap_err().key(), "duplicate_id");
    let mut packs = good();
    let first = packs["voice_packs"][0].clone();
    packs["voice_packs"].as_array_mut().unwrap().push(first);
    assert_eq!(parse(&packs).unwrap_err().key(), "duplicate_id");
}

#[test]
fn a_stall_that_sells_an_unknown_item_or_repeats_one_is_refused() {
    let mut dangling = good();
    dangling["places"][0]["stalls"][0]["sells"] = json!(["apple", "plum"]);
    assert_eq!(
        parse(&dangling).unwrap_err(),
        BibleError::Dangling {
            path: "places[0].stalls[0].sells".to_owned(),
            list: "items".to_owned(),
            id: "plum".to_owned(),
        }
    );
    let mut repeated = good();
    repeated["places"][0]["stalls"][0]["sells"] = json!(["apple", "apple"]);
    assert_eq!(parse(&repeated).unwrap_err().key(), "repeated");
    let mut nothing = good();
    nothing["places"][0]["stalls"][0]["sells"] = json!([]);
    assert_eq!(parse(&nothing).unwrap_err().key(), "empty_text");
    let mut keeper = good();
    keeper["places"][0]["stalls"][0]["keeper"] = json!("not a persona");
    assert_eq!(parse(&keeper).unwrap_err().key(), "bad_keeper");
}

#[test]
fn ranges_must_be_sane() {
    let cases = [
        ("items/0/base_value_cp", json!(0)),
        ("places/0/stalls/0/haggle_floor_permille", json!(0)),
        ("places/0/stalls/0/haggle_floor_permille", json!(1001)),
        ("places/0/stalls/0/bulk/0/min_qty", json!(1)),
        ("places/0/stalls/0/bulk/0/discount_permille", json!(0)),
        ("places/0/stalls/0/bulk/0/discount_permille", json!(1000)),
    ];
    for (pointer, value) in cases {
        let mut bible = good();
        *bible.pointer_mut(&format!("/{pointer}")).unwrap() = value.clone();
        assert_eq!(
            parse(&bible).unwrap_err().key(),
            "out_of_range",
            "{pointer} = {value}"
        );
    }
    let mut blank = good();
    blank["items"][0]["name"] = json!("  ");
    assert_eq!(parse(&blank).unwrap_err().key(), "empty_text");
}

#[test]
fn bulk_tiers_ascend_and_never_cost_more_per_unit_for_a_bigger_lot() {
    for tiers in [
        json!([
            {"min_qty": 10, "discount_permille": 100},
            {"min_qty": 5, "discount_permille": 200}
        ]),
        json!([{"min_qty": 5, "discount_permille": 100}, {"min_qty": 5, "discount_permille": 200}]),
        json!([
            {"min_qty": 5, "discount_permille": 200},
            {"min_qty": 10, "discount_permille": 100}
        ]),
    ] {
        let mut bible = good();
        bible["places"][0]["stalls"][0]["bulk"] = tiers;
        assert_eq!(parse(&bible).unwrap_err().key(), "not_ascending");
    }
}

#[test]
fn a_single_unit_costs_its_base_value() {
    let q = quote(&item(3), 1, &stall(&[(5, 100)], 850)).unwrap();
    assert_eq!((q.ask_each_cp, q.ask_total_cp, q.tier_total_cp), (3, 3, 3));
    assert_eq!((q.discount_permille, q.floor_total_cp), (0, 3));
    let many = quote(&item(7), 4, &stall(&[], 1000)).unwrap();
    assert_eq!(
        (many.tier_total_cp, many.ask_total_cp, many.floor_total_cp),
        (28, 28, 28)
    );
    assert_eq!((many.tier_each_cp, many.floor_each_cp), (7, 7));
}

#[test]
fn a_bulk_tier_applies_from_its_minimum_and_the_largest_reached_wins() {
    let s = stall(&[(5, 100), (20, 250)], 1000);
    let at = |qty| quote(&item(100), qty, &s).unwrap();
    assert_eq!(at(4).tier_total_cp, 400);
    assert_eq!(at(5).tier_total_cp, 450);
    assert_eq!(at(19).tier_total_cp, 1710);
    assert_eq!(at(20).tier_total_cp, 1500);
    assert_eq!(at(20).ask_total_cp, 2000);
    assert_eq!(at(20).discount_permille, 250);
    assert_eq!(at(20).tier_each_cp, 75);
}

#[test]
fn the_discount_is_taken_on_the_total_so_five_apples_at_three_cost_fourteen() {
    // 15 cp asked; ten percent off is 13.5, so 14; 85 percent of 15 is 12.75, so 13.
    let q = quote(&item(3), 5, &stall(&[(5, 100)], 850)).unwrap();
    assert_eq!(
        (q.ask_total_cp, q.tier_total_cp, q.floor_total_cp),
        (15, 14, 13)
    );
    // Display only: 14 over 5 is 2.8, 13 over 5 is 2.6.
    assert_eq!((q.tier_each_cp, q.floor_each_cp), (3, 3));
}

#[test]
fn rounding_goes_up_and_a_lot_is_never_below_one_copper_a_unit() {
    let cheap = quote(&item(1), 100, &stall(&[(2, 999)], 1)).unwrap();
    assert_eq!(cheap.ask_total_cp, 100);
    assert_eq!(cheap.tier_total_cp, 100, "never below 1 cp each");
    assert_eq!(cheap.floor_total_cp, 1);
    assert_eq!((cheap.tier_each_cp, cheap.floor_each_cp), (1, 1));
    let one = quote(&item(1), 1, &stall(&[], 1)).unwrap();
    assert_eq!((one.tier_total_cp, one.floor_total_cp), (1, 1));
}

#[test]
fn a_quantity_below_every_tier_has_no_discount() {
    let q = quote(&item(3), 4, &stall(&[(5, 100)], 850)).unwrap();
    assert_eq!((q.discount_permille, q.ask_total_cp, q.tier_total_cp), (0, 12, 12));
    assert_eq!(q.floor_total_cp, 11, "10.2 rounds up");
}

#[test]
fn the_floor_is_never_above_the_tier_total() {
    for floor in [1, 500, 850, 899, 900, 901, 1000] {
        for qty in 1..=12 {
            for base in [1, 2, 3, 7, 100] {
                let q = quote(&item(base), qty, &stall(&[(5, 100)], floor)).unwrap();
                assert!(q.floor_total_cp >= 1);
                assert!(q.floor_total_cp <= q.tier_total_cp, "{floor} {qty} {base}");
                assert!(q.tier_total_cp <= q.ask_total_cp);
                assert!(q.tier_total_cp >= u64::from(qty));
            }
        }
    }
}

#[test]
fn quotes_saturate_instead_of_overflowing() {
    let q = quote(&item(u64::MAX), u32::MAX, &stall(&[(2, 1)], 1000)).unwrap();
    assert_eq!(q.ask_total_cp, u64::MAX);
    assert!(q.tier_total_cp <= q.ask_total_cp && q.tier_total_cp > u64::MAX / 2);
    assert_eq!(q.floor_total_cp, q.tier_total_cp);
    assert_eq!(q.ask_each_cp, u64::MAX);
    assert!(q.tier_each_cp >= 1);
}

#[test]
fn a_quote_needs_a_unit_and_an_item_the_stall_sells() {
    assert_eq!(
        quote(&item(3), 0, &stall(&[], 1000)),
        Err(QuoteError::ZeroQuantity)
    );
    let mut other = item(3);
    other.id = "other".to_owned();
    assert_eq!(
        quote(&other, 1, &stall(&[], 1000)),
        Err(QuoteError::StallDoesNotSell {
            item: "other".to_owned()
        })
    );
}

#[test]
fn a_quote_is_found_by_place_and_item() {
    let bible = parse(&good()).unwrap();
    let q = bible.quote_at("market", "pear", 5).unwrap();
    assert_eq!((q.ask_total_cp, q.tier_total_cp), (20, 18));
    assert_eq!(
        bible.quote_at("harbor", "pear", 1),
        Err(QuoteError::UnknownPlace {
            id: "harbor".to_owned()
        })
    );
    assert_eq!(
        bible.quote_at("market", "plum", 1).unwrap_err().key(),
        "unknown_item"
    );
    let mut more = good();
    more["items"].as_array_mut().unwrap().push(json!(
        {"id": "plum", "name": "plum", "unit": "each", "base_value_cp": 3}
    ));
    assert_eq!(
        parse(&more)
            .unwrap()
            .quote_at("market", "plum", 1)
            .unwrap_err()
            .key(),
        "not_sold"
    );
}

#[test]
fn a_proposal_is_held_between_the_floor_and_the_tier_total() {
    let q = quote(&item(3), 5, &stall(&[(5, 100)], 850)).unwrap();
    assert_eq!(clamp_quote(0, &q), 13);
    assert_eq!(clamp_quote(12, &q), 13);
    assert_eq!(clamp_quote(13, &q), 13);
    assert_eq!(clamp_quote(14, &q), 14);
    assert_eq!(clamp_quote(15, &q), 14);
    assert_eq!(clamp_quote(99, &q), 14);
    assert_eq!(clamp_quote(u64::MAX, &q), 14);
}

#[test]
fn a_quote_for_a_keeper_finds_their_stall_and_item_or_none() {
    let bible = parse(&good()).unwrap();
    let q = bible.quote_for_keeper("fruit_vendor", "apple", 5).unwrap();
    assert_eq!((q.ask_total_cp, q.tier_total_cp, q.floor_total_cp), (15, 14, 13));
    assert_eq!(bible.quote_for_keeper("fruit_vendor", "plum", 5), None);
    assert_eq!(bible.quote_for_keeper("baker", "apple", 5), None);
    assert_eq!(bible.quote_for_keeper("fruit_vendor", "apple", 0), None);
}

#[test]
fn a_bulk_tier_is_unposted_unless_the_bible_says_so() {
    let bible = parse(&good()).unwrap();
    assert!(!bible.places[0].stalls[0].bulk[0].posted);
    let mut posted = good();
    posted["places"][0]["stalls"][0]["bulk"][0]["posted"] = json!(true);
    assert!(parse(&posted).unwrap().places[0].stalls[0].bulk[0].posted);
}
