//! The `scene` verbs of the narration box, the seat strip and the logo: parsed, applied by the host
//! and drawn into the overlay, field by field, and the shipped placement script run end to end.

use df_playground::fixture::Fixture;
use df_playground::overlay::{Edit, Glyph, NarrationPlacement, Progress};
use df_playground::scene::SceneChange;

use crate::client::get;
use crate::command::{Command, CommandError, parse, split_words, usage_text};
use crate::dev::{DevError, run_dev};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::{ControlsReply, EventsReply};

const EMBER_ROADS: &str =
    include_str!("../../../assets/playground/fixtures/scene_ember_roads.json");
const TOUR: &str = include_str!("../../../development/playground-scripts/ember-roads-tour.txt");

fn scene(line: &str) -> Result<SceneChange, CommandError> {
    let words = split_words(line).expect("the line splits");
    match parse(&words)? {
        Command::Scene(change) => Ok(change),
        other => panic!("expected a scene change, got {other:?}"),
    }
}

#[test]
fn a_narration_box_is_set_from_its_text_and_every_other_field_is_optional() {
    let Ok(SceneChange::Narration {
        text,
        speaker,
        badge,
        continue_label,
        voice,
        placement,
        progress,
    }) = scene("scene narration set \"Rain on the quay.\"")
    else {
        panic!("a narration");
    };
    assert_eq!(text.as_deref(), Some("Rain on the quay."));
    assert_eq!(
        (
            &speaker,
            &badge,
            &continue_label,
            voice,
            placement,
            &progress
        ),
        (
            &Edit::Keep,
            &Edit::Keep,
            &Edit::Keep,
            None,
            None,
            &Edit::Keep
        )
    );
}

#[test]
fn every_field_of_the_narration_box_has_a_key_and_none_takes_it_away() {
    let Ok(SceneChange::Narration {
        speaker,
        badge,
        continue_label,
        voice,
        placement,
        progress,
        ..
    }) = scene(
        "scene narration set x speaker=Garrick badge=dice continue=Next voice=false placement=right scene=2/5:Opening",
    )
    else {
        panic!("a narration");
    };
    assert_eq!(speaker, Edit::Set("Garrick".to_owned()));
    assert_eq!(badge, Edit::Set(Glyph::Dice));
    assert_eq!(continue_label, Edit::Set("Next".to_owned()));
    assert_eq!(voice, Some(false));
    assert_eq!(placement, Some(NarrationPlacement::Right));
    assert_eq!(
        progress,
        Edit::Set(Progress {
            index: 2,
            total: 5,
            title: Some("Opening".to_owned())
        })
    );
    let Ok(SceneChange::Narration {
        text,
        speaker,
        badge,
        continue_label,
        progress,
        ..
    }) = scene("scene narration edit speaker=none badge=none continue=none scene=none")
    else {
        panic!("a narration");
    };
    assert_eq!(text, None);
    assert_eq!(
        (speaker, badge, continue_label, progress),
        (Edit::Clear, Edit::Clear, Edit::Clear, Edit::Clear)
    );
    let Ok(SceneChange::Narration { text, .. }) = scene("scene narration edit text=\"New line\"")
    else {
        panic!("a narration");
    };
    assert_eq!(text.as_deref(), Some("New line"));
}

#[test]
fn narration_clear_and_the_logo_and_the_seat_counts_are_parsed() {
    assert_eq!(
        scene("scene narration clear"),
        Ok(SceneChange::ClearNarration {})
    );
    assert_eq!(scene("scene logo on"), Ok(SceneChange::Brand { on: true }));
    assert_eq!(
        scene("scene logo off"),
        Ok(SceneChange::Brand { on: false })
    );
    assert_eq!(
        scene("scene seats open 2"),
        Ok(SceneChange::SeatsOpen { open: 2 })
    );
    assert_eq!(scene("scene seats clear"), Ok(SceneChange::SeatsClear {}));
    assert_eq!(
        scene("scene seat remove Kael"),
        Ok(SceneChange::SeatRemove {
            name: "Kael".to_owned()
        })
    );
}

#[test]
fn every_field_of_a_seat_card_has_a_key() {
    let Ok(SceneChange::SeatSet {
        name,
        class,
        level,
        portrait,
        glyph,
        accent,
        hit_points,
        maximum_hit_points,
        conditions,
    }) = scene(
        "scene seat set Kael class=Fighter level=3 portrait=brakka.webp glyph=shield accent=#e0675a hp=22 max=28 conditions=Poisoned,Prone",
    )
    else {
        panic!("a seat");
    };
    assert_eq!(name, "Kael");
    assert_eq!(class, Edit::Set("Fighter".to_owned()));
    assert_eq!(level, Edit::Set(3));
    assert_eq!(portrait, Edit::Set("brakka.webp".to_owned()));
    assert_eq!(glyph, Edit::Set(Glyph::Shield));
    assert_eq!(accent, Edit::Set("#e0675a".to_owned()));
    assert_eq!((hit_points, maximum_hit_points), (Some(22), Some(28)));
    assert_eq!(
        conditions,
        Some(vec!["Poisoned".to_owned(), "Prone".to_owned()])
    );
    let Ok(SceneChange::SeatSet {
        class,
        level,
        glyph,
        ..
    }) = scene("scene seat set Kael class=none level=none glyph=none")
    else {
        panic!("a seat");
    };
    assert_eq!(
        (class, level, glyph),
        (Edit::Clear, Edit::Clear, Edit::Clear)
    );
}

#[test]
fn a_key_that_does_not_belong_a_glyph_that_is_not_drawn_or_a_bad_number_is_refused_with_the_choices()
 {
    for line in [
        "scene narration set x colour=red",
        "scene narration set x badge=trophy",
        "scene narration set x placement=left",
        "scene narration set x voice=maybe",
        "scene narration set x scene=two",
        "scene narration set x scene=2",
        "scene seat set Kael level=high",
        "scene seat set Kael glyph=axe",
        "scene seat set Kael speed=3",
        "scene seats open many",
        "scene seats open 300",
        "scene logo maybe",
    ] {
        assert!(
            matches!(
                scene(line),
                Err(CommandError::BadChoice { .. } | CommandError::BadCount { .. })
            ),
            "{line}: {:?}",
            scene(line)
        );
    }
    let Err(CommandError::BadChoice { allowed, .. }) = scene("scene narration set x badge=trophy")
    else {
        panic!("refused");
    };
    assert!(
        allowed.contains("sword") && allowed.contains("dice"),
        "{allowed}"
    );
    assert_eq!(
        scene("scene narration set x speaker=a speaker=b"),
        Err(CommandError::DuplicateKey("speaker".into()))
    );
    for line in [
        "scene narration",
        "scene narration edit",
        "scene narration set",
        "scene seat",
        "scene seat set",
        "scene seats",
        "scene logo",
    ] {
        assert!(matches!(scene(line), Err(CommandError::Usage(_))), "{line}");
    }
}

#[test]
fn the_help_describes_each_component_and_how_to_take_a_field_away() {
    let help = usage_text();
    for text in [
        "scene narration set",
        "scene seat set",
        "scene seats open",
        "scene logo on|off",
        "none takes a",
    ] {
        assert!(help.contains(text), "{text}");
    }
}

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

async fn shown(running: &Running) -> ControlsReply {
    let response = get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    serde_json::from_str(&response.body).expect("controls")
}

/// What the page draws: the fixture with the overlay laid on it.
async fn drawn(running: &Running) -> Fixture {
    let reply = shown(running).await;
    let mut fixture = Fixture::parse(EMBER_ROADS).expect("the shipped scene reads");
    reply.scene.apply_to(&mut fixture);
    fixture
}

async fn with_the_scene() -> Running {
    let running = start(&[STARTER_FILES[0], STARTER_FILES[1]]).await;
    running.write_asset("fixtures/scene_ember_roads.json", EMBER_ROADS);
    for portrait in [
        "brakka.webp",
        "sigrun.webp",
        "finn-alder.webp",
        "dain-rusk.webp",
    ] {
        running.write_asset(&format!("media/{portrait}"), "picture");
    }
    get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    running
}

#[tokio::test]
async fn each_verb_changes_the_overlay_and_is_one_audit_line_with_its_fields() {
    let running = with_the_scene().await;
    run(
        &running,
        "scene narration set \"Rain.\" speaker=Garrick badge=scales placement=right",
    )
    .await
    .expect("sets");
    let narration = drawn(&running).await.tv.narration.expect("a box");
    assert_eq!(narration.text, "Rain.");
    assert_eq!(narration.speaker.as_deref(), Some("Garrick"));
    assert_eq!(narration.badge, Some(Glyph::Scales));
    assert_eq!(narration.placement, NarrationPlacement::Right);

    run(&running, "scene narration edit speaker=none voice=false")
        .await
        .expect("edits one field");
    let narration = drawn(&running).await.tv.narration.expect("a box");
    assert_eq!(narration.speaker, None);
    assert!(!narration.voice);
    assert_eq!(narration.text, "Rain.", "the text stays");

    run(
        &running,
        "scene seat set Kael hp=7 max=40 class=Paladin glyph=sword",
    )
    .await
    .expect("a hero");
    run(&running, "scene seats open 3").await.expect("open");
    run(&running, "scene logo off").await.expect("logo");
    let scene = drawn(&running).await.tv;
    assert!(!scene.brand);
    let kael = scene
        .party
        .iter()
        .find(|member| member.name == "Kael")
        .expect("Kael");
    assert_eq!((kael.hit_points, kael.maximum_hit_points), (7, 40));
    let seats = scene.seats.expect("seats");
    assert_eq!(seats.open, 3);
    let look = seats
        .looks
        .iter()
        .find(|look| look.name == "Kael")
        .expect("a look");
    assert_eq!(look.class.as_deref(), Some("Paladin"));

    let events = get(running.port, "/events.json").await.expect("answers");
    let events: EventsReply = serde_json::from_str(&events.body).expect("events");
    let lines: Vec<String> = events.events.into_iter().map(|line| line.text).collect();
    let find = |verb: &str| {
        lines
            .iter()
            .find(|line| line.starts_with(&format!("PLAYGROUND {verb} ")))
            .unwrap_or_else(|| panic!("no audit line for {verb}: {lines:?}"))
            .clone()
    };
    let line = find("narration");
    assert!(
        line.contains("speaker=") && line.contains("badge=scales"),
        "{line}"
    );
    assert!(
        line.contains("placement=right") && line.contains("revision="),
        "{line}"
    );
    assert!(find("seat_set").contains("name=Kael"));
    assert!(find("seats_open").contains("open=3"));
    assert!(find("logo").contains("on=false"));
}

#[tokio::test]
async fn a_refused_change_leaves_the_overlay_as_it_was_and_says_why() {
    let running = with_the_scene().await;
    let before = drawn(&running).await;
    let Err(error) = run(&running, "scene seat set Nobody class=Rogue").await else {
        panic!("a hero not in the party needs hit points");
    };
    assert!(error.to_string().contains("hp=N"), "{error}");
    let Err(error) = run(&running, "scene narration set x scene=9/5").await else {
        panic!("a scene past the total");
    };
    assert!(error.to_string().contains("does not exist"), "{error}");
    let Err(error) = run(&running, "scene seat set Kael portrait=missing.webp").await else {
        panic!("a picture that is not in the library");
    };
    assert!(error.to_string().contains("missing.webp"), "{error}");
    assert_eq!(drawn(&running).await, before);
}

#[tokio::test]
async fn the_shipped_placement_script_assembles_the_scene_in_order() {
    let running = with_the_scene().await;
    // The pauses are real time in the shipped script; the check runs the same lines without them.
    let quick: String = TOUR
        .lines()
        .filter(|line| !line.starts_with("wait "))
        .map(|line| format!("{line}\n"))
        .collect();
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let path = directory.path().join("tour.txt");
    std::fs::write(&path, quick).expect("writes the script");
    let words = ["script".to_owned(), path.to_string_lossy().into_owned()];
    let said = run_dev(running.port, &words).await.expect("runs");
    assert!(said.ends_with("all ok"), "{said}");
    let scene = drawn(&running).await.tv;
    assert!(scene.brand, "the logo is on");
    let names: Vec<&str> = scene
        .party
        .iter()
        .map(|member| member.name.as_str())
        .collect();
    assert_eq!(names, ["Kael", "Lyra", "Seris", "Thoren"]);
    let kael = &scene.party[0];
    assert_eq!(kael.hit_points, 17, "the last change of the script");
    assert_eq!(scene.party[1].conditions, ["Poisoned"]);
    assert_eq!(scene.seats.expect("seats").open, 2);
    let narration = scene.narration.expect("a box");
    assert_eq!(narration.speaker.as_deref(), Some("The Dungeon Master"));
    assert_eq!(narration.badge, None);
    assert_eq!(
        narration
            .progress
            .map(|progress| (progress.index, progress.total)),
        Some((1, 5))
    );
    assert!(narration.text.starts_with("You want passage"));
}
