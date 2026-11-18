//! The SRD 5.2.1 equipment catalog, row by row from the printed tables: Weapons (p.91), Armor
//! (p.92), Tools (pp.93-94), Adventuring Gear (p.95), Ammunition (p.96) and the focus and holy
//! symbol tables (pp.96-97). Container capacities and pack contents are from each item's
//! description, cited where they are set. Coin values: 1 GP = 10 SP = 100 CP (SRD p.89).

use super::{Capacity, EquipmentCategory, EquipmentItem, Weight};
use crate::coins::Price;

use EquipmentCategory::{
    AdventuringGear, Ammunition, ClassGear, HeavyArmor, LightArmor, MartialWeapon, MediumArmor,
    Pack, Shield, SimpleWeapon, Tool,
};

const fn gp(count: u64) -> Price {
    Price::from_copper(count * 100)
}

const fn sp(count: u64) -> Price {
    Price::from_copper(count * 10)
}

const fn cp(count: u64) -> Price {
    Price::from_copper(count)
}

const fn lb(pounds: u64) -> Weight {
    Weight::pounds(pounds)
}

/// A weight with a fraction of a pound, in thousandths (1/2 lb is 500).
const fn mlb(milli_pounds: u64) -> Weight {
    Weight::from_milli_pounds(milli_pounds)
}

/// A weight the table prints as `—`.
const NONE: Weight = Weight::ZERO;

const fn item(
    id: &'static str,
    name: &'static str,
    category: EquipmentCategory,
    (cost, weight): (Price, Weight),
    srd_page: u16,
) -> EquipmentItem {
    EquipmentItem {
        id,
        name,
        category,
        cost: Some(cost),
        per: 1,
        weight,
        srd_page,
        capacity: None,
        contents: &[],
        used_up: false,
    }
}

const fn gear(id: &'static str, name: &'static str, cost: Price, weight: Weight) -> EquipmentItem {
    item(id, name, AdventuringGear, (cost, weight), 95)
}

const fn used_up(item: EquipmentItem) -> EquipmentItem {
    EquipmentItem {
        used_up: true,
        ..item
    }
}

const fn holds(item: EquipmentItem, capacity: Capacity) -> EquipmentItem {
    EquipmentItem {
        capacity: Some(capacity),
        ..item
    }
}

const fn tool(
    id: &'static str,
    name: &'static str,
    cost: Price,
    pounds: u64,
    page: u16,
) -> EquipmentItem {
    item(id, name, Tool, (cost, lb(pounds)), page)
}

const fn weapon(
    id: &'static str,
    name: &'static str,
    category: EquipmentCategory,
    cost: Price,
    weight: Weight,
) -> EquipmentItem {
    item(id, name, category, (cost, weight), 91)
}

const fn armor(
    id: &'static str,
    name: &'static str,
    category: EquipmentCategory,
    cost: Price,
    pounds: u64,
) -> EquipmentItem {
    item(id, name, category, (cost, lb(pounds)), 92)
}

/// A row of the Ammunition table (p.96): `cost` and `weight` buy `per` pieces.
const fn ammunition(
    id: &'static str,
    name: &'static str,
    per: u32,
    cost: Price,
    weight: Weight,
) -> EquipmentItem {
    EquipmentItem {
        per,
        used_up: true,
        ..item(id, name, Ammunition, (cost, weight), 96)
    }
}

const fn pack(
    id: &'static str,
    name: &'static str,
    (cost, weight): (Price, Weight),
    contents: &'static [(&'static str, u32)],
) -> EquipmentItem {
    EquipmentItem {
        contents,
        ..item(id, name, Pack, (cost, weight), 95)
    }
}

/// Holds up to `pounds` within `cubic` thousandths of a cubic foot.
const fn load(pounds: u64, cubic: u32) -> Capacity {
    Capacity {
        weight: Some(lb(pounds)),
        cubic_foot_thousandths: Some(cubic),
        fluid_ounces: None,
        only: &[],
    }
}

/// Holds up to `cubic` thousandths of a cubic foot, with no weight printed.
const fn space(cubic: u32) -> Capacity {
    Capacity {
        weight: None,
        cubic_foot_thousandths: Some(cubic),
        fluid_ounces: None,
        only: &[],
    }
}

/// Holds up to `ounces` fluid ounces of liquid.
const fn liquid(ounces: u32) -> Capacity {
    Capacity {
        weight: None,
        cubic_foot_thousandths: None,
        fluid_ounces: Some(ounces),
        only: &[],
    }
}

/// Holds only the listed things, at most the given count of each alone.
const fn only(things: &'static [(&'static str, u32)]) -> Capacity {
    Capacity {
        weight: None,
        cubic_foot_thousandths: None,
        fluid_ounces: None,
        only: things,
    }
}

/// Every item of the SRD 5.2.1 Equipment chapter with a single cost and weight. "Varies" rows are
/// their printed variants (gaming sets, musical instruments, foci, holy symbols).
pub const SRD_521_EQUIPMENT: &[EquipmentItem] = &[
    // Weapons table, p.91: Simple Melee, Simple Ranged, Martial Melee, Martial Ranged.
    weapon("club", "Club", SimpleWeapon, sp(1), lb(2)),
    weapon("dagger", "Dagger", SimpleWeapon, gp(2), lb(1)),
    weapon("greatclub", "Greatclub", SimpleWeapon, sp(2), lb(10)),
    weapon("handaxe", "Handaxe", SimpleWeapon, gp(5), lb(2)),
    weapon("javelin", "Javelin", SimpleWeapon, sp(5), lb(2)),
    weapon("light-hammer", "Light Hammer", SimpleWeapon, gp(2), lb(2)),
    weapon("mace", "Mace", SimpleWeapon, gp(5), lb(4)),
    weapon("quarterstaff", "Quarterstaff", SimpleWeapon, sp(2), lb(4)),
    weapon("sickle", "Sickle", SimpleWeapon, gp(1), lb(2)),
    weapon("spear", "Spear", SimpleWeapon, gp(1), lb(3)),
    weapon("dart", "Dart", SimpleWeapon, cp(5), mlb(250)),
    weapon(
        "light-crossbow",
        "Light Crossbow",
        SimpleWeapon,
        gp(25),
        lb(5),
    ),
    weapon("shortbow", "Shortbow", SimpleWeapon, gp(25), lb(2)),
    weapon("sling", "Sling", SimpleWeapon, sp(1), NONE),
    weapon("battleaxe", "Battleaxe", MartialWeapon, gp(10), lb(4)),
    weapon("flail", "Flail", MartialWeapon, gp(10), lb(2)),
    weapon("glaive", "Glaive", MartialWeapon, gp(20), lb(6)),
    weapon("greataxe", "Greataxe", MartialWeapon, gp(30), lb(7)),
    weapon("greatsword", "Greatsword", MartialWeapon, gp(50), lb(6)),
    weapon("halberd", "Halberd", MartialWeapon, gp(20), lb(6)),
    weapon("lance", "Lance", MartialWeapon, gp(10), lb(6)),
    weapon("longsword", "Longsword", MartialWeapon, gp(15), lb(3)),
    weapon("maul", "Maul", MartialWeapon, gp(10), lb(10)),
    weapon("morningstar", "Morningstar", MartialWeapon, gp(15), lb(4)),
    weapon("pike", "Pike", MartialWeapon, gp(5), lb(18)),
    weapon("rapier", "Rapier", MartialWeapon, gp(25), lb(2)),
    weapon("scimitar", "Scimitar", MartialWeapon, gp(25), lb(3)),
    weapon("shortsword", "Shortsword", MartialWeapon, gp(10), lb(2)),
    weapon("trident", "Trident", MartialWeapon, gp(5), lb(4)),
    weapon("warhammer", "Warhammer", MartialWeapon, gp(15), lb(5)),
    weapon("war-pick", "War Pick", MartialWeapon, gp(5), lb(2)),
    weapon("whip", "Whip", MartialWeapon, gp(2), lb(3)),
    weapon("blowgun", "Blowgun", MartialWeapon, gp(10), lb(1)),
    weapon(
        "hand-crossbow",
        "Hand Crossbow",
        MartialWeapon,
        gp(75),
        lb(3),
    ),
    weapon(
        "heavy-crossbow",
        "Heavy Crossbow",
        MartialWeapon,
        gp(50),
        lb(18),
    ),
    weapon("longbow", "Longbow", MartialWeapon, gp(50), lb(2)),
    weapon("musket", "Musket", MartialWeapon, gp(500), lb(10)),
    weapon("pistol", "Pistol", MartialWeapon, gp(250), lb(3)),
    // Armor table, p.92.
    armor("padded-armor", "Padded Armor", LightArmor, gp(5), 8),
    armor("leather-armor", "Leather Armor", LightArmor, gp(10), 10),
    armor(
        "studded-leather-armor",
        "Studded Leather Armor",
        LightArmor,
        gp(45),
        13,
    ),
    armor("hide-armor", "Hide Armor", MediumArmor, gp(10), 12),
    armor("chain-shirt", "Chain Shirt", MediumArmor, gp(50), 20),
    armor("scale-mail", "Scale Mail", MediumArmor, gp(50), 45),
    armor("breastplate", "Breastplate", MediumArmor, gp(400), 20),
    armor(
        "half-plate-armor",
        "Half Plate Armor",
        MediumArmor,
        gp(750),
        40,
    ),
    armor("ring-mail", "Ring Mail", HeavyArmor, gp(30), 40),
    armor("chain-mail", "Chain Mail", HeavyArmor, gp(75), 55),
    armor("splint-armor", "Splint Armor", HeavyArmor, gp(200), 60),
    armor("plate-armor", "Plate Armor", HeavyArmor, gp(1_500), 65),
    armor("shield", "Shield", Shield, gp(10), 6),
    // Artisan's Tools, p.93 (Tinker's, Weaver's and Woodcarver's on p.94).
    tool("alchemists-supplies", "Alchemist's Supplies", gp(50), 8, 93),
    tool("brewers-supplies", "Brewer's Supplies", gp(20), 9, 93),
    tool(
        "calligraphers-supplies",
        "Calligrapher's Supplies",
        gp(10),
        5,
        93,
    ),
    tool("carpenters-tools", "Carpenter's Tools", gp(8), 6, 93),
    tool("cartographers-tools", "Cartographer's Tools", gp(15), 6, 93),
    tool("cobblers-tools", "Cobbler's Tools", gp(5), 5, 93),
    tool("cooks-utensils", "Cook's Utensils", gp(1), 8, 93),
    tool("glassblowers-tools", "Glassblower's Tools", gp(30), 5, 93),
    tool("jewelers-tools", "Jeweler's Tools", gp(25), 2, 93),
    tool(
        "leatherworkers-tools",
        "Leatherworker's Tools",
        gp(5),
        5,
        93,
    ),
    tool("masons-tools", "Mason's Tools", gp(10), 8, 93),
    tool("painters-supplies", "Painter's Supplies", gp(10), 5, 93),
    tool("potters-tools", "Potter's Tools", gp(10), 3, 93),
    tool("smiths-tools", "Smith's Tools", gp(20), 8, 93),
    tool("tinkers-tools", "Tinker's Tools", gp(50), 10, 94),
    tool("weavers-tools", "Weaver's Tools", gp(1), 5, 94),
    tool("woodcarvers-tools", "Woodcarver's Tools", gp(1), 5, 94),
    // Other Tools, p.94. A gaming set weighs "—"; its variants and the instruments' are priced.
    tool("disguise-kit", "Disguise Kit", gp(25), 3, 94),
    tool("forgery-kit", "Forgery Kit", gp(15), 5, 94),
    tool("gaming-set-dice", "Gaming Set (dice)", sp(1), 0, 94),
    tool(
        "gaming-set-dragonchess",
        "Gaming Set (dragonchess)",
        gp(1),
        0,
        94,
    ),
    tool(
        "gaming-set-playing-cards",
        "Gaming Set (playing cards)",
        sp(5),
        0,
        94,
    ),
    tool(
        "gaming-set-three-dragon-ante",
        "Gaming Set (three-dragon ante)",
        gp(1),
        0,
        94,
    ),
    tool("herbalism-kit", "Herbalism Kit", gp(5), 3, 94),
    tool("bagpipes", "Bagpipes", gp(30), 6, 94),
    tool("drum", "Drum", gp(6), 3, 94),
    tool("dulcimer", "Dulcimer", gp(25), 10, 94),
    tool("flute", "Flute", gp(2), 1, 94),
    tool("horn", "Horn", gp(3), 2, 94),
    tool("lute", "Lute", gp(35), 2, 94),
    tool("lyre", "Lyre", gp(30), 2, 94),
    tool("pan-flute", "Pan Flute", gp(12), 2, 94),
    tool("shawm", "Shawm", gp(2), 1, 94),
    tool("viol", "Viol", gp(30), 1, 94),
    tool("navigators-tools", "Navigator's Tools", gp(25), 2, 94),
    tool("poisoners-kit", "Poisoner's Kit", gp(50), 2, 94),
    tool("thieves-tools", "Thieves' Tools", gp(25), 1, 94),
    // Adventuring Gear table, p.95. Capacities: Backpack, Barrel, Basket, Glass Bottle, Bucket and
    // the cases p.96; Chest and Flask p.97; Jug p.98; Iron Pot, Pouch, Quiver and Sack p.99; Vial
    // and Waterskin p.100.
    used_up(gear("acid", "Acid", gp(25), lb(1))),
    used_up(gear("alchemists-fire", "Alchemist's Fire", gp(50), lb(1))),
    used_up(gear("antitoxin", "Antitoxin", gp(50), NONE)),
    holds(gear("backpack", "Backpack", gp(2), lb(5)), load(30, 1_000)),
    gear("ball-bearings", "Ball Bearings", gp(1), lb(2)),
    // 40 gallons of liquid or 4 cubic feet of dry goods.
    holds(
        gear("barrel", "Barrel", gp(2), lb(70)),
        Capacity {
            weight: None,
            cubic_foot_thousandths: Some(4_000),
            fluid_ounces: Some(40 * 128),
            only: &[],
        },
    ),
    holds(gear("basket", "Basket", sp(4), lb(2)), load(40, 2_000)),
    gear("bedroll", "Bedroll", gp(1), lb(7)),
    gear("bell", "Bell", gp(1), NONE),
    gear("blanket", "Blanket", sp(5), lb(3)),
    gear("block-and-tackle", "Block and Tackle", gp(1), lb(5)),
    gear("book", "Book", gp(25), lb(5)),
    // 1½ pints.
    holds(
        gear("bottle-glass", "Bottle, Glass", gp(2), lb(2)),
        liquid(24),
    ),
    holds(gear("bucket", "Bucket", cp(5), lb(2)), space(500)),
    gear("caltrops", "Caltrops", gp(1), lb(2)),
    used_up(gear("candle", "Candle", cp(1), NONE)),
    holds(
        gear("case-crossbow-bolt", "Case, Crossbow Bolt", gp(1), lb(1)),
        only(&[("bolt", 20)]),
    ),
    holds(
        gear("case-map-or-scroll", "Case, Map or Scroll", gp(1), lb(1)),
        only(&[("paper", 10), ("parchment", 5)]),
    ),
    gear("chain", "Chain", gp(5), lb(10)),
    holds(gear("chest", "Chest", gp(5), lb(25)), space(12_000)),
    gear("climbers-kit", "Climber's Kit", gp(25), lb(12)),
    gear("fine-clothes", "Clothes, Fine", gp(15), lb(6)),
    gear("travelers-clothes", "Clothes, Traveler's", gp(2), lb(4)),
    gear("component-pouch", "Component Pouch", gp(25), lb(2)),
    gear("costume", "Costume", gp(5), lb(4)),
    gear("crowbar", "Crowbar", gp(2), lb(5)),
    holds(gear("flask", "Flask", cp(2), lb(1)), liquid(16)),
    gear("grappling-hook", "Grappling Hook", gp(2), lb(4)),
    gear("healers-kit", "Healer's Kit", gp(5), lb(3)),
    used_up(gear("holy-water", "Holy Water", gp(25), lb(1))),
    gear("hunting-trap", "Hunting Trap", gp(5), lb(25)),
    used_up(gear("ink", "Ink", gp(10), NONE)),
    gear("ink-pen", "Ink Pen", cp(2), NONE),
    holds(gear("jug", "Jug", cp(2), lb(4)), liquid(128)),
    gear("ladder", "Ladder", sp(1), lb(25)),
    gear("lamp", "Lamp", sp(5), lb(1)),
    gear("lantern-bullseye", "Lantern, Bullseye", gp(10), lb(2)),
    gear("lantern-hooded", "Lantern, Hooded", gp(5), lb(2)),
    gear("lock", "Lock", gp(10), lb(1)),
    gear("magnifying-glass", "Magnifying Glass", gp(100), NONE),
    gear("manacles", "Manacles", gp(2), lb(6)),
    gear("map", "Map", gp(1), NONE),
    gear("mirror", "Mirror", gp(5), mlb(500)),
    gear("net", "Net", gp(1), lb(3)),
    used_up(gear("oil", "Oil", sp(1), lb(1))),
    used_up(gear("paper", "Paper", sp(2), NONE)),
    used_up(gear("parchment", "Parchment", sp(1), NONE)),
    gear("perfume", "Perfume", gp(5), NONE),
    used_up(gear("poison-basic", "Poison, Basic", gp(100), NONE)),
    gear("pole", "Pole", cp(5), lb(7)),
    // 1 gallon.
    holds(gear("pot-iron", "Pot, Iron", gp(2), lb(10)), liquid(128)),
    used_up(gear(
        "potion-of-healing",
        "Potion of Healing",
        gp(50),
        mlb(500),
    )),
    // 6 pounds within one-fifth of a cubic foot.
    holds(gear("pouch", "Pouch", sp(5), lb(1)), load(6, 200)),
    holds(
        gear("quiver", "Quiver", gp(1), lb(1)),
        only(&[("arrow", 20)]),
    ),
    gear("ram-portable", "Ram, Portable", gp(4), lb(35)),
    used_up(gear("rations", "Rations", sp(5), lb(2))),
    gear("robe", "Robe", gp(1), lb(4)),
    gear("rope", "Rope", gp(1), lb(5)),
    holds(gear("sack", "Sack", cp(1), mlb(500)), load(30, 1_000)),
    gear("shovel", "Shovel", gp(2), lb(5)),
    gear("signal-whistle", "Signal Whistle", cp(5), NONE),
    used_up(gear(
        "spell-scroll-cantrip",
        "Spell Scroll (Cantrip)",
        gp(30),
        NONE,
    )),
    used_up(gear(
        "spell-scroll-level-1",
        "Spell Scroll (Level 1)",
        gp(50),
        NONE,
    )),
    gear("spikes-iron", "Spikes, Iron", gp(1), lb(5)),
    gear("spyglass", "Spyglass", gp(1_000), lb(1)),
    gear("string", "String", sp(1), NONE),
    gear("tent", "Tent", gp(2), lb(20)),
    gear("tinderbox", "Tinderbox", sp(5), lb(1)),
    used_up(gear("torch", "Torch", cp(1), lb(1))),
    holds(gear("vial", "Vial", gp(1), NONE), liquid(4)),
    // 4 pints; the weight is full.
    holds(gear("waterskin", "Waterskin", sp(2), lb(5)), liquid(64)),
    // Arcane Focuses table, p.96.
    item(
        "arcane-focus-crystal",
        "Arcane Focus (crystal)",
        AdventuringGear,
        (gp(10), lb(1)),
        96,
    ),
    item(
        "arcane-focus-orb",
        "Arcane Focus (orb)",
        AdventuringGear,
        (gp(20), lb(3)),
        96,
    ),
    item(
        "arcane-focus-rod",
        "Arcane Focus (rod)",
        AdventuringGear,
        (gp(10), lb(2)),
        96,
    ),
    item(
        "arcane-focus-staff",
        "Arcane Focus (staff)",
        AdventuringGear,
        (gp(5), lb(4)),
        96,
    ),
    item(
        "arcane-focus-wand",
        "Arcane Focus (wand)",
        AdventuringGear,
        (gp(10), lb(1)),
        96,
    ),
    // Druidic Focuses and Holy Symbols tables, p.97.
    item(
        "druidic-focus-sprig-of-mistletoe",
        "Druidic Focus (sprig of mistletoe)",
        AdventuringGear,
        (gp(1), NONE),
        97,
    ),
    item(
        "druidic-focus-wooden-staff",
        "Druidic Focus (wooden staff)",
        AdventuringGear,
        (gp(5), lb(4)),
        97,
    ),
    item(
        "druidic-focus-yew-wand",
        "Druidic Focus (yew wand)",
        AdventuringGear,
        (gp(10), lb(1)),
        97,
    ),
    item(
        "holy-symbol-amulet",
        "Holy Symbol (amulet)",
        AdventuringGear,
        (gp(5), lb(1)),
        97,
    ),
    item(
        "holy-symbol-emblem",
        "Holy Symbol (emblem)",
        AdventuringGear,
        (gp(5), NONE),
        97,
    ),
    item(
        "holy-symbol-reliquary",
        "Holy Symbol (reliquary)",
        AdventuringGear,
        (gp(5), lb(2)),
        97,
    ),
    // Wizard, Spellbook, p.78: "a Tiny object that weighs 3 pounds"; no cost is printed.
    EquipmentItem {
        cost: None,
        ..item("spellbook", "Spellbook", ClassGear, (gp(0), lb(3)), 78)
    },
    // Ammunition table, p.96: the amount a cost buys, and what that amount weighs.
    ammunition("arrow", "Arrow", 20, gp(1), lb(1)),
    ammunition("bolt", "Crossbow Bolt", 20, gp(1), mlb(1_500)),
    ammunition("firearm-bullet", "Firearm Bullet", 10, gp(3), lb(2)),
    ammunition("sling-bullet", "Sling Bullet", 20, cp(4), mlb(1_500)),
    ammunition("needle", "Blowgun Needle", 50, gp(1), lb(1)),
    // Packs, p.95; contents from each pack's description (Burglar's p.96; Diplomat's,
    // Dungeoneer's, Entertainer's and Explorer's p.97; Priest's and Scholar's p.99).
    pack(
        "burglars-pack",
        "Burglar's Pack",
        (gp(16), lb(42)),
        &[
            ("backpack", 1),
            ("ball-bearings", 1),
            ("bell", 1),
            ("candle", 10),
            ("crowbar", 1),
            ("lantern-hooded", 1),
            ("oil", 7),
            ("rations", 5),
            ("rope", 1),
            ("tinderbox", 1),
            ("waterskin", 1),
        ],
    ),
    pack(
        "diplomats-pack",
        "Diplomat's Pack",
        (gp(39), lb(39)),
        &[
            ("chest", 1),
            ("fine-clothes", 1),
            ("ink", 1),
            ("ink-pen", 5),
            ("lamp", 1),
            ("case-map-or-scroll", 2),
            ("oil", 4),
            ("paper", 5),
            ("parchment", 5),
            ("perfume", 1),
            ("tinderbox", 1),
        ],
    ),
    pack(
        "dungeoneer-pack",
        "Dungeoneer's Pack",
        (gp(12), lb(55)),
        &[
            ("backpack", 1),
            ("caltrops", 1),
            ("crowbar", 1),
            ("oil", 2),
            ("rations", 10),
            ("rope", 1),
            ("tinderbox", 1),
            ("torch", 10),
            ("waterskin", 1),
        ],
    ),
    pack(
        "entertainers-pack",
        "Entertainer's Pack",
        (gp(40), mlb(58_500)),
        &[
            ("backpack", 1),
            ("bedroll", 1),
            ("bell", 1),
            ("lantern-bullseye", 1),
            ("costume", 3),
            ("mirror", 1),
            ("oil", 8),
            ("rations", 9),
            ("tinderbox", 1),
            ("waterskin", 1),
        ],
    ),
    pack(
        "explorers-pack",
        "Explorer's Pack",
        (gp(10), lb(55)),
        &[
            ("backpack", 1),
            ("bedroll", 1),
            ("oil", 2),
            ("rations", 10),
            ("rope", 1),
            ("tinderbox", 1),
            ("torch", 10),
            ("waterskin", 1),
        ],
    ),
    pack(
        "priests-pack",
        "Priest's Pack",
        (gp(33), lb(29)),
        &[
            ("backpack", 1),
            ("blanket", 1),
            ("holy-water", 1),
            ("lamp", 1),
            ("rations", 7),
            ("robe", 1),
            ("tinderbox", 1),
        ],
    ),
    pack(
        "scholars-pack",
        "Scholar's Pack",
        (gp(40), lb(22)),
        &[
            ("backpack", 1),
            ("book", 1),
            ("ink", 1),
            ("ink-pen", 1),
            ("lamp", 1),
            ("oil", 10),
            ("parchment", 10),
            ("tinderbox", 1),
        ],
    ),
];
