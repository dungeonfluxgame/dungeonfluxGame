//! A Wizard built in creation casting through the host: a prepared spell spends a slot, a Ritual
//! of the spellbook spends none, refusals spend nothing, and each cast writes its `RULES` line.
use std::sync::Arc;

use df_rules::hero::{VitalsChange, plan_cast};
use df_table::creation::{CreationTable, HeroStart};
use df_table::identity::SeatId;
use df_table::machine::{CarryItem, Entry};

use super::cast::{CastError, cast_spell};
use crate::creation::rules::HeroRules;
use crate::creation::store;
use crate::creation::vitals::{apply_vitals, hero_vitals};
use crate::state::AppState;

const WIZARD: u64 = 21;

/// The picks that finish a Dwarf Wizard Sage begun from the look studio's species and class.
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
    ("background-equipment", "a"),
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

fn built_wizard() -> CreationTable {
    let seat = SeatId::new(1);
    let mut table =
        CreationTable::enter(&Entry::carrying(vec![CarryItem::Seat(seat)])).expect("enters");
    let hero = HeroStart {
        hero: WIZARD,
        name: "Odile Marsh".to_owned(),
        species: "dwarf".to_owned(),
        class: "wizard".to_owned(),
        portrait: None,
        public_story: Vec::new(),
    };
    table.begin(seat, hero, &HeroRules).expect("begins");
    for (slot, option) in WIZARD_PICKS {
        table
            .choose(seat, slot, option, &HeroRules)
            .expect("chooses");
    }
    table
}

async fn app_with_a_wizard() -> (tempfile::TempDir, Arc<AppState>) {
    use df_table::testkit::{MARKER_KIND, registry_from};
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let log = Arc::new(crate::event_log::EventLog::default());
    let app = Arc::new(AppState::new(
        df_table::canvas::Canvas::new(registry_from(&[MARKER_KIND])),
        crate::observe::HostEventSink::new(log, Arc::new(crate::observe::WallClock)),
        None,
        directory.path().to_path_buf(),
        crate::playground::PlaygroundFiles::new(
            directory.path().join("assets"),
            directory.path().join("pkg"),
        ),
    ));
    std::fs::create_dir_all(store::directory(&app)).expect("makes the players folder");
    store::change(&app, |_| Ok((built_wizard(), ())))
        .await
        .expect("keeps the record");
    (directory, app)
}

fn lines(app: &AppState) -> Vec<String> {
    app.log()
        .newest(64)
        .into_iter()
        .map(|line| line.text)
        .collect()
}

#[tokio::test]
async fn a_wizard_spends_a_slot_on_a_prepared_spell_and_none_on_a_ritual_of_the_book() {
    let (_directory, app) = app_with_a_wizard().await;
    let missile = cast_spell(&app, WIZARD, "magic-missile", 1, false)
        .await
        .expect("prepared, with a slot");
    assert_eq!(
        (missile.slot_spent, missile.slots_left, missile.ritual),
        (Some(1), 1, false)
    );
    let identify = cast_spell(&app, WIZARD, "identify", 1, true)
        .await
        .expect("a ritual of the spellbook");
    assert_eq!((identify.slot_spent, identify.slots_left), (None, 1));
    assert_eq!(
        identify.casting_time,
        "1 minute, plus 10 minutes as a Ritual"
    );
    let light = cast_spell(&app, WIZARD, "light", 0, false)
        .await
        .expect("a cantrip");
    assert_eq!(light.slots_left, 1);
    let logged = lines(&app);
    assert!(
        logged.iter().any(|line| line
            == "RULES cast hero=21 spell=magic-missile level=1 ritual=false slots_left=1"),
        "{logged:?}"
    );
    assert!(
        logged.iter().any(
            |line| line == "RULES cast hero=21 spell=identify level=1 ritual=true slots_left=1"
        ),
        "{logged:?}"
    );
}

#[tokio::test]
async fn a_refused_cast_spends_nothing_and_says_why() {
    let (_directory, app) = app_with_a_wizard().await;
    let refused = cast_spell(&app, WIZARD, "identify", 1, false)
        .await
        .expect_err("not prepared");
    assert_eq!(refused.key(), "not_prepared");
    for (spell, level, ritual, key) in [
        ("alarm", 1, true, "not_known"),
        ("fireball", 3, false, "unknown_spell"),
        ("magic-missile", 1, true, "not_a_ritual"),
        ("light", 1, false, "invalid_level"),
    ] {
        let error = cast_spell(&app, WIZARD, spell, level, ritual)
            .await
            .expect_err("refused");
        assert_eq!(error.key(), key, "{spell}");
    }
    let vitals = hero_vitals(&app, WIZARD)
        .await
        .expect("reads")
        .expect("built");
    assert_eq!(vitals.slots_left(1), 2);
    assert!(lines(&app).iter().any(|line| line
        == "RULES cast_refused hero=21 spell=identify level=1 ritual=false cause=not_prepared"));
    assert_eq!(
        cast_spell(&app, 99, "light", 0, false).await,
        Err(CastError::UnknownHero { hero: 99 })
    );
}

#[tokio::test]
async fn the_slots_run_out_and_a_long_rest_brings_them_back() {
    let (_directory, app) = app_with_a_wizard().await;
    for left in [1, 0] {
        let cast = cast_spell(&app, WIZARD, "sleep", 1, false)
            .await
            .expect("a slot is left");
        assert_eq!(cast.slots_left, left);
    }
    let out = cast_spell(&app, WIZARD, "shield", 1, false)
        .await
        .expect_err("no slot left");
    assert_eq!(out.key(), "no_slot");
    let recovered = apply_vitals(
        &app,
        WIZARD,
        VitalsChange::ArcaneRecovery { levels: vec![1] },
    )
    .await
    .expect("Arcane Recovery");
    assert_eq!(recovered.slots_left(1), 1);
    let rested = apply_vitals(&app, WIZARD, VitalsChange::LongRest)
        .await
        .expect("rests");
    assert_eq!(rested.slots_left(1), 2);
    assert!(
        lines(&app)
            .iter()
            .any(|line| line.contains("slots_1=2/2 change=long_rest"))
    );
}

#[test]
fn the_built_wizard_plans_its_casts_from_the_sheet_the_host_derives() {
    let table = built_wizard();
    let creation = table.seat(SeatId::new(1)).expect("in creation");
    let sheet = HeroRules::sheet(creation.hero.as_ref().expect("begun"), &creation.draft)
        .expect("a legal build");
    assert!(plan_cast(&sheet, "comprehend-languages", 1, true).is_ok());
}

#[test]
fn the_phone_is_shown_the_spells_the_slots_left_and_each_option_detail() {
    use crate::creation::view::{creation_view, sheet_view, with_vitals};
    use df_rules::hero::HeroVitals;
    let table = built_wizard();
    let creation = table.seat(SeatId::new(1)).expect("in creation");
    let sheet = HeroRules::sheet(creation.hero.as_ref().expect("begun"), &creation.draft)
        .expect("a legal build");
    let spent = HeroVitals::fresh(&sheet)
        .apply(&VitalsChange::SpendSpellSlot { level: 1 })
        .expect("a slot");
    let view = with_vitals(sheet_view(&sheet), Some(&spent));
    let casting = view.spellcasting.expect("a Wizard casts");
    assert_eq!(
        (
            casting.ability.as_str(),
            casting.save_dc,
            casting.attack_bonus
        ),
        ("Intelligence", 13, 5)
    );
    assert_eq!((casting.slots[0].left, casting.slots[0].maximum), (1, 2));
    let identify = casting
        .spells
        .iter()
        .find(|spell| spell.id == "identify")
        .expect("in the spellbook");
    assert!(identify.ritual && !identify.prepared);
    assert!(
        view.features
            .iter()
            .any(|feature| feature.name == "Arcane Recovery"
                && feature.status.as_deref() == Some("in_play"))
    );
    let shown = creation_view(Some(creation));
    let detail = |group: &str, option: &str| {
        shown
            .groups
            .iter()
            .find(|candidate| candidate.id == group)
            .and_then(|found| {
                found
                    .options
                    .iter()
                    .find(|candidate| candidate.id == option)
            })
            .and_then(|found| found.detail.clone())
    };
    let alarm = detail("spellbook", "alarm").expect("a spell has a detail");
    assert!(
        alarm.starts_with("Level 1 · Abjuration · ritual · "),
        "{alarm}"
    );
    let light = detail("cantrips", "light").expect("a cantrip has a detail");
    assert!(light.starts_with("Cantrip · Evocation · "), "{light}");
    assert_eq!(detail("alignment", "neutral-good"), None);
}
