//! Behavior of the gameplay pack and tuning loader, and parity of the shipped
//! `content/harbor-journey` files with the values compiled into the demo today.
use super::pack::{
    BackgroundBonus, COMPILED_ITEMS, CrossRefReason, MAX_FILE_BYTES, PACK_FILE_NAME, PackError,
    ParseErrorKind, Revision, SERVED_SCENE_ASSETS, SceneKey, SourceFile, TUNING_FILE_NAME, load,
    load_from_bytes, render_array_description,
};
use super::{journey, wire};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn shipped_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/harbor-journey")
}

fn shipped_text(name: &str) -> String {
    fs::read_to_string(shipped_directory().join(name)).expect("shipped file must be readable")
}

fn shipped_pack() -> String {
    shipped_text(PACK_FILE_NAME)
}

fn shipped_tuning() -> String {
    include_str!("fixtures/harbor-journey-tuning-baseline.json").to_owned()
}

fn refusal(pack: &str, tuning: &str) -> PackError {
    load_from_bytes(pack.as_bytes(), tuning.as_bytes()).expect_err("input must be refused")
}

/// Apply `change` to the JSON document in `text` and serialize it again.
fn edit(text: &str, change: impl FnOnce(&mut Value)) -> String {
    let mut document: Value = serde_json::from_str(text).expect("fixture must be JSON");
    change(&mut document);
    serde_json::to_string(&document).expect("fixture must serialize")
}

/// Replace (or insert) the value at a JSON pointer such as `/hero/armor_class`.
fn set(text: &str, pointer: &str, value: Value) -> String {
    edit(text, |document| {
        let (parent, key) = pointer
            .rsplit_once('/')
            .expect("pointer must have a parent");
        let parent = document
            .pointer_mut(parent)
            .expect("fixture parent must exist");
        match parent {
            Value::Object(map) => {
                map.insert(key.to_owned(), value);
            }
            Value::Array(items) => {
                let index: usize = key.parse().expect("array pointer must be numeric");
                *items.get_mut(index).expect("fixture index must exist") = value;
            }
            _ => panic!("pointer parent must be a container"),
        }
    })
}

fn remove(text: &str, pointer: &str) -> String {
    edit(text, |document| {
        let (parent, key) = pointer
            .rsplit_once('/')
            .expect("pointer must have a parent");
        match document
            .pointer_mut(parent)
            .expect("fixture parent must exist")
        {
            Value::Object(map) => {
                map.remove(key).expect("fixture key must exist");
            }
            Value::Array(items) => {
                items.remove(key.parse().expect("array pointer must be numeric"));
            }
            _ => panic!("pointer parent must be a container"),
        }
    })
}

/// Replace the first occurrence of `from`, which must exist.
fn replace_first(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "fixture anchor {from:?} must occur");
    text.replacen(from, to, 1)
}

fn assert_bounds(error: PackError, expected_file: SourceFile, expected_path: &str, actual: i64) {
    match error {
        PackError::Bounds {
            file,
            path,
            actual: found,
            ..
        } => {
            assert_eq!(
                (file, path.as_str(), found),
                (expected_file, expected_path, actual)
            );
        }
        other => panic!("expected Bounds at {expected_path}, got {other}"),
    }
}

fn assert_cross_ref(
    error: PackError,
    expected_file: SourceFile,
    expected_path: &str,
    expected_reason: CrossRefReason,
) {
    match error {
        PackError::CrossRef { file, path, reason } => {
            assert_eq!(
                (file, path.as_str(), reason),
                (expected_file, expected_path, expected_reason)
            );
        }
        other => panic!("expected CrossRef at {expected_path}, got {other}"),
    }
}

/// A scratch directory under `artifacts/tmp` that is removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/pack-tests")
            .join(format!("{name}-{}", std::process::id()));
        fs::create_dir_all(&path).expect("scratch directory must be creatable");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Best-effort teardown of test scratch space; a leftover directory is harmless.
        fs::remove_dir_all(&self.0).ok();
    }
}

// ---------------------------------------------------------------------------------------------
// Shipped files and revisions
// ---------------------------------------------------------------------------------------------

#[test]
fn shipped_files_load_with_only_disclosed_tactical_cover_policy() {
    let loaded = load(&shipped_directory()).expect("shipped files must load");

    assert!(
        loaded.departures
            == [
                "tactical.cover-centre-line",
                "tactical.cover-representative-height",
                "tactical.initiative-ties-input-order",
                "tactical.dead-space-passable",
            ],
        "departures: {:?}",
        loaded.departures
    );
    assert_eq!(loaded.pack.id, "harbor-journey");
    assert_eq!(loaded.tuning.pack, loaded.pack.id);
    assert_eq!(loaded.tuning.hero.ability_arrays.len(), 2);
    assert_eq!(loaded.pack.scenes.len(), 8);
}

#[test]
fn shipped_harbor_scenes_have_manifest_pinned_staging_facts() {
    let loaded = load(&shipped_directory()).expect("shipped files must load");

    for scene in &loaded.pack.scenes {
        let setting = scene
            .setting
            .as_ref()
            .expect("harbor scenes must declare their staging setting");
        match scene.key {
            SceneKey::Inn => {
                assert_eq!(setting.place_id, "harbor-inn");
                assert_eq!(
                    setting.kind,
                    super::adventure::setting::SettingKind::NetLoft
                );
                assert_eq!(scene.asset, "assets/ui/generated/scene-salt-quay-interior");
            }
            SceneKey::Room
            | SceneKey::Opening
            | SceneKey::DialogueSeal
            | SceneKey::DialogueEscort
            | SceneKey::Combat
            | SceneKey::Victory
            | SceneKey::Defeat => {
                assert_eq!(setting.place_id, "salt-quay");
                assert_eq!(setting.kind, super::adventure::setting::SettingKind::Quay);
                assert_eq!(scene.asset, "assets/ui/generated/scene-salt-quay-arrival");
            }
        }
    }
}

#[test]
fn revisions_are_sha256_of_the_exact_file_bytes() {
    let loaded = load(&shipped_directory()).expect("shipped files must load");

    for (name, revision) in [
        (PACK_FILE_NAME, loaded.pack_revision),
        (TUNING_FILE_NAME, loaded.tuning_revision),
    ] {
        let bytes = fs::read(shipped_directory().join(name)).expect("file must be readable");
        let expected: [u8; 32] = Sha256::digest(&bytes).into();
        assert_eq!(revision.as_bytes(), &expected, "{name}");
        assert_eq!(revision.to_hex().len(), 64);
    }
    assert_ne!(loaded.pack_revision, loaded.tuning_revision);
}

#[test]
fn revision_matches_the_published_sha256_test_vector() {
    // FIPS 180-2 appendix B.1: SHA-256("abc").
    assert_eq!(
        Revision::of(b"abc").to_hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn whitespace_only_edit_changes_the_revision_but_not_the_values() {
    let original = load_from_bytes(shipped_pack().as_bytes(), shipped_tuning().as_bytes())
        .expect("shipped text must load");
    let edited_tuning = format!("{}\n", shipped_tuning());

    let edited = load_from_bytes(shipped_pack().as_bytes(), edited_tuning.as_bytes())
        .expect("edited text must load");

    assert_ne!(original.tuning_revision, edited.tuning_revision);
    assert_eq!(original.pack_revision, edited.pack_revision);
    assert_eq!(original.tuning, edited.tuning);
}

// ---------------------------------------------------------------------------------------------
// Directory and file errors
// ---------------------------------------------------------------------------------------------

#[test]
fn missing_directory_is_refused_naming_the_pack_file() {
    let scratch = Scratch::new("missing-directory");
    let directory = scratch.0.join("no-such-pack");

    let error = load(&directory).expect_err("a missing directory must not fall back");

    assert_eq!(
        error,
        PackError::Missing {
            file: SourceFile::Pack,
            path: directory.join(PACK_FILE_NAME)
        }
    );
    assert!(error.to_string().contains("pack.json"));
}

#[test]
fn missing_tuning_file_is_refused_naming_the_tuning_file() {
    let scratch = Scratch::new("missing-tuning");
    fs::write(scratch.0.join(PACK_FILE_NAME), shipped_pack()).expect("fixture must be writable");

    let error = load(&scratch.0).expect_err("a missing file must not fall back");

    assert_eq!(
        error,
        PackError::Missing {
            file: SourceFile::Tuning,
            path: scratch.0.join(TUNING_FILE_NAME)
        }
    );
    assert!(error.to_string().contains("tuning.json"));
}

#[test]
fn unreadable_file_is_an_io_error_naming_the_file() {
    let scratch = Scratch::new("unreadable");
    fs::create_dir(scratch.0.join(PACK_FILE_NAME)).expect("fixture must be creatable");

    let error = load(&scratch.0).expect_err("a directory is not a readable file");

    assert!(
        matches!(
            error,
            PackError::Io {
                file: SourceFile::Pack,
                ..
            }
        ),
        "got {error:?}"
    );
}

#[test]
fn oversize_file_is_refused_before_parsing() {
    let scratch = Scratch::new("oversize");
    fs::write(scratch.0.join(PACK_FILE_NAME), shipped_pack()).expect("fixture must be writable");
    let oversize = vec![b' '; usize::try_from(MAX_FILE_BYTES).expect("cap fits") + 1];
    fs::write(scratch.0.join(TUNING_FILE_NAME), &oversize).expect("fixture must be writable");

    let from_disk = load(&scratch.0).expect_err("an oversize file must be refused");
    let from_memory = refusal(
        &shipped_pack(),
        std::str::from_utf8(&oversize).expect("ascii"),
    );

    let expected = PackError::TooLarge {
        file: SourceFile::Tuning,
        bytes: MAX_FILE_BYTES + 1,
        limit: MAX_FILE_BYTES,
    };
    assert_eq!(from_disk, expected);
    assert_eq!(from_memory, expected);
}

#[test]
fn files_loaded_from_a_directory_match_loading_the_same_bytes() {
    let scratch = Scratch::new("round-trip");
    fs::write(scratch.0.join(PACK_FILE_NAME), shipped_pack()).expect("fixture must be writable");
    fs::write(scratch.0.join(TUNING_FILE_NAME), shipped_tuning())
        .expect("fixture must be writable");

    let from_disk = load(&scratch.0).expect("copied files must load");
    let from_memory = load_from_bytes(shipped_pack().as_bytes(), shipped_tuning().as_bytes())
        .expect("same bytes must load");

    assert_eq!(from_disk, from_memory);
}

// ---------------------------------------------------------------------------------------------
// Parse errors: unknown key, duplicate key, wrong type, syntax
// ---------------------------------------------------------------------------------------------

#[test]
fn unknown_key_is_refused_with_file_and_path() {
    let tuning = replace_first(
        &shipped_tuning(),
        r#""rest": { "short_rest_seconds": 3600 }"#,
        r#""rest": { "short_rest_seconds": 3600, "surprise": 1 }"#,
    );

    let error = refusal(&shipped_pack(), &tuning);

    match &error {
        PackError::Parse {
            file,
            path,
            kind,
            message,
            ..
        } => {
            assert_eq!(*file, SourceFile::Tuning);
            assert_eq!(path, "rest.surprise");
            assert_eq!(*kind, ParseErrorKind::Data);
            assert!(message.contains("unknown field `surprise`"), "{message}");
        }
        other => panic!("expected Parse, got {other}"),
    }
    assert!(error.to_string().contains("tuning.json"));
}

#[test]
fn unknown_key_in_a_pack_scene_is_refused_with_its_index() {
    let pack = set(&shipped_pack(), "/scenes/2/mood", json!("grim"));

    match refusal(&pack, &shipped_tuning()) {
        PackError::Parse {
            file,
            path,
            message,
            ..
        } => {
            assert_eq!(file, SourceFile::Pack);
            assert_eq!(path, "scenes[2].mood");
            assert!(message.contains("unknown field `mood`"), "{message}");
        }
        other => panic!("expected Parse, got {other}"),
    }
}

#[test]
fn duplicate_key_is_refused_with_file_and_path() {
    let tuning = replace_first(
        &shipped_tuning(),
        r#""narrative": { "intervention_budget": 8 }"#,
        r#""narrative": { "intervention_budget": 8, "intervention_budget": 9 }"#,
    );

    match refusal(&shipped_pack(), &tuning) {
        PackError::Parse {
            file,
            path,
            kind,
            message,
            ..
        } => {
            assert_eq!(file, SourceFile::Tuning);
            assert_eq!(path, "narrative");
            assert_eq!(kind, ParseErrorKind::Data);
            assert!(
                message.contains("duplicate field `intervention_budget`"),
                "{message}"
            );
        }
        other => panic!("expected Parse, got {other}"),
    }
}

#[test]
fn duplicate_key_in_the_pack_is_refused() {
    let pack = replace_first(
        &shipped_pack(),
        r#""locale": "en","#,
        r#""locale": "en", "locale": "en","#,
    );

    match refusal(&pack, &shipped_tuning()) {
        PackError::Parse { file, message, .. } => {
            assert_eq!(file, SourceFile::Pack);
            assert!(message.contains("duplicate field `locale`"), "{message}");
        }
        other => panic!("expected Parse, got {other}"),
    }
}

#[test]
fn wrong_types_are_refused_at_the_value_path() {
    for (pointer, bad_value, expected_path) in [
        ("/hero/armor_class", json!("17"), "hero.armor_class"),
        ("/hero/armor_class", json!(17.5), "hero.armor_class"),
        ("/hero/armor_class", json!(300), "hero.armor_class"),
        ("/hero/armor_class", json!(-1), "hero.armor_class"),
        (
            "/hero/ability_arrays/0/scores",
            json!([15, 13, 14]),
            "hero.ability_arrays[0].scores",
        ),
        (
            "/bandit/abilities/strength",
            json!(null),
            "bandit.abilities.strength",
        ),
    ] {
        let tuning = set(&shipped_tuning(), pointer, bad_value.clone());

        match refusal(&shipped_pack(), &tuning) {
            PackError::Parse {
                file, path, kind, ..
            } => {
                assert_eq!(
                    (file, path.as_str(), kind),
                    (SourceFile::Tuning, expected_path, ParseErrorKind::Data),
                    "{bad_value}"
                );
            }
            other => panic!("expected Parse for {bad_value}, got {other}"),
        }
    }
}

#[test]
fn wrong_type_in_the_pack_names_the_pack_file_and_path() {
    let pack = set(&shipped_pack(), "/equipment/3/quantity", json!("eight"));

    match refusal(&pack, &shipped_tuning()) {
        PackError::Parse { file, path, .. } => {
            assert_eq!(
                (file, path.as_str()),
                (SourceFile::Pack, "equipment[3].quantity")
            );
        }
        other => panic!("expected Parse, got {other}"),
    }
}

#[test]
fn unknown_scene_key_is_refused_at_its_path() {
    let pack = set(&shipped_pack(), "/scenes/0/key", json!("epilogue"));

    match refusal(&pack, &shipped_tuning()) {
        PackError::Parse { file, path, .. } => {
            assert_eq!((file, path.as_str()), (SourceFile::Pack, "scenes[0].key"));
        }
        other => panic!("expected Parse, got {other}"),
    }
}

#[test]
fn truncated_and_trailing_documents_are_syntax_errors() {
    let truncated = &shipped_tuning()[..100];
    let trailing = format!("{} x", shipped_tuning());

    let early_end = refusal(&shipped_pack(), truncated);
    let extra = refusal(&shipped_pack(), &trailing);

    assert!(matches!(
        early_end,
        PackError::Parse {
            file: SourceFile::Tuning,
            kind: ParseErrorKind::UnexpectedEnd,
            ..
        }
    ));
    assert!(matches!(
        extra,
        PackError::Parse {
            file: SourceFile::Tuning,
            kind: ParseErrorKind::Syntax,
            ..
        }
    ));
}

#[test]
fn parse_message_is_truncated_and_display_excludes_the_document() {
    let long_value = "x".repeat(5000);
    let tuning = set(&shipped_tuning(), "/hero/armor_class", json!(long_value));

    let error = refusal(&shipped_pack(), &tuning);

    let PackError::Parse { message, .. } = &error else {
        panic!("expected Parse, got {error}")
    };
    assert!(
        message.chars().count() <= 161,
        "message kept {} characters",
        message.chars().count()
    );
    assert!(error.to_string().len() < 400);
}

#[test]
fn schema_marker_mismatch_is_refused_for_each_file() {
    let pack = set(&shipped_pack(), "/schema", json!("df-gameplay-pack/2"));
    let tuning = set(&shipped_tuning(), "/schema", json!("df-gameplay-tuning/0"));

    assert!(matches!(
        refusal(&pack, &shipped_tuning()),
        PackError::Schema {
            file: SourceFile::Pack,
            expected: "df-gameplay-pack/1",
            ..
        }
    ));
    assert!(matches!(
        refusal(&shipped_pack(), &tuning),
        PackError::Schema {
            file: SourceFile::Tuning,
            expected: "df-gameplay-tuning/1",
            ..
        }
    ));
}

// ---------------------------------------------------------------------------------------------
// Bounds
// ---------------------------------------------------------------------------------------------

#[test]
fn tuning_numbers_outside_the_bounds_table_are_refused() {
    for (pointer, bad_value, expected_path) in [
        ("/hero/armor_class", 9, "hero.armor_class"),
        ("/hero/armor_class", 31, "hero.armor_class"),
        ("/hero/proficiency_bonus", 1, "hero.proficiency_bonus"),
        ("/hero/proficiency_bonus", 7, "hero.proficiency_bonus"),
        ("/hero/base_hit_points", 0, "hero.base_hit_points"),
        ("/hero/base_hit_points", 61, "hero.base_hit_points"),
        (
            "/hero/background_bonus/strength",
            5,
            "hero.background_bonus.strength",
        ),
        (
            "/hero/background_bonus/constitution",
            5,
            "hero.background_bonus.constitution",
        ),
        (
            "/hero/ability_arrays/1/scores/3",
            2,
            "hero.ability_arrays[1].scores[3]",
        ),
        (
            "/hero/ability_arrays/1/scores/5",
            19,
            "hero.ability_arrays[1].scores[5]",
        ),
        ("/hero/second_wind/uses_max", 6, "hero.second_wind.uses_max"),
        (
            "/hero/second_wind/flat_bonus",
            11,
            "hero.second_wind.flat_bonus",
        ),
        (
            "/hero/second_wind/recharge_per_short_rest",
            3,
            "hero.second_wind.recharge_per_short_rest",
        ),
        ("/hero/weapon/damage_dice", 0, "hero.weapon.damage_dice"),
        ("/hero/weapon/damage_dice", 3, "hero.weapon.damage_dice"),
        ("/bandit/hit_points", 0, "bandit.hit_points"),
        ("/bandit/hit_points", 201, "bandit.hit_points"),
        ("/bandit/armor_class", 0, "bandit.armor_class"),
        ("/bandit/armor_class", 31, "bandit.armor_class"),
        (
            "/bandit/abilities/charisma",
            21,
            "bandit.abilities.charisma",
        ),
        ("/bandit/abilities/wisdom", 0, "bandit.abilities.wisdom"),
        ("/bandit/attack_bonus", -6, "bandit.attack_bonus"),
        ("/bandit/attack_bonus", 21, "bandit.attack_bonus"),
        ("/bandit/damage_modifier", 11, "bandit.damage_modifier"),
        ("/bandit/weapon/damage_dice", 3, "bandit.weapon.damage_dice"),
        ("/combat/round_ticks", 0, "combat.round_ticks"),
        ("/combat/round_ticks", 601, "combat.round_ticks"),
        (
            "/combat/critical_threshold",
            14,
            "combat.critical_threshold",
        ),
        (
            "/combat/critical_threshold",
            21,
            "combat.critical_threshold",
        ),
        ("/combat/combat_log_entries", 0, "combat.combat_log_entries"),
        (
            "/combat/combat_log_entries",
            33,
            "combat.combat_log_entries",
        ),
        ("/rest/short_rest_seconds", 59, "rest.short_rest_seconds"),
        (
            "/rest/short_rest_seconds",
            86_401,
            "rest.short_rest_seconds",
        ),
        (
            "/narrative/intervention_budget",
            0,
            "narrative.intervention_budget",
        ),
        (
            "/narrative/intervention_budget",
            65,
            "narrative.intervention_budget",
        ),
    ] {
        let tuning = set(&shipped_tuning(), pointer, json!(bad_value));

        assert_bounds(
            refusal(&shipped_pack(), &tuning),
            SourceFile::Tuning,
            expected_path,
            bad_value,
        );
    }
}

#[test]
fn background_bonus_cannot_push_a_score_past_twenty() {
    let tuning = set(
        &shipped_tuning(),
        "/hero/ability_arrays/0/scores/0",
        json!(18),
    );
    let tuning = set(&tuning, "/hero/background_bonus/strength", json!(4));

    assert_bounds(
        refusal(&shipped_pack(), &tuning),
        SourceFile::Tuning,
        "hero.ability_arrays[0].scores[0]",
        22,
    );
}

#[test]
fn ability_array_count_is_bounded() {
    let none = set(&shipped_tuning(), "/hero/ability_arrays", json!([]));

    assert_bounds(
        refusal(&shipped_pack(), &none),
        SourceFile::Tuning,
        "hero.ability_arrays",
        0,
    );
}

#[test]
fn die_sides_outside_the_allowed_set_are_refused() {
    for (pointer, path) in [
        ("/hero/second_wind/die_sides", "hero.second_wind.die_sides"),
        ("/hero/weapon/damage_sides", "hero.weapon.damage_sides"),
        ("/bandit/weapon/damage_sides", "bandit.weapon.damage_sides"),
    ] {
        let tuning = set(&shipped_tuning(), pointer, json!(7));

        match refusal(&shipped_pack(), &tuning) {
            PackError::NotAllowed {
                file,
                path: found,
                actual,
                allowed,
            } => {
                assert_eq!(
                    (file, found.as_str(), actual),
                    (SourceFile::Tuning, path, 7)
                );
                assert_eq!(allowed, &[4, 6, 8, 10, 12]);
            }
            other => panic!("expected NotAllowed at {path}, got {other}"),
        }
    }
}

#[test]
fn pack_text_outside_its_length_bounds_is_refused() {
    let long_clue = "c".repeat(513);
    let long_narration = "n".repeat(1025);
    let long_name = "b".repeat(49);
    for (pointer, bad_value, expected_path, actual) in [
        ("/courier_clue", long_clue.clone(), "courier_clue", 513),
        (
            "/scenes/1/narration",
            long_narration.clone(),
            "scenes[1].narration",
            1025,
        ),
        ("/bandit/name", long_name.clone(), "bandit.name", 49),
        ("/bandit/name", String::new(), "bandit.name", 0),
        (
            "/offer_labels/end-turn",
            String::new(),
            "offer_labels.end-turn",
            0,
        ),
        ("/scenes/0/title", String::new(), "scenes[0].title", 0),
        ("/room_code", "L".repeat(33), "room_code", 33),
        (
            "/creation/description",
            String::new(),
            "creation.description",
            0,
        ),
    ] {
        let pack = set(&shipped_pack(), pointer, json!(bad_value));

        assert_bounds(
            refusal(&pack, &shipped_tuning()),
            SourceFile::Pack,
            expected_path,
            actual,
        );
    }
}

#[test]
fn equipment_quantity_and_count_are_bounded() {
    let zero = set(&shipped_pack(), "/equipment/0/quantity", json!(0));
    let many = set(&shipped_pack(), "/equipment/12/quantity", json!(100));
    let empty = set(&shipped_pack(), "/equipment", json!([]));

    assert_bounds(
        refusal(&zero, &shipped_tuning()),
        SourceFile::Pack,
        "equipment[0].quantity",
        0,
    );
    assert_bounds(
        refusal(&many, &shipped_tuning()),
        SourceFile::Pack,
        "equipment[12].quantity",
        100,
    );
    assert_bounds(
        refusal(&empty, &shipped_tuning()),
        SourceFile::Pack,
        "equipment",
        0,
    );
}

#[test]
fn pick_list_must_not_be_empty() {
    let pack = set(&shipped_pack(), "/creation/pick_lists/skill", json!([]));

    assert_bounds(
        refusal(&pack, &shipped_tuning()),
        SourceFile::Pack,
        "creation.pick_lists.skill",
        0,
    );
}

// ---------------------------------------------------------------------------------------------
// Malformed text
// ---------------------------------------------------------------------------------------------

#[test]
fn malformed_text_is_refused_with_the_expected_form() {
    for (pointer, bad_value, expected_path) in [
        ("/locale", "fr", "locale"),
        ("/room_code", "LANT ERN", "room_code"),
        ("/id", "Harbor_Journey", "id"),
        (
            "/creation/groups/0/options/0/id",
            "Dwarf",
            "creation.groups[0].options[0].id",
        ),
        (
            "/creation/groups/3/description_template",
            "STR{9}",
            "creation.groups[3].description_template",
        ),
        (
            "/creation/groups/3/description_template",
            "STR{0",
            "creation.groups[3].description_template",
        ),
        (
            "/escort_support_suffix",
            "No placeholder here.",
            "escort_support_suffix",
        ),
        (
            "/escort_support_suffix",
            "{name} and {name}",
            "escort_support_suffix",
        ),
        (
            "/escort_support_suffix",
            "{name} {other}",
            "escort_support_suffix",
        ),
    ] {
        let pack = set(&shipped_pack(), pointer, json!(bad_value));

        match refusal(&pack, &shipped_tuning()) {
            PackError::Malformed { file, path, .. } => {
                assert_eq!(
                    (file, path.as_str()),
                    (SourceFile::Pack, expected_path),
                    "{bad_value}"
                );
            }
            other => panic!("expected Malformed for {bad_value:?}, got {other}"),
        }
    }
}

#[test]
fn malformed_pack_reference_in_tuning_is_refused() {
    let tuning = set(
        &shipped_tuning(),
        "/hero/ability_arrays/0/id",
        json!("Not Kebab"),
    );

    assert!(matches!(
        refusal(&shipped_pack(), &tuning),
        PackError::Malformed {
            file: SourceFile::Tuning,
            ..
        }
    ));
}

// ---------------------------------------------------------------------------------------------
// Cross references
// ---------------------------------------------------------------------------------------------

#[test]
fn tuning_for_a_different_pack_is_refused() {
    let tuning = set(&shipped_tuning(), "/pack", json!("other-pack"));

    assert_cross_ref(
        refusal(&shipped_pack(), &tuning),
        SourceFile::Tuning,
        "pack",
        CrossRefReason::PackIdMismatch,
    );
}

#[test]
fn ability_array_without_a_creation_option_is_refused() {
    let tuning = set(
        &shipped_tuning(),
        "/hero/ability_arrays/0/id",
        json!("brute"),
    );

    assert_cross_ref(
        refusal(&shipped_pack(), &tuning),
        SourceFile::Tuning,
        "hero.ability_arrays[0].id",
        CrossRefReason::ArrayWithoutOption,
    );
}

#[test]
fn creation_option_without_an_ability_array_is_refused() {
    let pack = edit(&shipped_pack(), |document| {
        let options = document["creation"]["groups"][3]["options"]
            .as_array_mut()
            .expect("array options must exist");
        options.push(json!({ "id": "extra", "label": "Extra" }));
    });

    assert_cross_ref(
        refusal(&pack, &shipped_tuning()),
        SourceFile::Pack,
        "creation.groups[3].options[2].id",
        CrossRefReason::OptionWithoutArray,
    );
}

#[test]
fn duplicate_ability_array_ids_are_refused() {
    let tuning = set(
        &shipped_tuning(),
        "/hero/ability_arrays/1/id",
        json!("stalwart"),
    );

    assert_cross_ref(
        refusal(&shipped_pack(), &tuning),
        SourceFile::Tuning,
        "hero.ability_arrays[1].id",
        CrossRefReason::DuplicateId,
    );
}

#[test]
fn items_outside_the_compiled_set_are_refused() {
    let equipment = set(&shipped_pack(), "/equipment/0/item", json!("lightsaber"));
    let hero_weapon = set(&shipped_tuning(), "/hero/weapon/item", json!("lightsaber"));

    assert_cross_ref(
        refusal(&equipment, &shipped_tuning()),
        SourceFile::Pack,
        "equipment[0].item",
        CrossRefReason::UnknownItem,
    );
    assert_cross_ref(
        refusal(&shipped_pack(), &hero_weapon),
        SourceFile::Tuning,
        "hero.weapon.item",
        CrossRefReason::UnknownItem,
    );
}

#[test]
fn duplicate_equipment_items_are_refused() {
    let pack = set(&shipped_pack(), "/equipment/1/item", json!("chain-mail"));

    assert_cross_ref(
        refusal(&pack, &shipped_tuning()),
        SourceFile::Pack,
        "equipment[1].item",
        CrossRefReason::DuplicateId,
    );
}

#[test]
fn scene_assets_must_be_served() {
    let pack = edit(&shipped_pack(), |document| {
        let scenes = document["scenes"]
            .as_array_mut()
            .expect("scenes must be an array");
        for scene in scenes.iter_mut() {
            scene
                .as_object_mut()
                .expect("scene must be an object")
                .insert("setting".to_owned(), Value::Null);
        }
        scenes[4]["asset"] = json!("assets/ui/scenes/unknown.png");
    });

    assert_cross_ref(
        refusal(&pack, &shipped_tuning()),
        SourceFile::Pack,
        "scenes[4].asset",
        CrossRefReason::AssetNotServed,
    );
}

#[test]
fn every_scene_key_must_appear_exactly_once() {
    let missing = remove(&shipped_pack(), "/scenes/5");
    let duplicate = set(&shipped_pack(), "/scenes/1/key", json!("room"));

    assert_cross_ref(
        refusal(&missing, &shipped_tuning()),
        SourceFile::Pack,
        "scenes",
        CrossRefReason::MissingScene,
    );
    assert_cross_ref(
        refusal(&duplicate, &shipped_tuning()),
        SourceFile::Pack,
        "scenes[1].key",
        CrossRefReason::DuplicateScene,
    );
}

#[test]
fn rested_narration_belongs_to_victory_and_defeat_only() {
    let missing = remove(&shipped_pack(), "/scenes/6/narration_rested");
    let extra = set(
        &shipped_pack(),
        "/scenes/0/narration_rested",
        json!("Unexpected."),
    );

    assert_cross_ref(
        refusal(&missing, &shipped_tuning()),
        SourceFile::Pack,
        "scenes[6].narration_rested",
        CrossRefReason::RestedNarrationMismatch,
    );
    assert_cross_ref(
        refusal(&extra, &shipped_tuning()),
        SourceFile::Pack,
        "scenes[0].narration_rested",
        CrossRefReason::RestedNarrationMismatch,
    );
}

#[test]
fn rested_story_text_keeps_short_rest_rules_out_of_narration() {
    let loaded = load_from_bytes(shipped_pack().as_bytes(), shipped_tuning().as_bytes()).unwrap();

    super::pack::with_test_pack(loaded, || {
        for key in [
            super::pack::SceneKey::Victory,
            super::pack::SceneKey::Defeat,
        ] {
            let scene = super::pack::scene(key);
            let (narration, rules) = super::pack::rested_story_and_rules(&scene).unwrap();
            assert!(narration.contains("hour catching its breath"), "{key:?}");
            assert!(!narration.contains("Hit Point Dice"), "{key:?}");
            assert!(!narration.contains("Second Wind"), "{key:?}");
            assert!(!narration.contains("stable 0-HP"), "{key:?}");
            assert!(rules.contains("Hit Point Dice"), "{key:?}");
            assert!(rules.contains("Second Wind"), "{key:?}");
            assert!(rules.contains("1d4 hours"), "{key:?}");
        }
    });
}

#[test]
fn creation_groups_must_be_the_supported_set() {
    let unknown = set(&shipped_pack(), "/creation/groups/0/id", json!("mystery"));
    let missing = remove(&shipped_pack(), "/creation/groups/0");
    let duplicated = edit(&shipped_pack(), |document| {
        let copy = document["creation"]["groups"][0].clone();
        document["creation"]["groups"]
            .as_array_mut()
            .expect("groups must exist")
            .push(copy);
    });
    let foreign_option = set(
        &shipped_pack(),
        "/creation/groups/0/options/0/id",
        json!("elf"),
    );

    assert_cross_ref(
        refusal(&unknown, &shipped_tuning()),
        SourceFile::Pack,
        "creation.groups[0].id",
        CrossRefReason::UnknownGroup,
    );
    assert_cross_ref(
        refusal(&missing, &shipped_tuning()),
        SourceFile::Pack,
        "creation.groups",
        CrossRefReason::MissingGroup,
    );
    assert_cross_ref(
        refusal(&duplicated, &shipped_tuning()),
        SourceFile::Pack,
        "creation.groups[7].id",
        CrossRefReason::DuplicateGroup,
    );
    assert_cross_ref(
        refusal(&foreign_option, &shipped_tuning()),
        SourceFile::Pack,
        "creation.groups[0].options[0].id",
        CrossRefReason::UnknownOption,
    );
}

#[test]
fn stored_legacy_rest_sentences_project_story_and_current_rules_without_claiming_revival() {
    let pack: super::pack::PackFile =
        serde_json::from_slice(include_bytes!("fixtures/harbor-journey-pack-baseline.json"))
            .unwrap();
    for scene in pack
        .scenes
        .iter()
        .filter(|scene| scene.narration_rested.is_some())
    {
        let (story, rules) = super::pack::rested_story_and_rules(scene).unwrap();
        assert!(story.contains("The party spends an hour catching its breath."));
        assert!(!story.contains("awaken") && !story.contains("awake with 1 HP"));
        assert!(!story.contains("Hit Point Dice"));
        assert!(rules.contains("chooses whether to spend Hit Point Dice"));
        assert!(rules.contains("1d4 hours") && rules.contains("dead creature remains dead"));
    }
}

#[test]
fn authored_rest_story_does_not_require_mechanical_sentences_to_render() {
    let mut scene = super::pack::scene(super::pack::SceneKey::Victory);
    scene.narration_rested = Some("The party trades stories beside the hearth.".into());
    let (story, rules) = super::pack::rested_story_and_rules(&scene).unwrap();
    assert_eq!(story, "The party trades stories beside the hearth.");
    assert!(rules.contains("chooses whether to spend Hit Point Dice"));
    scene.narration_rested = Some(String::new());
    assert!(super::pack::rested_story_and_rules(&scene).is_err());
}

#[test]
fn description_template_belongs_to_the_array_group_only() {
    let without = remove(&shipped_pack(), "/creation/groups/3/description_template");
    let misplaced = set(
        &shipped_pack(),
        "/creation/groups/0/description_template",
        json!("{0}"),
    );

    assert_cross_ref(
        refusal(&without, &shipped_tuning()),
        SourceFile::Pack,
        "creation.groups[3].description_template",
        CrossRefReason::TemplateMisplaced,
    );
    assert_cross_ref(
        refusal(&misplaced, &shipped_tuning()),
        SourceFile::Pack,
        "creation.groups[0].description_template",
        CrossRefReason::TemplateMisplaced,
    );
}

#[test]
fn pick_list_values_must_be_accepted_by_the_compiled_validator() {
    let unknown = set(
        &shipped_pack(),
        "/creation/pick_lists/language/0",
        json!("klingon"),
    );
    let repeated = set(
        &shipped_pack(),
        "/creation/pick_lists/alignment/1",
        json!("lawful-good"),
    );

    assert_cross_ref(
        refusal(&unknown, &shipped_tuning()),
        SourceFile::Pack,
        "creation.pick_lists.language[0]",
        CrossRefReason::UnknownPickValue,
    );
    assert_cross_ref(
        refusal(&repeated, &shipped_tuning()),
        SourceFile::Pack,
        "creation.pick_lists.alignment[1]",
        CrossRefReason::DuplicateId,
    );
}

#[test]
fn cross_reference_errors_name_the_file_and_path_in_display() {
    let tuning = set(&shipped_tuning(), "/pack", json!("other-pack"));

    let message = refusal(&shipped_pack(), &tuning).to_string();

    assert!(
        message.contains("tuning.json") && message.contains("pack"),
        "{message}"
    );
}

// ---------------------------------------------------------------------------------------------
// Departures
// ---------------------------------------------------------------------------------------------

#[test]
fn changed_standard_values_are_reported_as_departures_in_a_fixed_order() {
    let tuning = set(&shipped_tuning(), "/hero/armor_class", json!(16));
    let tuning = set(&tuning, "/hero/weapon/damage_sides", json!(8));
    let tuning = set(&tuning, "/bandit/hit_points", json!(20));
    let tuning = set(&tuning, "/combat/round_ticks", json!(8));
    let tuning = set(&tuning, "/rest/short_rest_seconds", json!(1800));

    let loaded = load_from_bytes(shipped_pack().as_bytes(), tuning.as_bytes())
        .expect("in-bounds edits must load");

    assert_eq!(
        loaded.departures,
        [
            "hero.armor_class",
            "hero.weapon.damage_sides",
            "bandit.hit_points",
            "combat.round_ticks",
            "rest.short_rest_seconds"
        ]
    );
}

#[test]
fn non_standard_values_are_not_departures() {
    let tuning = set(&shipped_tuning(), "/combat/combat_log_entries", json!(10));
    let tuning = set(&tuning, "/narrative/intervention_budget", json!(9));
    let pack = set(&shipped_pack(), "/bandit/name", json!("Wharf Thug"));
    let pack = set(&pack, "/scenes/0/title", json!("A new title"));
    let pack = set(&pack, "/room_code", json!("HARBOR"));

    let loaded = load_from_bytes(pack.as_bytes(), tuning.as_bytes()).expect("edits must load");

    assert!(
        loaded.departures.is_empty(),
        "departures: {:?}",
        loaded.departures
    );
}

#[test]
fn changed_arrays_pick_lists_and_equipment_are_departures() {
    let tuning = set(
        &shipped_tuning(),
        "/hero/ability_arrays/0/scores",
        json!([15, 13, 14, 10, 12, 9]),
    );
    let pack = remove(&shipped_pack(), "/creation/pick_lists/language/8");
    let pack = set(&pack, "/equipment/12/quantity", json!(19));

    let loaded = load_from_bytes(pack.as_bytes(), tuning.as_bytes()).expect("edits must load");

    assert_eq!(
        loaded.departures,
        [
            "hero.ability_arrays",
            "creation.pick_lists.language",
            "equipment"
        ]
    );
}

#[test]
fn reordering_a_pick_list_is_not_a_departure() {
    let pack = edit(&shipped_pack(), |document| {
        document["creation"]["pick_lists"]["skill"]
            .as_array_mut()
            .expect("skill list must exist")
            .reverse();
    });

    let loaded =
        load_from_bytes(pack.as_bytes(), shipped_tuning().as_bytes()).expect("edit must load");

    assert!(
        loaded.departures.is_empty(),
        "departures: {:?}",
        loaded.departures
    );
}

// ---------------------------------------------------------------------------------------------
// Parity with the values compiled into the demo today. These tests exist to catch drift between
// the shipped files and the code until PLAY-B02 steps 3 and 4 make the code read the files; the
// source-text checks must be retired together with the literals they audit.
// ---------------------------------------------------------------------------------------------

#[test]
fn shipped_pick_lists_equal_the_compiled_rules_lists() {
    let loaded = load(&shipped_directory()).expect("shipped files must load");
    let lists = &loaded.pack.creation.pick_lists;

    assert_eq!(lists.alignment, df_rules::local_journey::ALIGNMENTS);
    assert_eq!(lists.language, df_rules::local_journey::STANDARD_LANGUAGES);
    assert_eq!(lists.skill, df_rules::local_journey::FIGHTER_SKILLS);
    assert_eq!(lists.gaming_set, df_rules::local_journey::GAMING_SETS);
}

#[test]
fn shipped_room_code_equals_the_compiled_room_code() {
    let loaded = load(&shipped_directory()).expect("shipped files must load");

    assert_eq!(loaded.pack.room_code, journey::ROOM_CODE);
}

#[test]
fn shipped_courier_clue_equals_the_prepared_recording() {
    let loaded = load(&shipped_directory()).expect("shipped files must load");

    assert_eq!(loaded.pack.courier_clue, super::courier_ai::RESPONSE);
}

#[test]
fn shipped_creation_content_equals_the_wire_creation_offer() {
    let loaded = load(&shipped_directory()).expect("shipped files must load");
    let offer = wire::creation_offer().unwrap();

    assert_eq!(loaded.pack.creation.description, offer.description);
    for group in &loaded.pack.creation.groups {
        let wire_group = offer
            .groups
            .iter()
            .find(|candidate| candidate.group_id == group.id)
            .unwrap_or_else(|| panic!("wire offer has no group {}", group.id));
        assert_eq!(wire_group.label, group.label, "{}", group.id);
        assert_eq!(
            wire_group.options.len(),
            group.options.len(),
            "{}",
            group.id
        );
        for (wire_option, option) in wire_group.options.iter().zip(&group.options) {
            let expected_description = match (&group.description_template, &option.description) {
                (Some(template), _) => {
                    let array = loaded
                        .tuning
                        .hero
                        .ability_arrays
                        .iter()
                        .find(|array| array.id == option.id)
                        .expect("every array option has an array");
                    render_array_description(
                        template,
                        array.scores,
                        loaded.tuning.hero.background_bonus,
                    )
                }
                (None, Some(description)) => description.clone(),
                (None, None) => String::new(),
            };
            assert_eq!(wire_option.option_id, option.id);
            assert_eq!(wire_option.label, option.label, "{}", option.id);
            assert_eq!(
                wire_option.description, expected_description,
                "{}",
                option.id
            );
        }
    }
}

#[test]
fn array_template_renders_scores_after_the_background_bonus() {
    let text = render_array_description(
        "STR{0} DEX{1} CON{2} INT{3} WIS{4} CHA{5} +{bonus_str}/+{bonus_con}",
        [15, 13, 14, 10, 12, 8],
        BackgroundBonus {
            strength: 2,
            constitution: 1,
        },
    );

    assert_eq!(text, "STR17 DEX13 CON15 INT10 WIS12 CHA8 +2/+1");
}

#[test]
fn shipped_resource_bounds_equal_the_journey_resource_constraints() {
    let loaded = load(&shipped_directory()).expect("shipped files must load");
    let constraints = journey::resources().expect("journey resources must build");
    let bounds = |owner: [u8; 16], name: &str| {
        let owner = journey::entity(owner).expect("fixed id must be valid");
        let found = constraints
            .iter()
            .find(|constraint| constraint.owner == owner && constraint.resource.as_str() == name)
            .unwrap_or_else(|| panic!("no constraint {name}"));
        (found.minimum, found.maximum)
    };
    let hero_constitution_modifier = loaded
        .tuning
        .hero
        .ability_arrays
        .iter()
        .map(|array| {
            let score = array.scores[2] + loaded.tuning.hero.background_bonus.constitution;
            df_rules::local_journey::ability_modifier(score)
        })
        .max()
        .expect("at least one array");
    let hero_hit_point_maximum =
        i64::from(loaded.tuning.hero.base_hit_points) + i64::from(hero_constitution_modifier);
    let armor_class = i64::from(loaded.tuning.hero.armor_class);

    for hero in [[0x63; 16], [0x64; 16]] {
        assert_eq!(bounds(hero, "hit-points"), (1, hero_hit_point_maximum));
        assert_eq!(bounds(hero, "armor-class"), (armor_class, armor_class));
        assert_eq!(
            bounds(hero, "second-wind"),
            (0, i64::from(loaded.tuning.hero.second_wind.uses_max))
        );
    }
    let bandit = [0x65; 16];
    assert_eq!(
        bounds(bandit, "hit-points"),
        (1, i64::from(loaded.tuning.bandit.hit_points))
    );
    let bandit_armor_class = i64::from(loaded.tuning.bandit.armor_class);
    assert_eq!(
        bounds(bandit, "armor-class"),
        (bandit_armor_class, bandit_armor_class)
    );
}
#[test]
fn shipped_scene_assets_are_the_served_paths() {
    let loaded = load(&shipped_directory()).expect("shipped files must load");
    let served = include_str!("public_assets.rs");

    for path in SERVED_SCENE_ASSETS {
        assert!(
            served.contains(&format!("\"/{path}\"")),
            "{path} is not served"
        );
    }
    for scene in &loaded.pack.scenes {
        assert!(
            SERVED_SCENE_ASSETS.contains(&scene.asset.as_str())
                || super::adventure::generated_asset(&scene.asset),
            "{} unused",
            scene.asset
        );
    }
}

#[test]
fn compiled_item_set_is_contained_in_the_journey_content_entries() {
    for item in COMPILED_ITEMS {
        assert!(
            journey::CONTENT_ENTRIES.contains(item),
            "{item} is not a content entry"
        );
    }
}

#[test]
fn selected_hero_weapon_requires_starting_equipment() {
    let tuning = set(&shipped_tuning(), "/hero/weapon/item", json!("scimitar"));
    assert!(
        matches!(refusal(&shipped_pack(), &tuning), PackError::CrossRef {
        file: SourceFile::Tuning, path, reason: CrossRefReason::WeaponNotEquipped
    } if path == "hero.weapon.item")
    );
}

#[test]
fn antagonist_weapon_is_a_bounded_display_label() {
    let tuning = set(
        &shipped_tuning(),
        "/bandit/weapon/item",
        json!("ritual sickle"),
    );
    let loaded = load_from_bytes(shipped_pack().as_bytes(), tuning.as_bytes()).unwrap();
    assert_eq!(loaded.tuning.bandit.weapon.item, "ritual sickle");
    let tuning = set(
        &shipped_tuning(),
        "/bandit/weapon/item",
        json!("x".repeat(33)),
    );
    assert!(matches!(
        load_from_bytes(shipped_pack().as_bytes(), tuning.as_bytes()),
        Err(PackError::Bounds { .. })
    ));
}

#[test]
fn shipped_pack_puts_its_characters_on_stage_and_unknown_sprites_are_just_strings() {
    use super::pack::{CastRole, SceneKey};
    let loaded = load_from_bytes(shipped_pack().as_bytes(), shipped_tuning().as_bytes()).unwrap();
    let cast = |key: SceneKey| -> Vec<(String, CastRole, bool)> {
        let scene = loaded
            .pack
            .scenes
            .iter()
            .find(|scene| scene.key == key)
            .unwrap();
        super::wire::scene_cast(scene)
            .into_iter()
            .zip(&scene.cast)
            .map(|(actor, member)| (actor.name, member.role, actor.speaking))
            .collect()
    };
    assert_eq!(
        cast(SceneKey::Opening),
        [("Courier".to_owned(), CastRole::Patron, true)]
    );
    assert_eq!(
        cast(SceneKey::DialogueSeal),
        [
            ("Courier".to_owned(), CastRole::Patron, true),
            ("Bandit".to_owned(), CastRole::Antagonist, false)
        ]
    );
    assert_eq!(
        cast(SceneKey::Combat),
        [("Bandit".to_owned(), CastRole::Antagonist, true)]
    );
    assert_eq!(cast(SceneKey::Inn).len(), 1);
    assert!(cast(SceneKey::Room).is_empty());
    assert!(cast(SceneKey::Defeat).is_empty());
    let sprite = &loaded.pack.scenes[1].cast[0].sprite;
    assert_eq!(sprite, "assets/ui/generated/sprite-courier");
    // A sprite the client has never heard of is not a loader failure.
    let pack = set(
        &shipped_pack(),
        "/scenes/1/cast/0/sprite",
        json!("assets/elsewhere/not-a-known-stem"),
    );
    assert!(load_from_bytes(pack.as_bytes(), shipped_tuning().as_bytes()).is_ok());
    // But the cast is bounded.
    let pack = set(&shipped_pack(), "/scenes/1/cast/0/name", json!(""));
    assert!(matches!(
        refusal(&pack, &shipped_tuning()),
        PackError::Bounds { .. }
    ));
}

#[test]
fn shipped_pack_lists_its_secret_terms_and_speakers_and_bad_ones_are_refused() {
    let loaded = load_from_bytes(shipped_pack().as_bytes(), shipped_tuning().as_bytes()).unwrap();
    assert_eq!(loaded.pack.secret_terms, ["Vell", "Harbor Inn"]);
    let names: Vec<&str> = loaded
        .pack
        .speakers
        .iter()
        .map(|speaker| speaker.name.as_str())
        .collect();
    assert_eq!(names, ["Courier", "Bandit"]);
    for scene in &loaded.pack.scenes {
        assert!(
            scene.speaker.is_empty() || names.contains(&scene.speaker.as_str()),
            "every shown speaker has a profile"
        );
    }
    for (pointer, value) in [
        ("/speakers/0/role", json!("")),
        ("/speakers/0/voice", json!("v".repeat(97))),
        ("/secret_terms/0", json!("")),
    ] {
        let pack = set(&shipped_pack(), pointer, value);
        assert!(
            matches!(refusal(&pack, &shipped_tuning()), PackError::Bounds { .. }),
            "{pointer}"
        );
    }
    use super::pack::{Age, Gender};
    assert_eq!(loaded.pack.speakers[0].gender, Some(Gender::He));
    assert_eq!(loaded.pack.speakers[0].age, Some(Age::Young));
    assert_eq!(
        super::pack::with_test_pack(loaded.clone(), super::pack::speakers).len(),
        2
    );
    for (pointer, value) in [
        ("/speakers/0/gender", json!("xe")),
        ("/speakers/0/age", json!("ancient")),
        ("/scenes/1/cast/0/gender", json!("")),
    ] {
        let pack = set(&shipped_pack(), pointer, value);
        assert!(
            matches!(refusal(&pack, &shipped_tuning()), PackError::Parse { .. }),
            "{pointer}"
        );
    }
    let pack = edit(&shipped_pack(), |document| {
        document["speakers"][0]["mood"] = json!("cheerful");
    });
    assert!(matches!(
        refusal(&pack, &shipped_tuning()),
        PackError::Parse { .. }
    ));
}

#[test]
fn duplicate_speaker_names_are_refused_regardless_of_profile_order() {
    for prepend in [true, false] {
        let pack = edit(&shipped_pack(), |document| {
            let speakers = document["speakers"]
                .as_array_mut()
                .expect("speakers must be an array");
            let mut duplicate = speakers[0].clone();
            duplicate["role"] = json!("An unsafe conflicting role");
            duplicate["voice"] = json!("A conflicting voice");
            duplicate["age"] = serde_json::Value::Null;
            if prepend {
                speakers.insert(0, duplicate);
            } else {
                speakers.push(duplicate);
            }
        });

        let duplicate_path = if prepend {
            "speakers[1].name"
        } else {
            "speakers[2].name"
        };
        assert_cross_ref(
            refusal(&pack, &shipped_tuning()),
            SourceFile::Pack,
            duplicate_path,
            CrossRefReason::DuplicateId,
        );
    }
}

#[test]
fn tempo_is_optional_revisioned_presentation_tuning_without_rules_departures() {
    let pack = shipped_pack();
    let tuning = shipped_tuning();
    let shipped = load_from_bytes(pack.as_bytes(), tuning.as_bytes()).unwrap();
    let omitted = remove(&tuning, "/tempo");
    let legacy = load_from_bytes(pack.as_bytes(), omitted.as_bytes()).unwrap();
    assert_eq!(legacy.tuning.tempo, shipped.tuning.tempo);
    let edited = set(&tuning, "/tempo/impulses_permille/critical", json!(200));
    let changed = load_from_bytes(pack.as_bytes(), edited.as_bytes()).unwrap();
    assert!(changed.departures.is_empty());
    assert_ne!(shipped.tuning_revision, changed.tuning_revision);
    assert_ne!(shipped.tuning.tempo, changed.tuning.tempo);
}

#[test]
fn invalid_tempo_is_refused_with_a_typed_tuning_error() {
    for (pointer, value) in [
        ("/tempo/base_intensity_permille/combat", json!(1001)),
        ("/tempo/impulses_permille/critical", json!(1001)),
        ("/tempo/inertia_half_life_millis", json!(0)),
        ("/tempo/phase_thresholds_permille", json!([500, 200, 600])),
        ("/tempo/fatigue_charge_permille", json!(1001)),
        ("/tempo/hysteresis_permille", json!(300)),
        ("/tempo/release_hold_millis", json!(999)),
        ("/tempo/release_hold_millis", json!(120_001)),
    ] {
        let changed = set(&shipped_tuning(), pointer, value);
        assert!(matches!(
            refusal(&shipped_pack(), &changed),
            PackError::Bounds {
                file: SourceFile::Tuning,
                ..
            } | PackError::Malformed {
                file: SourceFile::Tuning,
                ..
            }
        ));
    }
    let unknown = set(&shipped_tuning(), "/tempo/secret", json!(1));
    assert!(matches!(
        refusal(&shipped_pack(), &unknown),
        PackError::Parse {
            file: SourceFile::Tuning,
            ..
        }
    ));
}

mod adventure {
    use super::*;

    #[test]
    fn generated_scene_findings_are_accepted_srd_5_2_1_pages_6_186_187_189() {
        let pack = edit(&shipped_pack(), |document| {
            document["scenes"][1]["findings"] = json!([
                {
                    "id": "ledger-mark",
                    "sentence": "The ledger's last page is torn out.",
                    "skill": "investigation",
                    "dc": 25,
                    "private": false,
                    "consequence": "enemy-weakness"
                },
                {
                    "id": "wet-footprints",
                    "sentence": "Wet footprints lead toward the ferry gate.",
                    "skill": "perception",
                    "dc": 30,
                    "private": false
                }
            ]);
        });

        assert!(
            load_from_bytes(pack.as_bytes(), shipped_tuning().as_bytes()).is_ok(),
            "generated scene findings should be part of the scene contract"
        );
    }
}
