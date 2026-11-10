//! The `scene roll` verb: what it reads into, what it refuses, and that what it reads is a roll
//! the page can draw.

use df_playground::fixture::{
    DieRole, InspirationReroll, NaturalMark, RollDie, RollKind, RollMode, RollModifier,
    RollOutcome, RollTarget, RollView,
};
use df_playground::scene::SceneChange;

use crate::command::{Command, CommandError, parse, split_words};

fn read(line: &str) -> Result<(RollView, u64), CommandError> {
    match parse(&split_words(line).expect("the line splits"))? {
        Command::Scene(SceneChange::Roll { view, min_hold_ms }) => Ok((view, min_hold_ms)),
        other => panic!("expected a roll, got {other:?}"),
    }
}

#[test]
fn a_skill_check_reads_its_dice_modifiers_target_and_verdict_as_given() {
    let (view, hold) = read(concat!(
        r#"scene roll skill_check "Athletics (Strength)" id=r1 dice=20:14 "#,
        r#"mods=Strength:3,Proficiency:2 target=DC:15 outcome=success hold=9300"#,
    ))
    .expect("reads");
    assert_eq!(hold, 9_300);
    assert_eq!(
        view,
        RollView {
            id: "r1".to_owned(),
            title: "Athletics (Strength)".to_owned(),
            kind: RollKind::SkillCheck,
            mode: RollMode::Normal,
            dice: vec![RollDie {
                sides: 20,
                face: 14,
                role: DieRole::Test,
                kept: true
            }],
            modifiers: vec![
                RollModifier {
                    label: "Strength".to_owned(),
                    value: 3
                },
                RollModifier {
                    label: "Proficiency".to_owned(),
                    value: 2
                },
            ],
            total: 19,
            target: Some(RollTarget {
                label: "DC".to_owned(),
                value: 15
            }),
            outcome: Some(RollOutcome::Success),
            natural: NaturalMark::Unmarked,
            inspiration_reroll: None,
        }
    );
    assert_eq!(view.check(), Ok(()));
}

#[test]
fn advantage_names_the_die_that_was_left_out_and_the_total_counts_only_the_kept_one() {
    let (view, _) = read(concat!(
        r#"scene roll saving_throw "Constitution" mode=advantage dice=20:6:dropped,20:17 "#,
        r#"mods=Constitution:4"#
    ))
    .expect("reads");
    assert_eq!(view.mode, RollMode::Advantage);
    assert!(!view.dice[0].kept && view.dice[1].kept);
    assert_eq!(view.total, 21);
    assert_eq!(view.check(), Ok(()));
}

#[test]
fn damage_dice_take_the_damage_role_and_a_bonus_die_names_its_own() {
    let (view, _) =
        read(r#"scene roll damage "Longsword" dice=8:5,4:3:bonus mods=Strength:3"#).expect("reads");
    assert_eq!(view.dice[0].role, DieRole::Damage);
    assert_eq!(view.dice[1].role, DieRole::Bonus);
    assert_eq!(view.total, 11);
}

#[test]
fn a_percentile_die_and_a_reroll_and_a_natural_mark_are_read() {
    let (view, _) =
        read(r#"scene roll table_roll "Wild surge" dice=100:57 natural=none reroll=0:12"#)
            .expect("reads");
    assert_eq!(view.dice[0].sides, 100);
    assert_eq!(
        view.inspiration_reroll,
        Some(InspirationReroll { index: 0, was: 12 })
    );
    let (view, _) =
        read(r#"scene roll melee_attack "Strike" dice=20:20 natural=twenty"#).expect("reads");
    assert_eq!(view.natural, NaturalMark::Twenty);
}

#[test]
fn a_total_given_that_is_not_the_sum_is_read_as_given_for_the_host_to_refuse() {
    let (view, _) = read(r#"scene roll damage "Fireball" dice=6:4 total=9"#).expect("reads");
    assert_eq!(view.total, 9);
    assert!(view.check().is_err());
}

#[test]
fn a_kind_a_die_a_mode_or_a_key_that_does_not_exist_is_refused_with_what_is_allowed() {
    for line in [
        r#"scene roll lockpick "Door""#,
        r#"scene roll damage "Fireball" dice=6"#,
        r#"scene roll damage "Fireball" dice=6:x"#,
        r#"scene roll damage "Fireball" dice=6:4:sparkly"#,
        r#"scene roll damage "Fireball" mode=lucky"#,
        r#"scene roll damage "Fireball" outcome=triumph"#,
        r#"scene roll damage "Fireball" mods=Strength"#,
        r#"scene roll damage "Fireball" sparkle=yes"#,
    ] {
        assert!(
            matches!(read_error(line), CommandError::BadChoice { .. }),
            "{line}"
        );
    }
}

fn read_error(line: &str) -> CommandError {
    read(line).expect_err("is refused")
}

#[test]
fn a_roll_without_a_kind_and_a_title_is_the_usage_and_a_key_given_twice_is_refused() {
    assert!(matches!(
        read_error("scene roll skill_check"),
        CommandError::Usage(_)
    ));
    assert!(matches!(
        read_error(r#"scene roll damage "Fireball" id=a id=b"#),
        CommandError::DuplicateKey(_)
    ));
}

#[test]
fn roll_clear_takes_the_card_away_and_takes_nothing_else() {
    let words = split_words("scene roll-clear").expect("splits");
    assert_eq!(
        parse(&words).expect("reads"),
        Command::Scene(SceneChange::ClearRoll {})
    );
    let words = split_words("scene roll-clear now").expect("splits");
    assert!(matches!(parse(&words), Err(CommandError::Usage(_))));
}

/// The gallery script rolls one of every kind and every die size, and each line is a roll the
/// page can draw.
#[test]
fn the_gallery_script_rolls_every_kind_and_every_die_size_with_values_that_add_up() {
    let script = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../development/playground-scripts/roll-gallery.txt"
    ))
    .expect("the gallery script is there");
    let mut kinds = std::collections::BTreeSet::new();
    let mut sides = std::collections::BTreeSet::new();
    let mut modes = std::collections::BTreeSet::new();
    for line in script
        .lines()
        .filter(|line| line.starts_with("scene roll "))
    {
        let (view, _) = read(line).unwrap_or_else(|error| panic!("{line}: {error}"));
        assert_eq!(view.check(), Ok(()), "{line}");
        kinds.insert(view.kind.key());
        modes.insert(view.mode.key());
        sides.extend(view.dice.iter().map(|die| die.sides));
    }
    assert_eq!(kinds.len(), RollKind::ALL.len());
    assert_eq!(sides, [4, 6, 8, 10, 12, 20, 100].into());
    assert_eq!(modes.len(), 3);
}
