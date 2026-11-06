//! Every closed key set of the crate: `key()` is exactly the serde name, `from_key` inverts it, and
//! no two variants share a key.
use crate::local_journey::{ChallengeRating, HeroSpecies, JourneyRuleError, SpeciesSize};
use crate::species::{
    Ancestry, DraconicAncestry, ElvenLineage, FiendishLegacy, GiantAncestry, GnomishLineage,
};
use crate::tactical::{
    Cover, CreatureKind, DamageType, Lighting, Mastery, Side, Size, SpeciesReaction, Weapon,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::fmt::Debug;

/// Asserts the key contract for a set that is only written.
fn assert_written_keys<T>(all: &[T], key: fn(T) -> &'static str, from_key: fn(&str) -> Option<T>)
where
    T: Copy + Debug + PartialEq + Serialize,
{
    let mut seen: Vec<&str> = Vec::new();
    for &variant in all {
        let text = key(variant);
        assert_eq!(
            serde_json::to_value(variant).expect("a closed key serializes"),
            Value::String(text.to_owned()),
            "{variant:?}"
        );
        assert_eq!(from_key(text), Some(variant), "{text}");
        assert!(!seen.contains(&text), "{text} is used twice");
        seen.push(text);
    }
    assert_eq!(from_key("not-a-key"), None);
}

/// Asserts the key contract for a set that is also read.
fn assert_keys<T>(all: &[T], key: fn(T) -> &'static str, from_key: fn(&str) -> Option<T>)
where
    T: Copy + Debug + PartialEq + Serialize + DeserializeOwned,
{
    assert_written_keys(all, key, from_key);
    for &variant in all {
        let read: T = serde_json::from_value(Value::String(key(variant).to_owned()))
            .expect("a closed key reads back");
        assert_eq!(read, variant);
    }
}

#[test]
fn every_closed_key_is_its_serde_name() {
    assert_written_keys(
        JourneyRuleError::ALL,
        JourneyRuleError::key,
        JourneyRuleError::from_key,
    );
    assert_keys(HeroSpecies::ALL, HeroSpecies::key, HeroSpecies::from_key);
    assert_keys(SpeciesSize::ALL, SpeciesSize::key, SpeciesSize::from_key);
    assert_keys(
        DraconicAncestry::ALL,
        DraconicAncestry::key,
        DraconicAncestry::from_key,
    );
    assert_keys(ElvenLineage::ALL, ElvenLineage::key, ElvenLineage::from_key);
    assert_keys(
        GnomishLineage::ALL,
        GnomishLineage::key,
        GnomishLineage::from_key,
    );
    assert_keys(
        GiantAncestry::ALL,
        GiantAncestry::key,
        GiantAncestry::from_key,
    );
    assert_keys(
        FiendishLegacy::ALL,
        FiendishLegacy::key,
        FiendishLegacy::from_key,
    );
    assert_keys(Lighting::ALL, Lighting::key, Lighting::from_key);
    assert_keys(Cover::ALL, Cover::key, Cover::from_key);
    assert_keys(Side::ALL, Side::key, Side::from_key);
    assert_keys(Size::ALL, Size::key, Size::from_key);
    assert_keys(CreatureKind::ALL, CreatureKind::key, CreatureKind::from_key);
    assert_keys(Weapon::ALL, Weapon::key, Weapon::from_key);
    assert_keys(Mastery::ALL, Mastery::key, Mastery::from_key);
    assert_keys(DamageType::ALL, DamageType::key, DamageType::from_key);
    assert_keys(
        SpeciesReaction::ALL,
        SpeciesReaction::key,
        SpeciesReaction::from_key,
    );
}

#[test]
fn the_old_pascal_case_spelling_is_no_longer_a_key() {
    assert_eq!(Weapon::from_key("LightCrossbow"), None);
    assert!(serde_json::from_value::<Weapon>(json!("LightCrossbow")).is_err());
    assert!(serde_json::from_value::<Side>(json!("Heroes")).is_err());
}

#[test]
fn authored_three_quarters_cover_is_still_read_and_written_in_snake_case() {
    let read: Cover = serde_json::from_value(json!("three-quarters")).expect("authored spelling");
    assert_eq!(read, Cover::ThreeQuarters);
    assert_eq!(
        serde_json::to_value(read).expect("cover serializes"),
        json!("three_quarters")
    );
    assert_eq!(Cover::from_key("three-quarters"), None);
}

#[test]
fn data_carrying_rules_enums_name_their_variants_in_snake_case() {
    assert_eq!(
        serde_json::to_value(ChallengeRating::Quarter).expect("serializes"),
        json!("quarter")
    );
    assert_eq!(
        serde_json::to_value(ChallengeRating::Whole(3)).expect("serializes"),
        json!({ "whole": 3 })
    );
    let ancestry = Ancestry::Elven(ElvenLineage::WoodElf);
    let written = serde_json::to_value(ancestry).expect("serializes");
    assert_eq!(written, json!({ "elven": "wood_elf" }));
    assert_eq!(
        serde_json::from_value::<Ancestry>(written).expect("reads back"),
        ancestry
    );
}
