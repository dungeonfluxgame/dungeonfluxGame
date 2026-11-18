//! A hero of any 2024 core build: the offers, the derived level 1 sheet and "Surprise me"
//! (`planning/phone-and-creation.md` P2; rules coverage R01 and R03).
//!
//! The selected source is SRD 5.2.1 (`development/rules-sources/srd-5.2.1-en.pdf`): its nine
//! species, twelve classes and four backgrounds are offered with their creation facts. A core
//! option it does not print (the Aasimar, twelve backgrounds) is offered by name, design only,
//! and gives the sheet nothing. A build is never played as another one: what the rules cannot
//! play yet (the features of every class but the Fighter, Rogue and Wizard, species traits without
//! a handler, a source not admitted) is a `DesignMark` on the sheet.
//!
//! Reading order: `catalog` (the facts), `choices` (the offers and what a build asks for),
//! `derive` (the checks and the typed build), `numbers` (the sheet's numbers), `sheet` (its
//! shape), `surprise` (a full legal build from a species and class), `vitals` (the live Hit
//! Points, uses and spell slots) and `cast` (whether a spell may be cast). Pure: no I/O, no clock,
//! no dice; "Surprise me" takes the caller's choice of index.
mod cast;
mod catalog;
mod choices;
mod derive;
mod numbers;
mod sheet;
mod surprise;
mod vitals;

pub use cast::{CastError, CastPlan, plan_cast};
pub use catalog::{CoreSpecies, HeroBackground, HeroClass, Tool};
pub use choices::{
    DesignOnlyReason, FightingStyle, HeroOffer, HeroOffers, OfferStatus, because, build_groups,
    hero_offers, option_detail, option_id,
};
pub use derive::{derive_hero, starting_kit};
pub use numbers::{HERO_RULES_SOURCE, HERO_SHEET_SCHEMA, LEVEL_ONE_PROFICIENCY_BONUS};
pub use sheet::{
    AbilityTable, DesignMark, Explained, FeatureStatus, HeroSheet, HitPoints, SheetAttack,
    SheetHealing, SheetMastery, SheetSpell, SlotCount, Spellcasting, StartingItem, StartingKit,
    short_key,
};
pub use surprise::{Pick, fill_build, surprise_build};
pub use vitals::{
    ARCANE_RECOVERY, FeatureUses, HeroVitals, SECOND_WIND, SpellSlots, VitalsChange, VitalsError,
};

/// The choice slots a build fills, for callers that name one.
pub mod slots {
    pub use super::choices::{
        ABILITY_INCREASE, ABILITY_METHOD, ALIGNMENT, BACKGROUND, BACKGROUND_EQUIPMENT, CANTRIPS,
        CLASS, CLASS_EQUIPMENT, CLASS_SKILLS, EXPERTISE, FIGHTING_STYLE, GAMING_SET, INCREASE_ONE,
        INCREASE_TWO, INSTRUMENTS, LANGUAGES, MONK_TOOL, ONE_EACH, POINT_BUY, PREPARED,
        ROGUE_LANGUAGE, SKILLED, SPECIES, SPELLBOOK, STANDARD_ARRAY, TWO_AND_ONE, VERSATILE,
        VERSATILE_SPELL_LIST, WEAPON_MASTERY,
    };
}

#[cfg(test)]
mod hero_tests;
