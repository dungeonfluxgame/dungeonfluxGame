//! The live changes to the narration box, the seat strip and the logo: what each one does to the
//! overlay and to the scene drawn from it, and what is refused.

use crate::fixture::Fixture;
use crate::overlay::{Edit, Glyph, NarrationPlacement, Progress};
use crate::scene::{Override, SceneChange, SceneError, SceneOverlay};

fn base() -> Fixture {
    Fixture::parse(
        r##"{"name":"B","tv":{"place":"P","status":{"kind":"ok"},
            "caption":{"text":"A caption"},
            "party":[{"name":"Kael","hit_points":10,"maximum_hit_points":12}],
            "seats":{"looks":[{"name":"Kael","class":"Fighter","level":3}],"open":1},
            "elements":[]},
            "phone":{"hero":"H","status":{"kind":"ok"}}}"##,
    )
    .expect("reads")
}

/// A narration change with every field left alone, to set the few a test is about.
#[derive(Default)]
struct N {
    text: Option<String>,
    speaker: Edit<String>,
    badge: Edit<Glyph>,
    continue_label: Edit<String>,
    voice: Option<bool>,
    placement: Option<NarrationPlacement>,
    progress: Edit<Progress>,
}

impl N {
    fn go(self) -> SceneChange {
        SceneChange::Narration {
            text: self.text,
            speaker: self.speaker,
            badge: self.badge,
            continue_label: self.continue_label,
            voice: self.voice,
            placement: self.placement,
            progress: self.progress,
        }
    }
}

fn text(text: &str) -> SceneChange {
    N {
        text: Some(text.to_owned()),
        ..N::default()
    }
    .go()
}

/// A seat change with every field left alone.
#[derive(Default)]
struct S {
    class: Edit<String>,
    level: Edit<u8>,
    portrait: Edit<String>,
    glyph: Edit<Glyph>,
    accent: Edit<String>,
    hit_points: Option<u32>,
    maximum_hit_points: Option<u32>,
    conditions: Option<Vec<String>>,
}

impl S {
    fn go(self, name: &str) -> SceneChange {
        SceneChange::SeatSet {
            name: name.to_owned(),
            class: self.class,
            level: self.level,
            portrait: self.portrait,
            glyph: self.glyph,
            accent: self.accent,
            hit_points: self.hit_points,
            maximum_hit_points: self.maximum_hit_points,
            conditions: self.conditions,
        }
    }
}

fn drawn(overlay: &SceneOverlay) -> Fixture {
    let mut fixture = base();
    overlay.apply_to(&mut fixture);
    fixture
}

#[test]
fn a_narration_box_is_made_from_its_text_alone_and_every_field_is_then_changed_one_at_a_time() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(text("Rain on the quay."), &base())
        .expect("made");
    let made = drawn(&overlay).tv.narration.expect("a box");
    assert_eq!(made.text, "Rain on the quay.");
    assert!(made.voice && made.speaker.is_none() && made.badge.is_none());

    let edits = [
        N {
            speaker: Edit::Set("Garrick".to_owned()),
            ..N::default()
        },
        N {
            badge: Edit::Set(Glyph::Dice),
            ..N::default()
        },
        N {
            continue_label: Edit::Set("Continue".to_owned()),
            ..N::default()
        },
        N {
            voice: Some(false),
            ..N::default()
        },
        N {
            placement: Some(NarrationPlacement::Right),
            ..N::default()
        },
        N {
            progress: Edit::Set(Progress {
                index: 1,
                total: 5,
                title: None,
            }),
            ..N::default()
        },
    ];
    for edit in edits {
        overlay.apply(edit.go(), &base()).expect("edited");
    }
    let edited = drawn(&overlay).tv.narration.expect("a box");
    assert_eq!(edited.text, "Rain on the quay.", "the text stays");
    assert_eq!(edited.speaker.as_deref(), Some("Garrick"));
    assert_eq!(edited.badge, Some(Glyph::Dice));
    assert_eq!(edited.continue_label.as_deref(), Some("Continue"));
    assert!(!edited.voice);
    assert_eq!(edited.placement, NarrationPlacement::Right);
    assert_eq!(edited.progress.map(|progress| progress.total), Some(5));

    let clear = N {
        speaker: Edit::Clear,
        badge: Edit::Clear,
        ..N::default()
    };
    overlay.apply(clear.go(), &base()).expect("cleared fields");
    let cleared = drawn(&overlay).tv.narration.expect("a box");
    assert_eq!((cleared.speaker, cleared.badge), (None, None));
}

#[test]
fn changing_a_narration_box_that_does_not_exist_needs_its_text() {
    let mut overlay = SceneOverlay::default();
    assert_eq!(
        overlay.apply(N::default().go(), &base()),
        Err(SceneError::NoNarrationYet)
    );
    assert!(overlay.is_empty(), "a refusal changes nothing");
}

#[test]
fn clearing_the_narration_takes_it_away_and_a_later_set_brings_it_back() {
    let mut overlay = SceneOverlay::default();
    overlay.apply(text("One."), &base()).expect("set");
    overlay
        .apply(SceneChange::ClearNarration {}, &base())
        .expect("cleared");
    assert_eq!(overlay.narration, Some(Override::Clear));
    assert!(drawn(&overlay).tv.narration.is_none());
    assert_eq!(
        overlay.apply(N::default().go(), &base()),
        Err(SceneError::NoNarrationYet),
        "after a clear there is nothing to change"
    );
    overlay.apply(text("Two."), &base()).expect("set");
    assert_eq!(drawn(&overlay).tv.narration.expect("a box").text, "Two.");
}

#[test]
fn a_narration_box_is_checked_with_the_rules_a_scene_file_is() {
    let mut overlay = SceneOverlay::default();
    let blank_speaker = N {
        text: Some("x".to_owned()),
        speaker: Edit::Set(String::new()),
        ..N::default()
    };
    let bad_scene = N {
        text: Some("x".to_owned()),
        progress: Edit::Set(Progress {
            index: 9,
            total: 5,
            title: None,
        }),
        ..N::default()
    };
    for bad in [text("  "), blank_speaker.go(), bad_scene.go()] {
        assert!(overlay.apply(bad, &base()).is_err());
    }
    assert!(overlay.is_empty());
}

#[test]
fn a_hero_who_is_in_the_party_gets_a_look_and_keeps_their_hit_points() {
    let mut overlay = SceneOverlay::default();
    let look = S {
        class: Edit::Set("Paladin".to_owned()),
        level: Edit::Set(4),
        glyph: Edit::Set(Glyph::Shield),
        accent: Edit::Set("#e0675a".to_owned()),
        portrait: Edit::Set("brakka.webp".to_owned()),
        ..S::default()
    };
    overlay.apply(look.go("Kael"), &base()).expect("set");
    let scene = drawn(&overlay).tv;
    let look = &scene.seats.as_ref().expect("seats").looks[0];
    assert_eq!(look.class.as_deref(), Some("Paladin"));
    assert_eq!(look.level, Some(4));
    assert_eq!(look.glyph, Some(Glyph::Shield));
    assert_eq!(look.portrait.as_deref(), Some("brakka.webp"));
    assert_eq!(look.accent.as_deref(), Some("#e0675a"));
    assert_eq!(
        scene.party[0].hit_points, 10,
        "hit points are the party's own"
    );
    assert_eq!(scene.seats.expect("seats").open, 1, "open seats stay");
}

#[test]
fn hit_points_conditions_and_the_maximum_of_a_hero_change_through_the_seat_verb() {
    let mut overlay = SceneOverlay::default();
    let hurt = S {
        hit_points: Some(4),
        maximum_hit_points: Some(20),
        conditions: Some(vec!["Poisoned".to_owned()]),
        ..S::default()
    };
    overlay.apply(hurt.go("Kael"), &base()).expect("set");
    let member = &drawn(&overlay).tv.party[0];
    assert_eq!((member.hit_points, member.maximum_hit_points), (4, 20));
    assert_eq!(member.conditions, ["Poisoned"]);
}

#[test]
fn a_new_hero_needs_hit_points_and_the_maximum_defaults_to_them() {
    let mut overlay = SceneOverlay::default();
    assert_eq!(
        overlay.apply(S::default().go("Lyra"), &base()),
        Err(SceneError::NeedsHitPoints("Lyra".to_owned()))
    );
    assert!(overlay.is_empty());
    let added = S {
        class: Edit::Set("Ranger".to_owned()),
        hit_points: Some(9),
        ..S::default()
    };
    overlay.apply(added.go("Lyra"), &base()).expect("added");
    let scene = drawn(&overlay).tv;
    assert_eq!(scene.party.len(), 2);
    assert_eq!(scene.party[1].name, "Lyra");
    assert_eq!(scene.party[1].maximum_hit_points, 9);
    assert_eq!(scene.seats.expect("seats").looks.len(), 2);
}

#[test]
fn a_look_field_is_cleared_with_clear_and_left_alone_with_keep() {
    let mut overlay = SceneOverlay::default();
    let clear = S {
        class: Edit::Clear,
        ..S::default()
    };
    overlay.apply(clear.go("Kael"), &base()).expect("set");
    let look = &drawn(&overlay).tv.seats.expect("seats").looks[0];
    assert_eq!(look.class, None);
    assert_eq!(look.level, Some(3), "kept");
}

#[test]
fn removing_a_hero_takes_the_look_and_the_party_place_and_an_unknown_hero_is_refused() {
    let mut overlay = SceneOverlay::default();
    let kael = SceneChange::SeatRemove {
        name: "Kael".to_owned(),
    };
    overlay.apply(kael, &base()).expect("removed");
    let scene = drawn(&overlay).tv;
    assert!(scene.party.is_empty());
    assert!(scene.seats.expect("seats").looks.is_empty());
    let ghost = SceneChange::SeatRemove {
        name: "Ghost".to_owned(),
    };
    assert_eq!(
        overlay.apply(ghost, &base()),
        Err(SceneError::UnknownMember("Ghost".to_owned()))
    );
}

#[test]
fn open_seats_are_set_and_cleared_without_touching_the_heroes() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(SceneChange::SeatsOpen { open: 3 }, &base())
        .expect("open");
    let scene = drawn(&overlay).tv;
    assert_eq!(scene.seats.as_ref().expect("seats").open, 3);
    assert_eq!(scene.seats.expect("seats").looks.len(), 1);
    overlay
        .apply(SceneChange::SeatsClear {}, &base())
        .expect("cleared");
    let cleared = drawn(&overlay).tv;
    assert_eq!(cleared.seats.expect("seats").open, 0);
    assert_eq!(cleared.party.len(), 1, "the party stays");
}

#[test]
fn the_logo_is_switched_on_and_off_and_a_reset_returns_the_fixture() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(SceneChange::Brand { on: true }, &base())
        .expect("on");
    assert!(drawn(&overlay).tv.brand);
    overlay
        .apply(SceneChange::Brand { on: false }, &base())
        .expect("off");
    assert!(!drawn(&overlay).tv.brand);
    overlay.apply(text("x"), &base()).expect("set");
    overlay
        .apply(SceneChange::SeatsOpen { open: 2 }, &base())
        .expect("open");
    overlay
        .apply(SceneChange::Reset {}, &base())
        .expect("reset");
    assert!(overlay.is_empty());
    assert_eq!(drawn(&overlay), base());
}

#[test]
fn the_changes_cross_the_wire_as_json_and_a_misspelt_field_is_refused() {
    let change: SceneChange = serde_json::from_str(
        r#"{"change":"narration","text":"Hello","speaker":{"set":"Garrick"},"badge":"clear","placement":"right"}"#,
    )
    .expect("reads");
    let SceneChange::Narration {
        text,
        speaker,
        badge,
        placement,
        voice,
        ..
    } = change
    else {
        panic!("a narration");
    };
    assert_eq!(text.as_deref(), Some("Hello"));
    assert_eq!(speaker, Edit::Set("Garrick".to_owned()));
    assert_eq!(badge, Edit::Clear);
    assert_eq!(placement, Some(NarrationPlacement::Right));
    assert_eq!(voice, None);
    assert!(serde_json::from_str::<SceneChange>(r#"{"change":"seats_open","opn":2}"#).is_err());
    assert!(serde_json::from_str::<SceneChange>(r#"{"change":"brand","on":true}"#).is_ok());
}

#[test]
fn a_clear_of_the_overlay_keeps_the_counts_and_drops_the_components() {
    let mut overlay = SceneOverlay::default();
    overlay.apply(text("x"), &base()).expect("set");
    overlay
        .apply(SceneChange::Brand { on: true }, &base())
        .expect("on");
    let cleared = overlay.cleared();
    assert!(cleared.narration.is_none() && cleared.seats.is_none() && cleared.brand.is_none());
}
