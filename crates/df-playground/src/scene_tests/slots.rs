//! Every slot of the scene overlay: each one draws onto its own field of the fixture, each one
//! clears, each one is reported under its JSON name, and the fields no live change touches stay as
//! the fixture has them. The destructures below have no `..`, so a new field of `SceneOverlay`,
//! `TvScene`, `PhoneScene` or `Fixture` stops this file compiling until it is classified here.

use crate::clues::{ClueBoard, ClueNote};
use crate::fixture::{
    Camera, Caption, Choice, ChoiceGroup, Fixture, ImageRef, PartyMember, PhoneScene, PlateKind,
    TvScene, UnavailableChoice,
};
use crate::home::model::HomeScene;
use crate::lobby::model::{LoadingScreen, LobbyScreen, RoleplayScene, TopBar};
use crate::overlay::{Edit, NarrationBox, NarrationPlacement, PartyLook, SeatStrip};
use crate::scene::{SceneChange, SceneOverlay, Target, Track};

fn image(media: &str) -> ImageRef {
    ImageRef {
        media: media.to_owned(),
        caption: None,
        rotation_degrees: 0,
        slide: None,
        layout: None,
        plate: PlateKind::Room,
    }
}

fn caption(text: &str) -> Caption {
    Caption {
        speaker: None,
        text: text.to_owned(),
    }
}

fn member(name: &str) -> PartyMember {
    PartyMember {
        name: name.to_owned(),
        hit_points: 3,
        maximum_hit_points: 5,
        conditions: Vec::new(),
        color: None,
    }
}

fn choice(label: &str) -> Choice {
    Choice {
        label: label.to_owned(),
        explain: None,
    }
}

fn group(name: &str) -> ChoiceGroup {
    ChoiceGroup {
        name: name.to_owned(),
        choices: vec![choice("Greet")],
    }
}

fn unavailable(label: &str) -> UnavailableChoice {
    UnavailableChoice {
        label: label.to_owned(),
        reason: "No slots".to_owned(),
    }
}

fn camera(target: &str) -> Camera {
    Camera {
        target: Some(target.to_owned()),
        zoom_percent: 150,
        duration_ms: 400,
        serial: 1,
        began_ms: 0,
        elapsed_ms: 0,
    }
}

fn narration(text: &str) -> NarrationBox {
    NarrationBox {
        speaker: None,
        text: text.to_owned(),
        continue_label: None,
        badge: None,
        voice: true,
        progress: None,
        placement: NarrationPlacement::Center,
    }
}

fn seats(name: &str, open: u8) -> SeatStrip {
    SeatStrip {
        looks: vec![PartyLook {
            name: name.to_owned(),
            class: None,
            level: None,
            portrait: None,
            glyph: None,
            accent: None,
        }],
        open,
    }
}

fn lobby(title: &str) -> LobbyScreen {
    LobbyScreen {
        eyebrow: None,
        title: title.to_owned(),
        subtitle: None,
        seats: Vec::new(),
        join: None,
        party: None,
        begin: None,
        focus: None,
    }
}

fn loading(title: &str) -> LoadingScreen {
    LoadingScreen {
        kicker: None,
        title: title.to_owned(),
        subtitle: None,
        narrator: None,
        label: None,
        percent: 40,
        seats: Vec::new(),
    }
}

fn roleplay(caption: &str) -> RoleplayScene {
    RoleplayScene {
        caption: Some(caption.to_owned()),
        ..RoleplayScene::default()
    }
}

fn pills(session: &str) -> TopBar {
    TopBar {
        session: Some(session.to_owned()),
        ..TopBar::default()
    }
}

fn home(focus: &str) -> HomeScene {
    HomeScene {
        focus: Some(focus.to_owned()),
        ..HomeScene::default()
    }
}

fn clue_board(id: &str) -> ClueBoard {
    ClueBoard {
        notes: vec![ClueNote {
            id: id.to_owned(),
            name: "A clue".to_owned(),
            detail: "What it shows.".to_owned(),
            worked_out: false,
            picture: None,
        }],
    }
}

fn track(media: &str) -> Track {
    Track {
        media: media.to_owned(),
        target: Target::Both,
        volume_percent: 50,
        looping: true,
    }
}

/// A fixture with every slot filled, so a set and a clear both draw something different.
fn base() -> Fixture {
    let mut fixture = Fixture::parse(
        r#"{"name":"Base","description":"Every slot","tv":{"place":"Here","status":{"kind":"ok"},
            "elements":[]},"phone":{"hero":"Pip","goal":"Find the key","status":{"kind":"ok"},
            "narration":["It rains."],"note":{"text":"Psst"}}}"#,
    )
    .expect("the fixture reads");
    let tv = &mut fixture.tv;
    tv.caption = Some(caption("Base line."));
    tv.party = vec![member("Pip")];
    tv.backdrop = Some(image("base-backdrop.webp"));
    tv.card = Some(image("base-card.webp"));
    tv.camera = Some(camera("base-door"));
    tv.narration = Some(narration("Base narration."));
    tv.seats = Some(seats("Pip", 1));
    tv.brand = true;
    tv.lobby = Some(lobby("Base hall"));
    tv.loading = Some(loading("Base wait"));
    tv.roleplay = Some(roleplay("Base inn"));
    tv.pills = Some(pills("Base session"));
    tv.home = Some(home("actions:1"));
    let phone = &mut fixture.phone;
    phone.suggested = vec![choice("Sit down")];
    phone.groups = vec![group("Talk")];
    phone.unavailable = vec![unavailable("Cast Light")];
    phone.image = Some(image("base-phone.webp"));
    fixture.music = Some(track("base.ogg"));
    fixture
}

/// An overlay that sets every slot to a value the base does not have.
fn all_set() -> SceneOverlay {
    SceneOverlay {
        caption: Edit::Set(caption("Live line.")),
        party: Edit::Set(vec![member("Vex")]),
        backdrop: Edit::Set(image("live-backdrop.webp")),
        card: Edit::Set(image("live-card.webp")),
        camera: Edit::Set(camera("live-door")),
        narration: Edit::Set(narration("Live narration.")),
        seats: Edit::Set(seats("Vex", 2)),
        brand: Edit::Set(false),
        lobby: Edit::Set(lobby("Live hall")),
        loading: Edit::Set(loading("Live wait")),
        roleplay: Edit::Set(roleplay("Live inn")),
        pills: Edit::Set(pills("Live session")),
        home: Edit::Set(home("nav:2")),
        suggested: Edit::Set(vec![choice("Run")]),
        groups: Edit::Set(vec![group("Fight")]),
        unavailable: Edit::Set(vec![unavailable("Fly")]),
        phone_image: Edit::Set(image("live-phone.webp")),
        music: Edit::Set(track("live.ogg")),
        clues: Edit::Set(clue_board("live-clue")),
        cue_count: 0,
        slide_count: 0,
        camera_count: 0,
        cues: Vec::new(),
    }
}

/// An overlay that clears every slot.
fn all_clear() -> SceneOverlay {
    SceneOverlay {
        caption: Edit::Clear,
        party: Edit::Clear,
        backdrop: Edit::Clear,
        card: Edit::Clear,
        camera: Edit::Clear,
        narration: Edit::Clear,
        seats: Edit::Clear,
        brand: Edit::Clear,
        lobby: Edit::Clear,
        loading: Edit::Clear,
        roleplay: Edit::Clear,
        pills: Edit::Clear,
        home: Edit::Clear,
        suggested: Edit::Clear,
        groups: Edit::Clear,
        unavailable: Edit::Clear,
        phone_image: Edit::Clear,
        music: Edit::Clear,
        clues: Edit::Clear,
        cue_count: 0,
        slide_count: 0,
        camera_count: 0,
        cues: Vec::new(),
    }
}

fn drawn(overlay: &SceneOverlay) -> Fixture {
    let mut fixture = base();
    overlay.apply_to(&mut fixture);
    fixture
}

/// The fields no live change touches, which must stay as the base has them.
fn assert_fixture_only_fields_kept(fixture: &Fixture) {
    let base = base();
    assert_eq!(fixture.name, base.name);
    assert_eq!(fixture.description, base.description);
    assert_eq!(fixture.tv.place, base.tv.place);
    assert_eq!(fixture.tv.status, base.tv.status);
    assert_eq!(fixture.tv.elements, base.tv.elements);
    assert_eq!(fixture.phone.hero, base.phone.hero);
    assert_eq!(fixture.phone.goal, base.phone.goal);
    assert_eq!(fixture.phone.status, base.phone.status);
    assert_eq!(fixture.phone.narration, base.phone.narration);
    assert_eq!(fixture.phone.note, base.phone.note);
}

#[test]
fn every_tv_field_is_patched_or_fixture_only() {
    let overlay = all_set();
    let fixture = drawn(&overlay);
    assert_fixture_only_fields_kept(&fixture);
    let Fixture {
        name: _,
        description: _,
        tv,
        phone,
        music,
    } = fixture;
    let TvScene {
        place: _,
        status: _,
        caption,
        backdrop,
        card,
        camera,
        party,
        narration,
        seats,
        brand,
        lobby,
        loading,
        roleplay,
        pills,
        home,
        clues,
        elements: _,
    } = tv;
    let PhoneScene {
        hero: _,
        goal: _,
        status: _,
        narration: _,
        suggested,
        groups,
        unavailable,
        note: _,
        image,
    } = phone;
    assert_eq!(caption.as_ref(), overlay.caption.set_value());
    assert_eq!(Some(&party), overlay.party.set_value());
    assert_eq!(backdrop.as_ref(), overlay.backdrop.set_value());
    assert_eq!(card.as_ref(), overlay.card.set_value());
    assert_eq!(camera.as_ref(), overlay.camera.set_value());
    assert_eq!(narration.as_ref(), overlay.narration.set_value());
    assert_eq!(seats.as_ref(), overlay.seats.set_value());
    assert_eq!(Some(&brand), overlay.brand.set_value());
    assert_eq!(lobby.as_ref(), overlay.lobby.set_value());
    assert_eq!(loading.as_ref(), overlay.loading.set_value());
    assert_eq!(roleplay.as_ref(), overlay.roleplay.set_value());
    assert_eq!(pills.as_ref(), overlay.pills.set_value());
    assert_eq!(home.as_ref(), overlay.home.set_value());
    assert_eq!(clues.as_ref(), overlay.clues.set_value());
    assert_eq!(Some(&suggested), overlay.suggested.set_value());
    assert_eq!(Some(&groups), overlay.groups.set_value());
    assert_eq!(Some(&unavailable), overlay.unavailable.set_value());
    assert_eq!(image.as_ref(), overlay.phone_image.set_value());
    assert_eq!(music.as_ref(), overlay.music.set_value());
}

#[test]
fn every_slot_clears() {
    let fixture = drawn(&all_clear());
    assert_fixture_only_fields_kept(&fixture);
    let Fixture {
        name: _,
        description: _,
        tv,
        phone,
        music,
    } = fixture;
    let TvScene {
        place: _,
        status: _,
        caption,
        backdrop,
        card,
        camera,
        party,
        narration,
        seats,
        brand,
        lobby,
        loading,
        roleplay,
        pills,
        home,
        clues,
        elements: _,
    } = tv;
    let PhoneScene {
        hero: _,
        goal: _,
        status: _,
        narration: _,
        suggested,
        groups,
        unavailable,
        note: _,
        image,
    } = phone;
    assert_eq!(caption, None);
    assert!(party.is_empty());
    assert_eq!(backdrop, None);
    assert_eq!(card, None);
    assert_eq!(camera, None);
    assert_eq!(narration, None);
    assert_eq!(seats, None);
    assert!(!brand);
    assert_eq!(lobby, None);
    assert_eq!(loading, None);
    assert_eq!(roleplay, None);
    assert_eq!(pills, None);
    assert_eq!(home, None);
    assert_eq!(clues, None);
    assert!(suggested.is_empty());
    assert!(groups.is_empty());
    assert!(unavailable.is_empty());
    assert_eq!(image, None);
    assert_eq!(music, None);
}

#[test]
fn changed_slots_match_wire_names() {
    let overlay = all_set();
    let written = serde_json::to_value(&overlay).expect("writes");
    let mut keys: Vec<&str> = written
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .filter(|key| !matches!(*key, "cue_count" | "slide_count" | "camera_count" | "cues"))
        .collect();
    let mut slots = overlay.changed_slots();
    assert_eq!(slots.len(), 19, "{slots:?}");
    slots.sort_unstable();
    keys.sort_unstable();
    assert_eq!(slots, keys);
    assert_eq!(all_clear().changed_slots().len(), 19);
    assert!(!overlay.is_empty() && !all_clear().is_empty());
}

#[test]
fn default_overlay_draws_base() {
    let overlay = SceneOverlay::default();
    assert!(overlay.is_empty());
    assert!(overlay.changed_slots().is_empty());
    assert_eq!(drawn(&overlay), base());
    let counted = SceneOverlay {
        cue_count: 3,
        camera_count: 2,
        ..SceneOverlay::default()
    };
    assert!(counted.is_empty(), "past cues and moves do not count");
}

#[test]
fn the_default_overlay_is_written_with_only_its_counts() {
    let written = serde_json::to_string(&SceneOverlay::default()).expect("writes");
    assert_eq!(
        written,
        r#"{"cue_count":0,"slide_count":0,"camera_count":0,"cues":[]}"#
    );
    let read: SceneOverlay = serde_json::from_str(&written).expect("reads");
    assert_eq!(read, SceneOverlay::default());
}

#[test]
fn a_party_clear_and_a_camera_move_are_written_as_clear_and_set() {
    let base = base();
    let mut overlay = SceneOverlay::default();
    overlay
        .apply_at(SceneChange::PartyClear {}, &base, 1000)
        .expect("clears");
    overlay
        .apply_at(
            SceneChange::Camera {
                target: Some("door".to_owned()),
                zoom_percent: 150,
                duration_ms: 400,
            },
            &base,
            1000,
        )
        .expect("moves");
    let written = serde_json::to_string(&overlay).expect("writes");
    assert_eq!(
        written,
        concat!(
            r#"{"party":"clear","camera":{"set":{"target":"door","zoom_percent":150,"#,
            r#""duration_ms":400,"serial":1,"began_ms":1000,"elapsed_ms":0}},"#,
            r#""cue_count":0,"slide_count":0,"camera_count":1,"cues":[]}"#
        )
    );
    let read: SceneOverlay = serde_json::from_str(&written).expect("reads");
    assert_eq!(read, overlay);
    assert_eq!(overlay.changed_slots(), ["party", "camera"]);
    assert_eq!(overlay.camera_target(), Some("door"));
}

#[test]
fn a_kept_slot_written_as_null_or_an_unknown_key_is_refused() {
    for old in [
        r#"{"caption":null}"#,
        r#"{"screens":{"lobby":null}}"#,
        r#"{"phone_picture":"clear"}"#,
    ] {
        assert!(serde_json::from_str::<SceneOverlay>(old).is_err(), "{old}");
    }
}
