//! Tests of weight, baggage space and buying: exact purchases, change, refusals that move nothing,
//! container and carrying capacity, pack contents and the views.

use df_rules::coins::{Denomination, Price};
use df_rules::local_journey::SpeciesSize;
use df_rules::srd_equipment_prices::{Weight, carrying_capacity, equipment};

use super::bag::{gaining_coins, paying, purchase, receiving, view_of_bag, view_of_shop};
use super::{
    Inventory, InventoryChange, InventoryError, InventoryItem, ItemUse, OwnerKind, Stowed,
    apply_changes,
};

/// Strength 10, Medium: 150 lb.
const ROOMY: Weight = Weight::pounds(150);

fn ware(id: &str, count: u64) -> InventoryItem {
    let entry = equipment(id).expect("a catalog item");
    InventoryItem {
        id: id.to_owned(),
        name: entry.name.to_owned(),
        look: format!("a {} on the shelf", entry.name),
        usage: ItemUse::Kept,
        count,
        price_cp: entry.price_for(1).map(|price| price.copper()),
        coin: None,
    }
}

fn coin(id: &str, denomination: Denomination, count: u64) -> InventoryItem {
    InventoryItem {
        id: id.to_owned(),
        name: format!("{} pieces", denomination.key()),
        look: "worn coins".to_owned(),
        usage: ItemUse::Consumed,
        count,
        price_cp: None,
        coin: Some(denomination),
    }
}

fn hero(items: Vec<InventoryItem>) -> Inventory {
    Inventory::new("hero-1", OwnerKind::Character, items).expect("a hero")
}

fn outfitter() -> Inventory {
    Inventory::new(
        "outfitter",
        OwnerKind::Shop,
        vec![
            ware("dagger", 6),
            ware("rations", 40),
            ware("arrow", 100),
            ware("backpack", 3),
            ware("pouch", 3),
            ware("explorers-pack", 2),
            ware("plate-armor", 1),
            ware("sling-bullet", 40),
            coin("gold", Denomination::Gp, 50),
            coin("silver", Denomination::Sp, 30),
            coin("copper", Denomination::Cp, 30),
        ],
    )
    .expect("a shop")
}

fn count(inventory: &Inventory, id: &str) -> u64 {
    inventory.item(id).map_or(0, |item| item.count)
}

#[test]
fn an_exact_purchase_moves_the_price_and_the_goods() {
    let buyer = hero(vec![coin("gold", Denomination::Gp, 5)]);
    let (bought_into, shop, _, bought) =
        purchase(&buyer, &outfitter(), ("dagger", 2), ROOMY).expect("bought");
    assert_eq!(count(&bought_into, "gold"), 1);
    assert_eq!(count(&bought_into, "dagger"), 2);
    assert_eq!(count(&shop, "gold"), 54);
    assert_eq!(count(&shop, "dagger"), 4);
    assert_eq!(bought.price.copper(), 400);
    assert_eq!((bought.coins_before_cp, bought.coins_after_cp), (500, 100));
    // Two daggers (1 lb each) and one gold piece (1/50 lb).
    assert_eq!(bought.weight, Weight::from_milli_pounds(2_020));
}

#[test]
fn gold_paying_a_silver_price_gets_its_change_from_the_shop() {
    let buyer = hero(vec![coin("gold", Denomination::Gp, 1)]);
    let (bought_into, shop, _, bought) =
        purchase(&buyer, &outfitter(), ("rations", 1), ROOMY).expect("bought");
    assert_eq!(count(&bought_into, "gold"), 0);
    assert_eq!(count(&bought_into, "silver"), 5, "5 SP change for 1 GP");
    let silver = bought_into.item("silver").expect("the change");
    assert_eq!(silver.coin, Some(Denomination::Sp), "money stays money");
    assert_eq!(count(&shop, "gold"), 51);
    assert_eq!(count(&shop, "silver"), 25);
    assert_eq!((bought.coins_before_cp, bought.coins_after_cp), (100, 50));
}

#[test]
fn twenty_arrows_cost_one_gold_piece_and_go_in_the_quiver() {
    let buyer = hero(vec![coin("gold", Denomination::Gp, 1), ware("quiver", 1)]);
    let (bought_into, _, _, bought) =
        purchase(&buyer, &outfitter(), ("arrow", 20), ROOMY).expect("bought");
    assert_eq!(bought.price.copper(), 100);
    assert_eq!(
        bought_into.stowed,
        vec![Stowed {
            item: "arrow".to_owned(),
            container: "quiver".to_owned(),
            count: 20,
        }]
    );
}

#[test]
fn insufficient_coin_is_refused_and_moves_nothing() {
    let buyer = hero(vec![coin("silver", Denomination::Sp, 3)]);
    let refused = purchase(&buyer, &outfitter(), ("dagger", 1), ROOMY);
    assert_eq!(
        refused.map(|_| ()),
        Err(InventoryError::InsufficientCoin {
            owner: "hero-1".to_owned(),
            cost_cp: 200,
            purse_cp: 30,
        })
    );
    assert_eq!(count(&buyer, "silver"), 3);
}

#[test]
fn what_a_full_container_cannot_take_is_carried_loose() {
    let mut buyer = hero(vec![
        coin("gold", Denomination::Gp, 5),
        ware("quiver", 1),
        ware("arrow", 20),
    ]);
    buyer.stowed.push(Stowed {
        item: "arrow".to_owned(),
        container: "quiver".to_owned(),
        count: 20,
    });
    let (bought_into, ..) = purchase(&buyer, &outfitter(), ("arrow", 1), ROOMY).expect("bought");
    assert_eq!(count(&bought_into, "arrow"), 21);
    assert_eq!(bought_into.stowed[0].count, 20, "the quiver stays full");
    let bag = view_of_bag(&bought_into, Some(ROOMY)).expect("a bag");
    let loose = bag
        .lines
        .iter()
        .find(|line| line.item == "arrow" && line.container.is_none());
    assert_eq!(loose.map(|line| line.count), Some(1));
}

#[test]
fn a_full_backpack_does_not_block_a_longsword() {
    let mut buyer = hero(vec![
        coin("gold", Denomination::Gp, 15),
        ware("backpack", 1),
        ware("rations", 15),
    ]);
    buyer.stowed.push(Stowed {
        item: "rations".to_owned(),
        container: "backpack".to_owned(),
        count: 15,
    });
    let smith = Inventory::new(
        "weaponsmith",
        OwnerKind::Shop,
        vec![ware("longsword", 2), coin("gold", Denomination::Gp, 10)],
    )
    .expect("a shop");
    let (bought_into, ..) = purchase(&buyer, &smith, ("longsword", 1), ROOMY).expect("bought");
    assert_eq!(count(&bought_into, "longsword"), 1);
    assert!(
        bought_into
            .stowed
            .iter()
            .all(|stowed| stowed.item != "longsword"),
        "carried loose, not in the full backpack"
    );
    // Strength 3: 45 lb. The backpack and its 30 lb are 35 lb; a 3 lb longsword still fits, but
    // not when the load would pass the capacity.
    let weak = carrying_capacity(3, SpeciesSize::Medium);
    assert_eq!(
        purchase(&buyer, &smith, ("longsword", 1), weak).map(|_| ()),
        Ok(())
    );
    let heavier = carrying_capacity(2, SpeciesSize::Medium);
    assert!(matches!(
        purchase(&buyer, &smith, ("longsword", 1), heavier),
        Err(InventoryError::OverCapacity { .. })
    ));
}

#[test]
fn a_weight_past_u64_is_a_typed_overflow_not_zero() {
    let hoard = hero(vec![coin("copper", Denomination::Cp, u64::MAX / 2)]);
    assert!(matches!(
        view_of_bag(&hoard, Some(ROOMY)),
        Err(InventoryError::Overflow { .. })
    ));
}

#[test]
fn over_capacity_is_refused() {
    let buyer = hero(vec![coin("platinum", Denomination::Pp, 200)]);
    // Strength 8: 120 lb. Plate armour is 65 lb, the purse 4 lb.
    let weak = carrying_capacity(8, SpeciesSize::Medium);
    let fits = purchase(&buyer, &outfitter(), ("plate-armor", 1), weak).map(|_| ());
    assert_eq!(fits, Ok(()));
    let loaded = hero(vec![
        coin("platinum", Denomination::Pp, 200),
        ware("chain-mail", 1),
    ]);
    let refused = purchase(&loaded, &outfitter(), ("plate-armor", 1), weak);
    assert!(matches!(
        refused,
        Err(InventoryError::OverCapacity {
            capacity_milli_pounds: 120_000,
            ..
        })
    ));
}

#[test]
fn a_deal_that_would_load_a_hero_past_their_capacity_is_refused_and_moves_nothing() {
    use super::ItemCount;
    use super::operations::{DealSide, settled};
    let loaded = hero(vec![
        coin("platinum", Denomination::Pp, 200),
        ware("chain-mail", 1),
    ]);
    let shop = outfitter();
    // Strength 8: 120 lb. The hero carries 59 lb; plate armour is 65 lb more.
    let weak = carrying_capacity(8, SpeciesSize::Medium);
    let price = [ItemCount {
        item: "platinum".to_owned(),
        count: 150,
    }];
    let plate = [ItemCount {
        item: "plate-armor".to_owned(),
        count: 1,
    }];
    let side = |owner, gives, capacity| DealSide {
        owner,
        gives,
        capacity,
    };
    let refused = settled(
        (&loaded, side("hero-1", &price, Some(weak))),
        (&shop, side("outfitter", &plate, None)),
        "deal",
    );
    let Err(error) = refused else {
        panic!("a deal past the capacity settles");
    };
    assert!(
        matches!(
            error,
            InventoryError::OverCapacity {
                capacity_milli_pounds: 120_000,
                ..
            }
        ),
        "{error:?}"
    );
    // The same key the persona facts show (`settle_refusal`) and the phone words come from.
    assert_eq!(error.key(), "over_capacity");
    // A resident has no capacity to pass; a hero who can carry it takes it.
    for capacity in [None, Some(ROOMY)] {
        let (after, _, _, _) = settled(
            (&loaded, side("hero-1", &price, capacity)),
            (&shop, side("outfitter", &plate, None)),
            "deal",
        )
        .expect("settles");
        assert_eq!(count(&after, "plate-armor"), 1);
    }
}

#[test]
fn a_line_that_loads_a_hero_past_their_capacity_changes_nothing() {
    use super::operations::{change, get, set};
    use super::request::SetRequest;
    let directory = tempfile::tempdir().expect("a directory");
    let request = SetRequest {
        owner: "hero-1".to_owned(),
        kind: OwnerKind::Character,
        items: vec![ware("chain-mail", 1)],
    };
    set(directory.path(), request).expect("set");
    let plate = InventoryChange::Add {
        item: "plate-armor".to_owned(),
        count: 1,
        name: Some("Plate Armor".to_owned()),
        look: Some("a suit of plate".to_owned()),
        usage: Some(ItemUse::Kept),
        price_cp: None,
        why: "found".to_owned(),
    };
    // Strength 7: 105 lb. Chain mail is 55 lb, plate armour 65 lb more.
    let weak = carrying_capacity(7, SpeciesSize::Medium);
    let refused = change(
        directory.path(),
        "hero-1",
        std::slice::from_ref(&plate),
        Some(weak),
        None,
    );
    assert!(
        matches!(refused, Err(InventoryError::OverCapacity { .. })),
        "{refused:?}"
    );
    let kept = get(directory.path(), "hero-1").expect("the hero").inventory;
    assert_eq!(count(&kept, "plate-armor"), 0, "nothing of it is taken");
    // With room, or no capacity to check (a resident), it is taken.
    change(directory.path(), "hero-1", &[plate], Some(ROOMY), None).expect("taken");
}

#[test]
fn a_pack_arrives_as_its_contents_in_its_backpack() {
    let buyer = hero(vec![coin("gold", Denomination::Gp, 10)]);
    let (bought_into, _, _, bought) =
        purchase(&buyer, &outfitter(), ("explorers-pack", 1), ROOMY).expect("bought");
    assert_eq!(count(&bought_into, "explorers-pack"), 0);
    for (id, units) in [
        ("backpack", 1),
        ("bedroll", 1),
        ("oil", 2),
        ("rations", 10),
        ("rope", 1),
        ("tinderbox", 1),
        ("torch", 10),
        ("waterskin", 1),
    ] {
        assert_eq!(count(&bought_into, id), units, "{id}");
    }
    assert_eq!(
        bought_into.item("rations").map(|item| item.usage),
        Some(ItemUse::Consumed)
    );
    let bag = view_of_bag(&bought_into, Some(ROOMY)).expect("a bag");
    let backpack = bag
        .containers
        .iter()
        .find(|container| container.container == "backpack")
        .expect("the backpack");
    assert!(backpack.load <= Weight::pounds(30));
    assert_eq!(backpack.weight_capacity, Some(Weight::pounds(30)));
    // 55 lb of contents, no coins left.
    assert_eq!(bought.weight, Weight::pounds(55));
}

#[test]
fn carrying_capacity_for_strength_eight_and_fifteen() {
    assert_eq!(
        carrying_capacity(8, SpeciesSize::Small),
        Weight::pounds(120)
    );
    assert_eq!(
        carrying_capacity(15, SpeciesSize::Medium),
        Weight::pounds(225)
    );
}

#[test]
fn a_shop_that_cannot_make_change_refuses() {
    let till = Inventory::new(
        "outfitter",
        OwnerKind::Shop,
        vec![ware("rations", 5), coin("gold", Denomination::Gp, 5)],
    )
    .expect("a shop");
    let buyer = hero(vec![coin("gold", Denomination::Gp, 1)]);
    let refused = purchase(&buyer, &till, ("rations", 1), ROOMY).map(|_| ());
    assert_eq!(
        refused,
        Err(InventoryError::NoChange {
            shop: "outfitter".to_owned(),
            change_cp: 50,
        })
    );
}

#[test]
fn stock_and_bundles_are_checked() {
    let buyer = hero(vec![coin("gold", Denomination::Gp, 50)]);
    let shop = outfitter();
    assert!(matches!(
        purchase(&buyer, &shop, ("dagger", 7), ROOMY),
        Err(InventoryError::Insufficient {
            have: 6,
            asked: 7,
            ..
        })
    ));
    assert!(matches!(
        purchase(&buyer, &shop, ("rope", 1), ROOMY),
        Err(InventoryError::NotForSale { .. })
    ));
    assert!(matches!(
        purchase(&buyer, &shop, ("sling-bullet", 3), ROOMY),
        Err(InventoryError::UnevenBundle { per: 20, .. })
    ));
    let not_a_shop = hero(vec![ware("dagger", 1)]);
    assert!(matches!(
        purchase(&buyer, &not_a_shop, ("dagger", 1), ROOMY),
        Err(InventoryError::NotAShop { .. })
    ));
}

#[test]
fn using_up_a_stowed_item_takes_the_loose_ones_first() {
    let mut bag = hero(vec![ware("backpack", 1), ware("rations", 5)]);
    bag.stowed.push(Stowed {
        item: "rations".to_owned(),
        container: "backpack".to_owned(),
        count: 3,
    });
    let eat = |count| InventoryChange::Remove {
        item: "rations".to_owned(),
        count,
        why: "eaten".to_owned(),
    };
    let (after, _) = apply_changes(&bag, &[eat(2)]).expect("eaten");
    assert_eq!(after.stowed[0].count, 3);
    let (after, _) = apply_changes(&after, &[eat(2)]).expect("eaten");
    assert_eq!(after.stowed[0].count, 1);
    let drop_pack = InventoryChange::Remove {
        item: "backpack".to_owned(),
        count: 1,
        why: "dropped".to_owned(),
    };
    let (after, _) = apply_changes(&after, &[drop_pack]).expect("dropped");
    assert!(after.stowed.is_empty(), "contents come out with the pack");
}

#[test]
fn the_shop_view_lists_catalog_prices_and_weights() {
    let view = view_of_shop(&outfitter()).expect("a shop");
    assert_eq!(view.keeper, "outfitter");
    assert_eq!(view.coin, Some(Denomination::Gp));
    let arrows = view
        .lines
        .iter()
        .find(|line| line.item == "arrow")
        .expect("arrows");
    assert_eq!((arrows.price.copper(), arrows.per), (100, 20));
    assert_eq!(arrows.unit_weight, Some(Weight::from_milli_pounds(50)));
    assert!(arrows.buyable);
    assert!(view.lines.iter().all(|line| !line.item.is_empty()));
    assert!(view_of_shop(&hero(Vec::new())).is_err());
}

#[test]
fn a_kit_received_arrives_arranged_like_a_purchase() {
    let kit = hero(Vec::new());
    let why = "from the hero's starting equipment";
    let (kit, _, _) = receiving(&kit, ("quiver", 1), why, ROOMY).expect("received");
    let (kit, _, _) = receiving(&kit, ("arrows", 20), why, ROOMY).expect("received");
    let (kit, applied, weight) =
        receiving(&kit, ("burglars-pack", 1), why, ROOMY).expect("received");
    assert_eq!(
        kit.item("arrows")
            .map(|item| (item.name.as_str(), item.usage)),
        Some(("Arrow", ItemUse::Consumed))
    );
    assert!(kit.stowed.contains(&Stowed {
        item: "arrows".to_owned(),
        container: "quiver".to_owned(),
        count: 20,
    }));
    assert_eq!(count(&kit, "burglars-pack"), 0);
    assert_eq!(count(&kit, "candle"), 10);
    assert!(applied.iter().any(|taken| taken.item == "backpack"));
    // Quiver 1 lb, 20 arrows 1 lb, the pack's contents 42 lb.
    assert_eq!(weight, Weight::pounds(44));
    let (kit, _, weight) = receiving(&kit, ("spellbook", 1), why, ROOMY).expect("received");
    assert_eq!(
        weight,
        Weight::pounds(47),
        "the Spellbook weighs 3 lb (SRD p.78)"
    );
    assert_eq!(
        receiving(&kit, ("holy-symbol", 1), why, ROOMY).map(|_| ()),
        Err(InventoryError::NotInCatalog {
            item: "holy-symbol".to_owned()
        })
    );
    let weak = carrying_capacity(3, SpeciesSize::Small);
    assert!(matches!(
        receiving(&kit, ("plate-armor", 1), why, weak),
        Err(InventoryError::OverCapacity { .. })
    ));
}

// TODO(DEV-12): a repeated Buy seat action with the same `said` is answered once and charges once
// (the seat route's done_before covers it; the test belongs to the seat worker's Buy action).

#[test]
fn coins_gained_go_to_the_held_coin_or_a_standard_one() {
    let purse = hero(vec![coin("coins", Denomination::Sp, 3)]);
    let why = "a quest advance";
    let (purse, _, _) = gaining_coins(&purse, (Denomination::Sp, 7), why, ROOMY).expect("gained");
    assert_eq!(count(&purse, "coins"), 10);
    let (purse, _, weight) =
        gaining_coins(&purse, (Denomination::Gp, 8), why, ROOMY).expect("gained");
    let gold = purse.item("gold-piece").expect("a standard gold coin item");
    assert_eq!((gold.count, gold.coin), (8, Some(Denomination::Gp)));
    assert_eq!(
        weight,
        Weight::from_milli_pounds(18 * 20),
        "50 coins weigh a pound"
    );
    let mut clash = hero(vec![ware("dagger", 1)]);
    clash.items[0].id = "gold-piece".to_owned();
    assert!(matches!(
        gaining_coins(&clash, (Denomination::Gp, 1), why, ROOMY),
        Err(InventoryError::CoinMismatch { .. })
    ));
    assert!(matches!(
        gaining_coins(&purse, (Denomination::Pp, 3_000), why, Weight::pounds(10)),
        Err(InventoryError::OverCapacity { .. })
    ));
}

fn ferryman(items: Vec<InventoryItem>) -> Inventory {
    Inventory::new("ferryman", OwnerKind::Shop, items).expect("a ferryman")
}

#[test]
fn an_exact_fare_moves_the_coins_and_nothing_else() {
    let payer = hero(vec![coin("silver", Denomination::Sp, 8)]);
    let payee = ferryman(vec![coin("silver", Denomination::Sp, 2)]);
    let (paid_from, paid_to, _, paid) = paying(
        (&payer, &payee),
        Price::from_copper(50),
        "ferry fare",
        Some(ROOMY),
    )
    .expect("paid");
    assert_eq!(
        (count(&paid_from, "silver"), count(&paid_to, "silver")),
        (3, 7)
    );
    assert_eq!((paid.coins_before_cp, paid.coins_after_cp), (80, 30));
}

#[test]
fn a_fare_paid_in_gold_gets_its_change() {
    let payer = hero(vec![coin("gold-piece", Denomination::Gp, 2)]);
    let payee = ferryman(vec![coin("silver-piece", Denomination::Sp, 20)]);
    let (paid_from, paid_to, _, paid) = paying(
        (&payer, &payee),
        Price::from_copper(30),
        "toll",
        Some(ROOMY),
    )
    .expect("paid");
    assert_eq!(count(&paid_from, "gold-piece"), 1);
    assert_eq!(count(&paid_from, "silver-piece"), 7);
    assert_eq!(
        paid_from.item("silver-piece").and_then(|item| item.coin),
        Some(Denomination::Sp)
    );
    assert_eq!(
        (
            count(&paid_to, "gold-piece"),
            count(&paid_to, "silver-piece")
        ),
        (1, 13)
    );
    assert_eq!((paid.coins_before_cp, paid.coins_after_cp), (200, 170));
}

#[test]
fn a_short_purse_is_refused_and_moves_nothing() {
    let payer = hero(vec![coin("silver", Denomination::Sp, 2)]);
    let payee = ferryman(vec![coin("silver", Denomination::Sp, 5)]);
    assert_eq!(
        paying((&payer, &payee), Price::from_copper(50), "toll", None).map(|_| ()),
        Err(InventoryError::InsufficientCoin {
            owner: "hero-1".to_owned(),
            cost_cp: 50,
            purse_cp: 20,
        })
    );
    assert_eq!(count(&payer, "silver"), 2);
}

#[test]
fn a_payee_without_change_is_refused() {
    let payer = hero(vec![coin("gold-piece", Denomination::Gp, 1)]);
    let empty = ferryman(Vec::new());
    assert_eq!(
        paying((&payer, &empty), Price::from_copper(30), "toll", None).map(|_| ()),
        Err(InventoryError::NoChange {
            shop: "ferryman".to_owned(),
            change_cp: 70,
        })
    );
}

#[test]
fn a_two_silver_toll_paid_with_only_gold_takes_its_change_from_the_keepers_till() {
    // Run 9: Maud held only gold, and the toll keeper's purse could not make 80 cp of change.
    let payer = hero(vec![coin("gold-piece", Denomination::Gp, 3)]);
    let (till, added_cp) = super::bag::with_till(vec![coin("silver-piece", Denomination::Sp, 3)]);
    // 20 cp, 17 more sp and 10 gp: 20 + 170 + 1000.
    assert_eq!(added_cp, 1190);
    let keeper = Inventory::new("toll_keeper", OwnerKind::Shop, till).expect("a keeper");
    assert_eq!(count(&keeper, "silver-piece"), 20);
    assert_eq!(count(&keeper, "copper-piece"), 20);
    assert_eq!(count(&keeper, "gold-piece"), 10);
    let (paid_from, paid_to, _, paid) = paying(
        (&payer, &keeper),
        Price::from_copper(20),
        "the bridge toll",
        Some(ROOMY),
    )
    .expect("paid, with change");
    assert_eq!(count(&paid_from, "gold-piece"), 2);
    assert_eq!(count(&paid_from, "silver-piece"), 8);
    assert_eq!(count(&paid_to, "gold-piece"), 11);
    assert_eq!(count(&paid_to, "silver-piece"), 12);
    assert_eq!((paid.coins_before_cp, paid.coins_after_cp), (300, 280));
    // A full till is left as it is; one that gave change is topped up again (8 sp).
    let (_, again) = super::bag::with_till(keeper.items.clone());
    assert_eq!(again, 0);
    let (_, refilled) = super::bag::with_till(paid_to.items.clone());
    assert_eq!(refilled, 80);
}

#[test]
fn a_refused_payment_reads_as_the_phones_words_for_it() {
    use df_playground::pages::wire::PagesErrorKind;
    let refusal = InventoryError::NoChange {
        shop: "toll_keeper".to_owned(),
        change_cp: 80,
    };
    assert_eq!(refusal.pages_kind(), PagesErrorKind::NoChange);
    assert_eq!(
        refusal.pages_kind().player_words(),
        "They can't make change for that."
    );
    let unknown = InventoryError::UnknownOwner {
        owner: "ferryman".to_owned(),
    };
    assert_eq!(unknown.pages_kind(), PagesErrorKind::Inventory);
}

#[test]
fn a_reason_is_logged_as_an_id() {
    assert_eq!(
        super::bag::why_id("from the hero's starting equipment"),
        "from-the-heros-starting-equipment"
    );
    assert_eq!(
        super::bag::why_id("A reward advance from Mara on The Sunken Crypt!"),
        "a-reward-advance-from-mara-on-the-sunken-crypt"
    );
    assert_eq!(super::bag::why_id("  "), "-");
}
