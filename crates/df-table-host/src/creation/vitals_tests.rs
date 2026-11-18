//! A Rogue's and a Wizard's starting equipment and gold put into their bags over a running host:
//! packs as their contents, the quiver holding the arrows, the gold as gold pieces.
use serde_json::json;

use df_table::creation::{CreationTable, HeroStart};
use df_table::identity::SeatId;
use df_table::machine::{CarryItem, Entry};

use super::rules::HeroRules;
use super::store;
use super::vitals::{give_kit, give_kit_again};
use crate::dev_client::post_json;
use crate::test_support::{Running, STARTER_FILES, start};
use crate::world::inventory::request::{Changed, InventoryReply};
use crate::world::inventory::routes::path;
use crate::world::inventory_of;

const ROGUE: u64 = 31;
const WIZARD: u64 = 32;

/// The picks that finish a Tiefling Rogue Criminal begun from the look studio.
const ROGUE_PICKS: [(&str, &str); 28] = [
    ("fiendish-legacy", "infernal"),
    ("species-size", "medium"),
    ("species-spellcasting", "charisma"),
    ("background", "criminal"),
    ("ability-method", "standard-array"),
    ("strength", "12"),
    ("dexterity", "15"),
    ("constitution", "13"),
    ("intelligence", "14"),
    ("wisdom", "10"),
    ("charisma", "8"),
    ("ability-increase", "two-and-one"),
    ("increase-two", "dexterity"),
    ("increase-one", "constitution"),
    ("class-skill-1", "acrobatics"),
    ("class-skill-2", "deception"),
    ("class-skill-3", "perception"),
    ("class-skill-4", "investigation"),
    ("expertise-1", "stealth"),
    ("expertise-2", "perception"),
    ("mastery-1", "dagger"),
    ("mastery-2", "shortbow"),
    ("rogue-language", "infernal"),
    ("language-1", "elvish"),
    ("language-2", "dwarvish"),
    ("alignment", "chaotic-good"),
    ("class-equipment", "a"),
    ("background-equipment", "a"),
];

/// The picks that finish a Dwarf Wizard Sage, taking the Sage's 50 GP for its kit.
const WIZARD_PICKS: [(&str, &str); 29] = [
    ("background", "sage"),
    ("ability-method", "point-buy"),
    ("strength", "8"),
    ("dexterity", "14"),
    ("constitution", "15"),
    ("intelligence", "15"),
    ("wisdom", "10"),
    ("charisma", "8"),
    ("ability-increase", "one-each"),
    ("class-skill-1", "investigation"),
    ("class-skill-2", "medicine"),
    ("language-1", "dwarvish"),
    ("language-2", "elvish"),
    ("alignment", "neutral-good"),
    ("class-equipment", "a"),
    ("background-equipment", "b"),
    ("cantrip-1", "light"),
    ("cantrip-2", "mage-hand"),
    ("cantrip-3", "prestidigitation"),
    ("spellbook-1", "detect-magic"),
    ("spellbook-2", "comprehend-languages"),
    ("spellbook-3", "identify"),
    ("spellbook-4", "magic-missile"),
    ("spellbook-5", "shield"),
    ("spellbook-6", "sleep"),
    ("prepared-1", "magic-missile"),
    ("prepared-2", "shield"),
    ("prepared-3", "sleep"),
    ("prepared-4", "detect-magic"),
];

fn hero(id: u64, species: &str, class: &str) -> HeroStart {
    HeroStart {
        hero: id,
        name: "Sable Wren".to_owned(),
        species: species.to_owned(),
        class: class.to_owned(),
        portrait: None,
        public_story: Vec::new(),
    }
}

fn built_pair() -> CreationTable {
    let (one, two) = (SeatId::new(1), SeatId::new(2));
    let mut table = CreationTable::enter(&Entry::carrying(vec![
        CarryItem::Seat(one),
        CarryItem::Seat(two),
    ]))
    .expect("enters");
    for (seat, start, picks) in [
        (one, hero(ROGUE, "tiefling", "rogue"), &ROGUE_PICKS[..]),
        (two, hero(WIZARD, "dwarf", "wizard"), &WIZARD_PICKS[..]),
    ] {
        table.begin(seat, start, &HeroRules).expect("begins");
        for (slot, option) in picks {
            table
                .choose(seat, slot, option, &HeroRules)
                .expect("chooses");
        }
    }
    table
}

async fn host_with_two_heroes() -> Running {
    let running = start(&STARTER_FILES).await;
    std::fs::create_dir_all(store::directory(&running.state)).expect("makes the players folder");
    store::change(&running.state, |_| Ok((built_pair(), ())))
        .await
        .expect("keeps the record");
    for owner in ["hero-31", "hero-32"] {
        let made: InventoryReply<Changed> = post_json(
            running.port,
            path::INVENTORY_SET,
            &json!({"owner": owner, "kind": "character", "items": []}),
        )
        .await
        .expect("answers");
        assert!(matches!(made, crate::dev_http::DevReply::Done { .. }));
    }
    running
}

fn held(running: &Running, owner: &str, item: &str) -> u64 {
    inventory_of(&running.state, owner)
        .iter()
        .find(|held| held.id == item)
        .map_or(0, |held| held.count)
}

#[tokio::test]
async fn a_new_rogue_and_wizard_get_their_kit_and_gold_in_the_bag() {
    let running = host_with_two_heroes().await;
    for hero in [ROGUE, WIZARD] {
        give_kit(&running.state, hero).await.expect("gives");
    }
    // Rogue A: 8 GP; Criminal A: 16 GP (SRD pp. 61, 83).
    assert_eq!(held(&running, "hero-31", "gold-piece"), 24);
    // Wizard A: 5 GP; the Sage's B: 50 GP (SRD pp. 77, 83).
    assert_eq!(held(&running, "hero-32", "gold-piece"), 55);
    // The Burglar's Pack arrives as its contents, its own backpack among them.
    assert_eq!(held(&running, "hero-31", "burglars-pack"), 0);
    assert_eq!(held(&running, "hero-31", "backpack"), 1);
    assert_eq!(held(&running, "hero-31", "arrows"), 20);
    // The Criminal's and the Rogue's Thieves' Tools are two tools.
    assert_eq!(held(&running, "hero-31", "thieves-tools"), 2);
    assert_eq!(held(&running, "hero-32", "spellbook"), 1);
    assert_eq!(held(&running, "hero-32", "scholars-pack"), 0);
    // Given once: a second hand-out adds nothing.
    give_kit(&running.state, ROGUE).await.expect("gives");
    assert_eq!(held(&running, "hero-31", "gold-piece"), 24);
    let lines: Vec<String> = running
        .state
        .log()
        .newest(256)
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("CREATION kit_given hero=31 ")
                && line.ends_with("gold_gp=24")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn every_start_gives_the_same_bag_its_kit_and_coins_alone_and_no_pack_line() {
    // The creation record only: no inventory was made for either hero.
    let running = start(&STARTER_FILES).await;
    std::fs::create_dir_all(store::directory(&running.state)).expect("makes the players folder");
    store::change(&running.state, |_| Ok((built_pair(), ())))
        .await
        .expect("keeps the record");
    crate::seat::scenario::kit_for_day(&running.state, WIZARD)
        .await
        .expect("a bag");
    let first = inventory_of(&running.state, "hero-32");
    // Wizard A: 5 GP; the Sage's B: 50 GP (SRD pp. 77, 83); the Scholar's Pack as its contents,
    // with no line of the pack left behind.
    assert_eq!(held(&running, "hero-32", "gold-piece"), 55);
    assert!(
        first.iter().all(|item| item.id != "scholars-pack"),
        "{first:?}"
    );
    assert!(first.iter().all(|item| item.count > 0), "{first:?}");
    // A run that left more behind (run 9: the kit given on top of the last one), then a second
    // Start: the bag is the kit and coins again, the same as the first.
    give_kit_again(&running.state, WIZARD).await.expect("gives");
    assert_eq!(held(&running, "hero-32", "gold-piece"), 110);
    crate::seat::scenario::kit_for_day(&running.state, WIZARD)
        .await
        .expect("a bag");
    assert_eq!(inventory_of(&running.state, "hero-32"), first);
    let lines: Vec<String> = running
        .state
        .log()
        .newest(512)
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line == "INVENTORY reset owner=hero-32 kind=character items_before=0"),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| {
            line.starts_with("INVENTORY reset owner=hero-32 kind=character items_before=")
                && !line.ends_with("items_before=0")
        }),
        "{lines:?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| *line == "SCENARIO kit hero=32 given")
            .count(),
        2,
        "{lines:?}"
    );
    // A hero with no build in creation waits, with an empty bag.
    crate::seat::scenario::kit_for_day(&running.state, 99)
        .await
        .expect("a bag");
    assert!(
        running
            .state
            .log()
            .newest(16)
            .iter()
            .any(|line| line.text == "SCENARIO kit hero=99 waits reason=no_build")
    );
}
