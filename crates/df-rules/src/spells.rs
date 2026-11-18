//! The spells a level 1 Wizard chooses from: the SRD 5.2.1 Wizard cantrips and level 1 spells
//! (Wizard Spell List, p. 79), each with its facts and a short rules summary in our own words.
//!
//! Owns the catalog (`WIZARD_SPELLS`), lookup by id (`spell`), and the casting rules the catalog
//! needs: a Ritual takes 10 minutes longer and spends no slot (pp. 104, 187). Never does: list a
//! spell the SRD does not print, resolve a spell's effect (the dungeon master narrates it, Jev
//! judges what it reveals), or decide who may cast (`hero::plan_cast`).
use crate::listed::listed_enum;

use self::SpellSchool::{
    Abjuration, Conjuration, Divination, Enchantment, Evocation, Illusion, Necromancy,
    Transmutation,
};

/// The minutes a Ritual adds to a spell's casting time (SRD p. 104, "Rituals"; p. 187).
pub const RITUAL_EXTRA_MINUTES: u16 = 10;

listed_enum! {
    keyed
    /// The eight schools of magic (SRD p. 103).
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SpellSchool {
        Abjuration,
        Conjuration,
        Divination,
        Enchantment,
        Evocation,
        Illusion,
        Necromancy,
        Transmutation,
    }
}

impl SpellSchool {
    pub fn key(self) -> &'static str {
        match self {
            Self::Abjuration => "abjuration",
            Self::Conjuration => "conjuration",
            Self::Divination => "divination",
            Self::Enchantment => "enchantment",
            Self::Evocation => "evocation",
            Self::Illusion => "illusion",
            Self::Necromancy => "necromancy",
            Self::Transmutation => "transmutation",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Abjuration => "Abjuration",
            Self::Conjuration => "Conjuration",
            Self::Divination => "Divination",
            Self::Enchantment => "Enchantment",
            Self::Evocation => "Evocation",
            Self::Illusion => "Illusion",
            Self::Necromancy => "Necromancy",
            Self::Transmutation => "Transmutation",
        }
    }
}

/// One spell's facts. `level` 0 is a cantrip. `casting_time` is the normal casting time; a spell
/// with `ritual` may also be cast as a Ritual.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpellFacts {
    /// The kebab-case id a build picks and a cast names (`magic-missile`).
    pub id: &'static str,
    pub name: &'static str,
    pub level: u8,
    pub school: SpellSchool,
    pub ritual: bool,
    pub concentration: bool,
    pub casting_time: &'static str,
    pub range: &'static str,
    pub components: &'static str,
    pub duration: &'static str,
    /// What the spell does, in one or two plain sentences of our own.
    pub summary: &'static str,
    /// The printed SRD 5.2.1 page of the spell's description.
    pub page: u16,
}

impl SpellFacts {
    pub fn is_cantrip(&self) -> bool {
        self.level == 0
    }
}

/// The spell whose id is `id`, if the catalog has it.
pub fn spell(id: &str) -> Option<&'static SpellFacts> {
    WIZARD_SPELLS.iter().find(|spell| spell.id == id)
}

/// The Wizard spells of `level`, in list order.
pub fn wizard_spells(level: u8) -> impl Iterator<Item = &'static SpellFacts> {
    WIZARD_SPELLS
        .iter()
        .filter(move |spell| spell.level == level)
}

/// The Wizard cantrips and level 1 spells of SRD 5.2.1 (list p. 79; each row cites its
/// description's page). Ritual and Concentration tags follow the list's Special column.
pub const WIZARD_SPELLS: &[SpellFacts] = &[
    SpellFacts {
        id: "acid-splash",
        name: "Acid Splash",
        level: 0,
        school: Evocation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "60 feet",
        components: "V, S",
        duration: "Instantaneous",
        summary: "A bubble of acid bursts in a 5-foot-radius Sphere; each creature in it makes a \
            Dexterity save or takes 1d6 Acid damage.",
        page: 107,
    },
    SpellFacts {
        id: "chill-touch",
        name: "Chill Touch",
        level: 0,
        school: Necromancy,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "Touch",
        components: "V, S",
        duration: "Instantaneous",
        summary: "Melee spell attack for 1d10 Necrotic damage; the target can't regain Hit Points \
            until the end of your next turn.",
        page: 115,
    },
    SpellFacts {
        id: "dancing-lights",
        name: "Dancing Lights",
        level: 0,
        school: Illusion,
        ritual: false,
        concentration: true,
        casting_time: "Action",
        range: "120 feet",
        components: "V, S, M (a bit of phosphorus)",
        duration: "Concentration, up to 1 minute",
        summary: "Up to four hovering torch-like lights (or one glowing humanlike shape), each \
            shedding Dim Light in 10 feet; a Bonus Action moves them.",
        page: 121,
    },
    SpellFacts {
        id: "elementalism",
        name: "Elementalism",
        level: 0,
        school: Transmutation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "30 feet",
        components: "V, S",
        duration: "Instantaneous",
        summary: "A small harmless elemental effect in range: a breeze that shuts doors, dust or a \
            written word in sand, harmless embers and scented smoke, and the like.",
        page: 127,
    },
    SpellFacts {
        id: "fire-bolt",
        name: "Fire Bolt",
        level: 0,
        school: Evocation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "120 feet",
        components: "V, S",
        duration: "Instantaneous",
        summary: "Ranged spell attack for 1d10 Fire damage; an unattended flammable object it hits \
            starts burning.",
        page: 132,
    },
    SpellFacts {
        id: "light",
        name: "Light",
        level: 0,
        school: Evocation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "Touch",
        components: "V, M (a firefly or phosphorescent moss)",
        duration: "1 hour",
        summary: "An object you touch sheds Bright Light in 20 feet and Dim Light 20 feet beyond; \
            covering it blocks the light.",
        page: 144,
    },
    SpellFacts {
        id: "mage-hand",
        name: "Mage Hand",
        level: 0,
        school: Conjuration,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "30 feet",
        components: "V, S",
        duration: "1 minute",
        summary: "A spectral hand handles objects, opens unlocked doors and containers and carries \
            up to 10 pounds; it can't attack.",
        page: 145,
    },
    SpellFacts {
        id: "mending",
        name: "Mending",
        level: 0,
        school: Transmutation,
        ritual: false,
        concentration: false,
        casting_time: "1 minute",
        range: "Touch",
        components: "V, S, M (two lodestones)",
        duration: "Instantaneous",
        summary: "Repairs one break or tear no larger than 1 foot in an object you touch; it \
            doesn't restore magic.",
        page: 148,
    },
    SpellFacts {
        id: "message",
        name: "Message",
        level: 0,
        school: Transmutation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "120 feet",
        components: "S, M (a copper wire)",
        duration: "1 round",
        summary: "You whisper to one creature in range; only it hears, and it can whisper back.",
        page: 148,
    },
    SpellFacts {
        id: "minor-illusion",
        name: "Minor Illusion",
        level: 0,
        school: Illusion,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "30 feet",
        components: "S, M (a bit of fleece)",
        duration: "1 minute",
        summary: "A sound or the image of an object; a Study action and an Intelligence \
            (Investigation) check against your spell save DC reveal it.",
        page: 149,
    },
    SpellFacts {
        id: "poison-spray",
        name: "Poison Spray",
        level: 0,
        school: Necromancy,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "30 feet",
        components: "V, S",
        duration: "Instantaneous",
        summary: "Ranged spell attack for 1d12 Poison damage.",
        page: 153,
    },
    SpellFacts {
        id: "prestidigitation",
        name: "Prestidigitation",
        level: 0,
        school: Transmutation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "10 feet",
        components: "V, S",
        duration: "Up to 1 hour",
        summary: "A small trick: a harmless sensory effect, lighting or snuffing a candle, \
            cleaning or soiling a small object, warming or flavouring, or a small mark.",
        page: 154,
    },
    SpellFacts {
        id: "ray-of-frost",
        name: "Ray of Frost",
        level: 0,
        school: Evocation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "60 feet",
        components: "V, S",
        duration: "Instantaneous",
        summary: "Ranged spell attack for 1d8 Cold damage; the target's Speed drops by 10 feet \
            until the start of your next turn.",
        page: 157,
    },
    SpellFacts {
        id: "shocking-grasp",
        name: "Shocking Grasp",
        level: 0,
        school: Evocation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "Touch",
        components: "V, S",
        duration: "Instantaneous",
        summary: "Melee spell attack for 1d8 Lightning damage; the target can't make Opportunity \
            Attacks until the start of its next turn.",
        page: 162,
    },
    SpellFacts {
        id: "true-strike",
        name: "True Strike",
        level: 0,
        school: Divination,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "Self",
        components: "S, M (a weapon you are proficient with, worth 1+ CP)",
        duration: "Instantaneous",
        summary: "One attack with the weapon using your spellcasting ability; its damage may be \
            Radiant.",
        page: 171,
    },
    SpellFacts {
        id: "alarm",
        name: "Alarm",
        level: 1,
        school: Abjuration,
        ritual: true,
        concentration: false,
        casting_time: "1 minute",
        range: "30 feet",
        components: "V, S, M (a bell and silver wire)",
        duration: "8 hours",
        summary: "Wards a door, window or area up to a 20-foot Cube; you are alerted, aloud or in \
            your mind, when a creature touches or enters it.",
        page: 107,
    },
    SpellFacts {
        id: "burning-hands",
        name: "Burning Hands",
        level: 1,
        school: Evocation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "Self",
        components: "V, S",
        duration: "Instantaneous",
        summary: "15-foot Cone: a Dexterity save against 3d6 Fire damage, half on a success; \
            unattended flammable objects catch fire.",
        page: 114,
    },
    SpellFacts {
        id: "charm-person",
        name: "Charm Person",
        level: 1,
        school: Enchantment,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "30 feet",
        components: "V, S",
        duration: "1 hour",
        summary: "One Humanoid makes a Wisdom save (with Advantage if you fight it) or is Charmed \
            and Friendly to you; it knows afterwards.",
        page: 115,
    },
    SpellFacts {
        id: "chromatic-orb",
        name: "Chromatic Orb",
        level: 1,
        school: Evocation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "90 feet",
        components: "V, S, M (a diamond worth 50+ GP)",
        duration: "Instantaneous",
        summary: "Ranged spell attack with an orb of a chosen element for 3d8 damage; matching \
            dice let it leap to another target.",
        page: 115,
    },
    SpellFacts {
        id: "color-spray",
        name: "Color Spray",
        level: 1,
        school: Illusion,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "Self",
        components: "V, S, M (a pinch of colorful sand)",
        duration: "Instantaneous",
        summary: "15-foot Cone: a Constitution save or Blinded until the end of your next turn.",
        page: 116,
    },
    SpellFacts {
        id: "comprehend-languages",
        name: "Comprehend Languages",
        level: 1,
        school: Divination,
        ritual: true,
        concentration: false,
        casting_time: "Action",
        range: "Self",
        components: "V, S, M (a pinch of soot and salt)",
        duration: "1 hour",
        summary: "You understand the literal meaning of any language you hear or see signed, and \
            writing you touch, about a page a minute; it doesn't decode codes or symbols.",
        page: 117,
    },
    SpellFacts {
        id: "detect-magic",
        name: "Detect Magic",
        level: 1,
        school: Divination,
        ritual: true,
        concentration: true,
        casting_time: "Action",
        range: "Self",
        components: "V, S",
        duration: "Concentration, up to 10 minutes",
        summary: "You sense magic within 30 feet and can see its aura on creatures and objects, \
            and learn a spell's school; 1 foot of stone, dirt or wood, 1 inch of metal or a thin \
            sheet of lead blocks the sense.",
        page: 123,
    },
    SpellFacts {
        id: "disguise-self",
        name: "Disguise Self",
        level: 1,
        school: Illusion,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "Self",
        components: "V, S",
        duration: "1 hour",
        summary: "You and your gear look different; touch, or a Study action with an Intelligence \
            (Investigation) check against your spell save DC, sees through it.",
        page: 124,
    },
    SpellFacts {
        id: "expeditious-retreat",
        name: "Expeditious Retreat",
        level: 1,
        school: Transmutation,
        ritual: false,
        concentration: true,
        casting_time: "Bonus Action",
        range: "Self",
        components: "V, S",
        duration: "Concentration, up to 10 minutes",
        summary: "You Dash now, and can Dash as a Bonus Action until the spell ends.",
        page: 129,
    },
    SpellFacts {
        id: "false-life",
        name: "False Life",
        level: 1,
        school: Necromancy,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "Self",
        components: "V, S, M (a drop of alcohol)",
        duration: "Instantaneous",
        summary: "You gain 2d4 + 4 Temporary Hit Points.",
        page: 129,
    },
    SpellFacts {
        id: "feather-fall",
        name: "Feather Fall",
        level: 1,
        school: Transmutation,
        ritual: false,
        concentration: false,
        casting_time: "Reaction, when you or a creature within 60 feet falls",
        range: "60 feet",
        components: "V, M (a small feather or piece of down)",
        duration: "1 minute",
        summary: "Up to five falling creatures descend at 60 feet a round and take no damage from \
            the fall.",
        page: 130,
    },
    SpellFacts {
        id: "find-familiar",
        name: "Find Familiar",
        level: 1,
        school: Conjuration,
        ritual: true,
        concentration: false,
        casting_time: "1 hour",
        range: "10 feet",
        components: "V, S, M (burning incense worth 10+ GP, consumed)",
        duration: "Instantaneous",
        summary: "A spirit in a small animal form serves you; within 100 feet you speak to it in \
            thought and can borrow its senses.",
        page: 130,
    },
    SpellFacts {
        id: "floating-disk",
        name: "Floating Disk",
        level: 1,
        school: Conjuration,
        ritual: true,
        concentration: false,
        casting_time: "Action",
        range: "30 feet",
        components: "V, S, M (a drop of mercury)",
        duration: "1 hour",
        summary: "A 3-foot disk of force floats 3 feet up, carries up to 500 pounds and follows \
            you within 20 feet.",
        page: 133,
    },
    SpellFacts {
        id: "fog-cloud",
        name: "Fog Cloud",
        level: 1,
        school: Conjuration,
        ritual: false,
        concentration: true,
        casting_time: "Action",
        range: "120 feet",
        components: "V, S",
        duration: "Concentration, up to 1 hour",
        summary: "A 20-foot-radius Sphere of Heavily Obscuring fog until the spell ends or a \
            strong wind scatters it.",
        page: 133,
    },
    SpellFacts {
        id: "grease",
        name: "Grease",
        level: 1,
        school: Conjuration,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "60 feet",
        components: "V, S, M (a bit of pork rind or butter)",
        duration: "1 minute",
        summary: "A 10-foot square turns to Difficult Terrain; creatures in it make a Dexterity \
            save or fall Prone.",
        page: 137,
    },
    SpellFacts {
        id: "hideous-laughter",
        name: "Hideous Laughter",
        level: 1,
        school: Enchantment,
        ritual: false,
        concentration: true,
        casting_time: "Action",
        range: "30 feet",
        components: "V, S, M (a tart and a feather)",
        duration: "Concentration, up to 1 minute",
        summary: "One creature makes a Wisdom save or is Prone and Incapacitated with laughter, \
            saving again each turn and when hurt.",
        page: 140,
    },
    SpellFacts {
        id: "ice-knife",
        name: "Ice Knife",
        level: 1,
        school: Conjuration,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "60 feet",
        components: "S, M (a drop of water or a piece of ice)",
        duration: "Instantaneous",
        summary: "Ranged spell attack for 1d10 Piercing; the shard then bursts and each creature \
            within 5 feet makes a Dexterity save or takes 2d6 Cold.",
        page: 141,
    },
    SpellFacts {
        id: "identify",
        name: "Identify",
        level: 1,
        school: Divination,
        ritual: true,
        concentration: false,
        casting_time: "1 minute",
        range: "Touch",
        components: "V, S, M (a pearl worth 100+ GP)",
        duration: "Instantaneous",
        summary: "You learn a magic item's properties, Attunement, charges and the spells on it, \
            or the spells affecting a creature you touch.",
        page: 142,
    },
    SpellFacts {
        id: "illusory-script",
        name: "Illusory Script",
        level: 1,
        school: Illusion,
        ritual: true,
        concentration: false,
        casting_time: "1 minute",
        range: "Touch",
        components: "S, M (ink worth 10+ GP, consumed)",
        duration: "10 days",
        summary: "Writing that reads as meant to you and those you choose, and as unreadable \
            script to everyone else.",
        page: 142,
    },
    SpellFacts {
        id: "jump",
        name: "Jump",
        level: 1,
        school: Transmutation,
        ritual: false,
        concentration: false,
        casting_time: "Bonus Action",
        range: "Touch",
        components: "V, S, M (a grasshopper's hind leg)",
        duration: "1 minute",
        summary: "A willing creature can jump up to 30 feet once a turn by spending 10 feet of \
            movement.",
        page: 143,
    },
    SpellFacts {
        id: "longstrider",
        name: "Longstrider",
        level: 1,
        school: Transmutation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "Touch",
        components: "V, S, M (a pinch of dirt)",
        duration: "1 hour",
        summary: "A creature you touch has its Speed raised by 10 feet.",
        page: 145,
    },
    SpellFacts {
        id: "mage-armor",
        name: "Mage Armor",
        level: 1,
        school: Abjuration,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "Touch",
        components: "V, S, M (a piece of cured leather)",
        duration: "8 hours",
        summary: "A willing creature without armor has a base AC of 13 plus its Dexterity \
            modifier.",
        page: 145,
    },
    SpellFacts {
        id: "magic-missile",
        name: "Magic Missile",
        level: 1,
        school: Evocation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "120 feet",
        components: "V, S",
        duration: "Instantaneous",
        summary: "Three darts of force each strike a creature you can see for 1d4 + 1 Force \
            damage.",
        page: 146,
    },
    SpellFacts {
        id: "protection-from-evil-and-good",
        name: "Protection from Evil and Good",
        level: 1,
        school: Abjuration,
        ritual: false,
        concentration: true,
        casting_time: "Action",
        range: "Touch",
        components: "V, S, M (a flask of Holy Water worth 25+ GP, consumed)",
        duration: "Concentration, up to 10 minutes",
        summary: "A willing creature is warded from Aberrations, Celestials, Elementals, Fey, \
            Fiends and Undead: they attack it at Disadvantage and can't charm, frighten or possess \
            it.",
        page: 157,
    },
    SpellFacts {
        id: "ray-of-sickness",
        name: "Ray of Sickness",
        level: 1,
        school: Necromancy,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "60 feet",
        components: "V, S",
        duration: "Instantaneous",
        summary: "Ranged spell attack for 2d8 Poison damage; the target is Poisoned until the end \
            of your next turn.",
        page: 158,
    },
    SpellFacts {
        id: "shield",
        name: "Shield",
        level: 1,
        school: Abjuration,
        ritual: false,
        concentration: false,
        casting_time: "Reaction, when an attack roll hits you or Magic Missile targets you",
        range: "Self",
        components: "V, S",
        duration: "1 round",
        summary: "+5 AC until the start of your next turn, against the triggering attack too, and \
            no damage from Magic Missile.",
        page: 161,
    },
    SpellFacts {
        id: "silent-image",
        name: "Silent Image",
        level: 1,
        school: Illusion,
        ritual: false,
        concentration: true,
        casting_time: "Action",
        range: "60 feet",
        components: "V, S, M (a bit of fleece)",
        duration: "Concentration, up to 10 minutes",
        summary: "A purely visual image up to a 15-foot Cube, moved as a Magic action; touch shows \
            it is an illusion.",
        page: 162,
    },
    SpellFacts {
        id: "sleep",
        name: "Sleep",
        level: 1,
        school: Enchantment,
        ritual: false,
        concentration: true,
        casting_time: "Action",
        range: "60 feet",
        components: "V, S, M (a pinch of sand or rose petals)",
        duration: "Concentration, up to 1 minute",
        summary: "Creatures you choose in a 5-foot-radius Sphere save on Wisdom or are \
            Incapacitated, and Unconscious if they fail again.",
        page: 163,
    },
    SpellFacts {
        id: "thunderwave",
        name: "Thunderwave",
        level: 1,
        school: Evocation,
        ritual: false,
        concentration: false,
        casting_time: "Action",
        range: "Self",
        components: "V, S",
        duration: "Instantaneous",
        summary: "15-foot Cube: a Constitution save or 2d8 Thunder damage and a push of 10 feet \
            (half damage, no push, on a success); the boom carries 300 feet.",
        page: 169,
    },
    SpellFacts {
        id: "unseen-servant",
        name: "Unseen Servant",
        level: 1,
        school: Conjuration,
        ritual: true,
        concentration: false,
        casting_time: "Action",
        range: "60 feet",
        components: "V, S, M (a bit of string and of wood)",
        duration: "1 hour",
        summary: "An invisible, mindless force does simple chores at your command; it can't \
            attack.",
        page: 171,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_holds_the_wizard_list_of_srd_page_79() {
        assert_eq!(wizard_spells(0).count(), 15);
        assert_eq!(wizard_spells(1).count(), 30);
        let rituals: Vec<_> = WIZARD_SPELLS
            .iter()
            .filter(|spell| spell.ritual)
            .map(|spell| spell.id)
            .collect();
        assert_eq!(
            rituals,
            [
                "alarm",
                "comprehend-languages",
                "detect-magic",
                "find-familiar",
                "floating-disk",
                "identify",
                "illusory-script",
                "unseen-servant",
            ]
        );
        for id in [
            "light",
            "mage-hand",
            "prestidigitation",
            "detect-magic",
            "comprehend-languages",
            "identify",
        ] {
            assert!(spell(id).is_some(), "{id}");
        }
    }

    #[test]
    fn ids_are_unique_and_kebab_case() {
        for (index, spell) in WIZARD_SPELLS.iter().enumerate() {
            assert!(
                spell
                    .id
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character == '-'),
                "{}",
                spell.id
            );
            assert!(
                WIZARD_SPELLS[..index]
                    .iter()
                    .all(|earlier| earlier.id != spell.id)
            );
        }
    }

    #[test]
    fn school_keys_read_back() {
        for school in SpellSchool::ALL {
            assert_eq!(SpellSchool::from_key(school.key()), Some(*school));
        }
    }
}
