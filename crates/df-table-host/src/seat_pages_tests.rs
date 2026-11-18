//! The seat pages' own wording of what the inventory and the memory hold: prices, weights, the
//! load against its room, a shop's stock and the map after a crossing.

use df_playground::pages::wire::{BagItem, Kept, KeptKind};
use df_rules::coins::{Denomination, Price};
use df_rules::srd_equipment_prices::Weight;

use crate::seat_pages::{load_of, places_of, price_words, shop_shown, weigh_items};
use crate::world::inventory::{BagLine, BagView, ContainerLoad, ShareLoad, ShopView, StockLine};

fn pounds(pounds: u64) -> Weight {
    Weight::pounds(pounds)
}

fn bag() -> BagView {
    BagView {
        owner: "hero-1".to_owned(),
        lines: vec![
            BagLine {
                item: "torch".to_owned(),
                name: "Torch".to_owned(),
                count: 3,
                unit_weight: Some(pounds(1)),
                weight: Some(pounds(3)),
                container: Some("backpack".to_owned()),
            },
            BagLine {
                item: "torch".to_owned(),
                name: "Torch".to_owned(),
                count: 2,
                unit_weight: Some(pounds(1)),
                weight: Some(pounds(2)),
                container: None,
            },
            BagLine {
                item: "charm".to_owned(),
                name: "Lucky charm".to_owned(),
                count: 1,
                unit_weight: None,
                weight: None,
                container: None,
            },
        ],
        containers: vec![
            ContainerLoad {
                container: "backpack".to_owned(),
                name: "Backpack".to_owned(),
                count: 1,
                load: pounds(30),
                weight_capacity: Some(pounds(30)),
                shares: None,
            },
            ContainerLoad {
                container: "quiver".to_owned(),
                name: "Quiver".to_owned(),
                count: 1,
                load: Weight::from_milli_pounds(600),
                weight_capacity: None,
                shares: Some(ShareLoad { used: 12, most: 20 }),
            },
        ],
        coins: Vec::new(),
        coins_cp: 0,
        weight: Weight::from_milli_pounds(162_500),
        unweighed: vec!["charm".to_owned()],
        capacity: Some(pounds(150)),
    }
}

fn carried(id: &str, count: u64) -> BagItem {
    BagItem {
        id: id.to_owned(),
        name: id.to_owned(),
        look: String::new(),
        count,
        kept: true,
        picture: None,
        worth: None,
        weight: None,
        stowed: None,
    }
}

#[test]
fn a_price_reads_in_the_largest_coin_that_makes_it_exactly() {
    assert_eq!(price_words(100), "1 gp");
    assert_eq!(price_words(250), "25 sp");
    assert_eq!(price_words(50), "5 sp");
    assert_eq!(price_words(4), "4 cp");
    assert_eq!(price_words(0), "0 cp");
}

#[test]
fn each_carried_thing_says_what_it_weighs_and_where_it_is_packed() {
    let mut items = vec![carried("torch", 5), carried("charm", 1)];
    weigh_items(&mut items, &bag());
    assert_eq!(items[0].weight.as_deref(), Some("1 lb each, 5 lb"));
    assert_eq!(items[0].stowed.as_deref(), Some("In the Backpack"));
    assert_eq!(items[1].weight, None);
    assert_eq!(items[1].stowed, None);
}

#[test]
fn the_load_is_the_total_against_the_capacity_and_each_container_against_its_room() {
    let load = load_of(&bag());
    assert_eq!(load.weight, "162.5 lb");
    assert_eq!(load.capacity.as_deref(), Some("150 lb"));
    assert!(load.over);
    let backpack = &load.containers[0];
    assert_eq!(
        (backpack.load.as_str(), backpack.capacity.as_deref()),
        ("30 lb", Some("30 lb"))
    );
    assert!(backpack.full);
    let quiver = &load.containers[1];
    assert_eq!(quiver.shares.as_deref(), Some("12 of 20"));
    assert!(!quiver.full);
    assert_eq!(load.unweighed, vec!["Lucky charm".to_owned()]);
}

#[test]
fn a_shop_prices_a_bundle_by_the_bundle_and_keeps_its_keepers_name() {
    let view = ShopView {
        keeper: "garrick".to_owned(),
        coin: Some(Denomination::Gp),
        lines: vec![
            StockLine {
                item: "arrow".to_owned(),
                name: "Arrows".to_owned(),
                in_stock: 40,
                price: Price::from_copper(100),
                per: 20,
                unit_weight: Some(Weight::from_milli_pounds(50)),
                buyable: true,
            },
            StockLine {
                item: "charm".to_owned(),
                name: "Lucky charm".to_owned(),
                in_stock: 1,
                price: Price::from_copper(20),
                per: 1,
                unit_weight: None,
                buyable: false,
            },
        ],
    };
    let shop = shop_shown(view, "Garrick".to_owned());
    assert_eq!(
        (shop.id.as_str(), shop.keeper.as_str()),
        ("garrick", "Garrick")
    );
    assert_eq!(shop.lines[0].price, "1 gp for 20");
    assert_eq!(shop.lines[0].weight.as_deref(), Some("0.05 lb"));
    assert_eq!(shop.lines[1].price, "2 sp");
    assert!(!shop.lines[1].buyable);
}

#[test]
fn after_a_crossing_the_map_shows_only_the_places_of_the_new_region() {
    let place = |id: &str, seq: u64| Kept {
        id: id.to_owned(),
        kind: KeptKind::Place,
        seq,
        title: id.to_owned(),
        text: String::new(),
        picture: Some(format!("{id}.png")),
    };
    // Newest first, as the notebook keeps them.
    let kept = vec![place("far-shore", 3), place("tomb", 2), place("town", 1)];
    let everywhere = places_of(&kept, None, None, None);
    assert_eq!(everywhere.len(), 3);
    let region = vec!["far-shore".to_owned()];
    let after = places_of(&kept, Some(&region), None, None);
    assert_eq!(
        after
            .iter()
            .map(|place| place.id.as_str())
            .collect::<Vec<_>>(),
        vec!["far-shore"]
    );
    assert!(after[0].here);
    // A region with nothing kept in it yet shows an empty map.
    assert!(places_of(&kept, Some(&[]), None, None).is_empty());
}
