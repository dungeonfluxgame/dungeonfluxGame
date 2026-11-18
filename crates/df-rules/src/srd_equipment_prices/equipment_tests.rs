use super::{
    COIN_WEIGHT, EquipmentCategory, SRD_521_EQUIPMENT, Weight, carrying_capacity, equipment,
    price_cp,
};
use crate::local_journey::SpeciesSize;

fn pounds(id: &str) -> Weight {
    equipment(id).expect("listed").unit_weight()
}

#[test]
fn ids_are_unique_and_every_unit_weight_is_exact() {
    for (index, entry) in SRD_521_EQUIPMENT.iter().enumerate() {
        assert!(
            SRD_521_EQUIPMENT[index + 1..]
                .iter()
                .all(|other| other.id != entry.id),
            "{}",
            entry.id
        );
        assert_eq!(
            entry.weight.milli_pounds() % u64::from(entry.per),
            0,
            "{}",
            entry.id
        );
        let chapter = (89..=100).contains(&entry.srd_page);
        assert!(chapter || entry.id == "spellbook", "{}", entry.id);
    }
}

#[test]
fn weights_and_prices_match_the_printed_tables() {
    let rows = [
        ("dagger", 200, 1_000),
        ("shortsword", 1_000, 2_000),
        ("rapier", 2_500, 2_000),
        ("quarterstaff", 20, 4_000),
        ("light-crossbow", 2_500, 5_000),
        ("dart", 5, 250),
        ("arrow", 5, 50),
        ("bolt", 5, 75),
        ("backpack", 200, 5_000),
        ("bedroll", 100, 7_000),
        ("rations", 50, 2_000),
        ("rope", 100, 5_000),
        ("torch", 1, 1_000),
        ("tinderbox", 50, 1_000),
        ("waterskin", 20, 5_000),
        ("crowbar", 200, 5_000),
        ("thieves-tools", 2_500, 1_000),
        ("ink", 1_000, 0),
        ("ink-pen", 2, 0),
        ("paper", 20, 0),
        ("lantern-hooded", 500, 2_000),
        ("oil", 10, 1_000),
        ("healers-kit", 500, 3_000),
        ("explorers-pack", 1_000, 55_000),
        ("entertainers-pack", 4_000, 58_500),
        ("plate-armor", 150_000, 65_000),
        ("mirror", 500, 500),
    ];
    for (id, copper, milli_pounds) in rows {
        assert_eq!(
            price_cp(id).map(|price| price.copper()),
            Some(copper),
            "{id}"
        );
        assert_eq!(pounds(id), Weight::from_milli_pounds(milli_pounds), "{id}");
    }
}

#[test]
fn a_kit_id_finds_the_baseline_entry() {
    assert_eq!(equipment("arrows").map(|item| item.id), Some("arrow"));
    assert_eq!(
        equipment("dungeoneers-pack").map(|item| item.id),
        Some("dungeoneer-pack")
    );
    for (kit, own) in [
        ("book-history", "book"),
        ("book-occult-lore", "book"),
        ("book-prayers", "book"),
        ("parchment-sheets", "parchment"),
    ] {
        assert_eq!(equipment(kit).map(|item| item.id), Some(own), "{kit}");
    }
    assert_eq!(equipment("holy-symbol"), None);
}

#[test]
fn the_spellbook_weighs_three_pounds_and_has_no_price() {
    let spellbook = equipment("spellbook").expect("listed");
    assert_eq!(
        (spellbook.weight, spellbook.srd_page),
        (Weight::pounds(3), 78)
    );
    assert_eq!(price_cp("spellbook"), None);
}

#[test]
fn twenty_sling_bullets_cost_four_copper_and_one_has_no_whole_price() {
    let bullet = equipment("sling-bullet").expect("listed");
    assert_eq!(price_cp("sling-bullet"), None);
    assert_eq!(bullet.price_for(20).map(|price| price.copper()), Some(4));
    assert_eq!(bullet.price_for(5).map(|price| price.copper()), Some(1));
}

#[test]
fn every_pack_weighs_what_its_contents_weigh() {
    let packs = SRD_521_EQUIPMENT
        .iter()
        .filter(|item| item.category == EquipmentCategory::Pack);
    let mut seen = 0;
    for pack in packs {
        seen += 1;
        let contents = pack.contents.iter().fold(0, |total, (id, count)| {
            total + pounds(id).milli_pounds() * u64::from(*count)
        });
        assert_eq!(contents, pack.weight.milli_pounds(), "{}", pack.id);
    }
    assert_eq!(seen, 7);
}

#[test]
fn explorers_and_dungeoneers_pack_contents_are_the_srd_lists() {
    let explorers = equipment("explorers-pack").expect("listed").contents;
    assert_eq!(
        explorers,
        &[
            ("backpack", 1),
            ("bedroll", 1),
            ("oil", 2),
            ("rations", 10),
            ("rope", 1),
            ("tinderbox", 1),
            ("torch", 10),
            ("waterskin", 1),
        ]
    );
    let dungeoneers = equipment("dungeoneer-pack").expect("listed").contents;
    assert!(dungeoneers.contains(&("caltrops", 1)));
    assert!(dungeoneers.contains(&("crowbar", 1)));
}

#[test]
fn containers_carry_their_srd_capacity() {
    let backpack = equipment("backpack").and_then(|item| item.capacity);
    let backpack = backpack.expect("a container");
    assert_eq!(backpack.weight, Some(Weight::pounds(30)));
    assert_eq!(backpack.cubic_foot_thousandths, Some(1_000));
    let pouch = equipment("pouch")
        .and_then(|item| item.capacity)
        .expect("a container");
    assert_eq!(pouch.weight, Some(Weight::pounds(6)));
    assert_eq!(pouch.cubic_foot_thousandths, Some(200));
    let quiver = equipment("quiver")
        .and_then(|item| item.capacity)
        .expect("a container");
    assert_eq!(quiver.only, &[("arrow", 20)]);
    let chest = equipment("chest")
        .and_then(|item| item.capacity)
        .expect("a container");
    assert_eq!(
        (chest.weight, chest.cubic_foot_thousandths),
        (None, Some(12_000))
    );
    for id in [
        "sack",
        "basket",
        "barrel",
        "flask",
        "vial",
        "waterskin",
        "case-map-or-scroll",
    ] {
        assert!(
            equipment(id).and_then(|item| item.capacity).is_some(),
            "{id}"
        );
    }
}

#[test]
fn every_container_limit_names_listed_items() {
    for container in SRD_521_EQUIPMENT.iter().filter_map(|item| item.capacity) {
        for (id, _) in container.only {
            assert!(equipment(id).is_some(), "{id}");
        }
    }
    for pack in SRD_521_EQUIPMENT {
        for (id, _) in pack.contents {
            assert!(equipment(id).is_some(), "{id}");
        }
    }
}

#[test]
fn carrying_capacity_is_strength_times_fifteen_pounds() {
    assert_eq!(
        carrying_capacity(8, SpeciesSize::Medium),
        Weight::pounds(120)
    );
    assert_eq!(
        carrying_capacity(15, SpeciesSize::Medium),
        Weight::pounds(225)
    );
    assert_eq!(
        carrying_capacity(8, SpeciesSize::Small),
        Weight::pounds(120)
    );
    assert_eq!(
        carrying_capacity(15, SpeciesSize::Small),
        Weight::pounds(225)
    );
}

#[test]
fn fifty_coins_weigh_a_pound() {
    assert_eq!(COIN_WEIGHT.checked_times(50), Some(Weight::pounds(1)));
}

#[test]
fn weights_print_in_pounds_with_their_fraction() {
    assert_eq!(Weight::pounds(2).to_string(), "2 lb");
    assert_eq!(Weight::from_milli_pounds(250).to_string(), "0.25 lb");
    assert_eq!(Weight::from_milli_pounds(58_500).to_string(), "58.5 lb");
    assert_eq!(Weight::from_milli_pounds(20).to_string(), "0.02 lb");
}
