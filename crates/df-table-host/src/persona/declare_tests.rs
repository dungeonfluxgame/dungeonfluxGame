//! A declared action and its emergency plate: the words and the closed `declared_action` checked
//! against what the character holds, the watch input, the picture's name-free words, and the
//! screen's rules (an emergency plate jumps the queue, supersedes the room plates started before
//! it, is replaced by a newer declaration, is shown at once and holds the screen).
//!
//! The screen keeps the host's own clock (`std::time::Instant`), so a slot is a short real one.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::cli::{PersonaCommand, parse};
use super::converse::Speaker;
use super::declare::{
    DECLARE_INSTRUCTION, DeclaredAction, NO_DUPLICATES, WatchDeclared, check_declared,
    declared_scene, unheld, watch_input,
};
use super::fixtures::{CHARACTER, ScriptedHost, innkeeper, line_answer, scripted_host};
use super::frame::{FrameAsk, FrameConfig, FrameReason};
use super::frame_watch::{WatchInput, room_prompt};
use super::plate::{
    RoomTurn, SharedScreen, begin_declaration, claim_declared, claim_room, claim_screen, not_shown,
    superseded_by,
};
use super::request::{PersonaReply, Spoken};
use super::routes::path as persona_path;
use super::storage::write_persona;
use crate::dev_client::post_json;
use crate::dev_http::DevReply;
use crate::movement::brief::BriefStyle;
use crate::world::{InventoryItem, ItemUse};

fn held(id: &str, count: u64) -> InventoryItem {
    InventoryItem {
        id: id.to_owned(),
        name: format!("{id} name"),
        look: format!("{id} look"),
        usage: ItemUse::Kept,
        count,
    }
}

fn action(doing: &str, items: &[&str]) -> DeclaredAction {
    DeclaredAction {
        doing: doing.to_owned(),
        pose: Some("bent over the table".to_owned()),
        items_in_use: items.iter().map(|item| (*item).to_owned()).collect(),
    }
}

#[test]
fn a_declaration_uses_only_what_the_character_holds_now() {
    let inventory = [held("easel", 1), held("ochre", 3), held("rags", 0)];
    assert_eq!(
        check_declared(&action("sets up", &["easel", "ochre"]), &inventory),
        Ok(())
    );
    assert_eq!(check_declared(&action("sets up", &[]), &inventory), Ok(()));
    let missing = check_declared(&action("lays in the door", &["brush", "ochre"]), &inventory)
        .expect_err("a brush is not held");
    assert!(
        missing.contains("brush") && !missing.contains("uses brush, ochre"),
        "{missing}"
    );
    assert!(
        missing.contains("they hold: easel, ochre"),
        "the reason lists what is held, not the items at zero: {missing}"
    );
    assert!(
        check_declared(&action("wipes the brush", &["rags"]), &inventory).is_err(),
        "an item at zero is not held"
    );
    assert!(check_declared(&action("  ", &["easel"]), &inventory).is_err());
    assert_eq!(
        unheld(
            &["brush".to_owned(), "easel".to_owned(), "brush".to_owned()],
            &inventory
        ),
        ["brush"],
        "each missing item once"
    );
    let nothing = check_declared(&action("paints", &["brush"]), &[]).expect_err("holds nothing");
    assert!(nothing.contains("they hold: nothing"), "{nothing}");
}

#[test]
fn the_picture_is_told_the_look_and_the_action_and_never_a_name() {
    let speaker = Speaker {
        id: "danny".to_owned(),
        display_name: "Danny".to_owned(),
        job: "A lean man in a straw hat and a paint-flecked smock".to_owned(),
    };
    let scene = declared_scene(
        &speaker.job,
        &action(
            "sets a board on an easel and lays in a door with ochre",
            &["easel"],
        ),
        &["folding easel".to_owned(), "primed boards".to_owned()],
        1,
    );
    assert_eq!(
        scene,
        concat!(
            "A person (A lean man in a straw hat and a paint-flecked smock) sets a board on an ",
            "easel and lays in a door with ochre, bent over the table, with their folding easel ",
            "and primed boards in view. The cast in view is one person, the one described, who ",
            "appears once and only once. Nobody is added and no one is copied: anyone else ",
            "already in the picture, such as small distant figures, stays as small, as far away ",
            "and as unlike this person as before. Everything else in the scene is as it was."
        )
    );
    let config = FrameConfig::default();
    let prompt = room_prompt(
        &BriefStyle::default(),
        Some(&config.edit_rule),
        &config.show.declare_framing,
        &scene,
    );
    let lower = prompt.to_lowercase();
    assert!(!lower.contains(&speaker.id) && !lower.contains(&speaker.display_name.to_lowercase()));
    assert!(prompt.contains("lays in a door with ochre") && prompt.contains("medium-wide"));
    let bare = declared_scene(
        "a painter",
        &DeclaredAction {
            doing: "sketches with charcoal.".to_owned(),
            pose: None,
            items_in_use: Vec::new(),
        },
        &[],
        3,
    );
    assert!(
        bare.starts_with(
            "A person (a painter) sketches with charcoal. The cast in view is 3 people;"
        ) && bare.contains("one of them and appears once and only once"),
        "{bare}"
    );
}

#[test]
fn a_declared_plate_bans_duplicates_in_neutral_words_beside_the_house_look() {
    let style = BriefStyle::default();
    let mut avoid = style.avoid.clone();
    avoid.extend(NO_DUPLICATES.map(str::to_owned));
    assert_eq!(&avoid[..style.avoid.len()], style.avoid.as_slice());
    let ban = avoid[style.avoid.len()..].join(", ");
    assert_eq!(ban, "the same person twice, clones, extra people");
}

#[test]
fn the_watch_is_handed_a_declaration_as_one_line() {
    let declared = action("kneads dough", &["flour"]);
    let line = watch_input("baker", "I knead the dough, flour to my elbows.", &declared);
    assert!(line.starts_with("@declare baker | {"), "{line}");
    assert!(!line.contains('\n'));
    let WatchInput::Declare { speaker, body } = WatchInput::parse(&line) else {
        panic!("a declaration: {line}");
    };
    assert_eq!(speaker, "baker");
    let back: WatchDeclared = serde_json::from_str(body).unwrap();
    assert_eq!(back.action, declared);
    assert_eq!(back.line, "I knead the dough, flour to my elbows.");
    for odd in [
        "@declare ",
        "@declare baker",
        "@declare baker | ",
        "@declare  | {}",
    ] {
        assert_eq!(WatchInput::parse(odd), WatchInput::Unreadable, "{odd}");
    }
}

#[test]
fn a_declaration_is_asked_without_a_model_and_makes_the_declared_reason() {
    let ask = FrameAsk::Declare {
        scene: "A person paints.".to_owned(),
    };
    assert_eq!(ask.reason(), FrameReason::Declared);
    assert_eq!(FrameReason::Declared.key(), "declared");
    assert!(
        !FrameReason::Declared.is_insert(),
        "a declared plate is a plate of the room"
    );
    let body = json!({"reason": "declare", "scene": "A person paints."});
    assert_eq!(serde_json::from_value::<FrameAsk>(body).unwrap(), ask);
    let show = FrameConfig::default().show;
    assert_eq!(
        (show.declare_hold_seconds, show.minimum_shown_seconds),
        (20, 8)
    );
}

#[test]
fn persona_declare_reads_the_words_the_options_and_the_situation() {
    let words: Vec<String> = [
        "declare",
        "danny",
        "speak=true",
        "recent=Danny: Hm.",
        "uses=easel,boards",
        "You set up to paint.",
    ]
    .map(str::to_owned)
    .to_vec();
    let PersonaCommand::Declare(command) = parse(&words).unwrap() else {
        panic!("a declare command");
    };
    let request = command.request;
    assert_eq!(request.character, "danny");
    assert!(request.speak && request.text.is_none());
    assert_eq!(request.uses, ["easel", "boards"]);
    assert_eq!(request.recent, ["Danny: Hm."]);
    assert_eq!(request.situation, "You set up to paint.");
    let verbatim: Vec<String> = [
        "declare",
        "danny",
        "text=I set the board on my easel.",
        "doing=sets a board on an easel",
    ]
    .map(str::to_owned)
    .to_vec();
    let PersonaCommand::Declare(command) = parse(&verbatim).unwrap() else {
        panic!("a declare command");
    };
    assert_eq!(
        command.request.text.as_deref(),
        Some("I set the board on my easel.")
    );
    assert_eq!(
        command.request.doing.as_deref(),
        Some("sets a board on an easel")
    );
    for odd in [
        &["declare"][..],
        &["declare", "danny"],
        &["declare", "danny", "uses=a,,b", "x"],
    ] {
        let odd: Vec<String> = odd.iter().map(|word| (*word).to_owned()).collect();
        assert!(parse(&odd).is_err(), "{odd:?}");
    }
}

// The screen.

const SLOT: Duration = Duration::from_millis(300);

/// A screen whose room plates come a slot apart, with `first` already up.
async fn screen_with(first: u32) -> SharedScreen {
    let screen = SharedScreen::default();
    screen.lock().unwrap().room_every = SLOT;
    assert!(claim_room(&screen, first, false).await);
    screen
}

#[tokio::test]
async fn an_emergency_plate_is_shown_at_once_ahead_of_the_plates_waiting_for_their_slots() {
    let screen = screen_with(1).await;
    // Two plates are ready and waiting for their slot; the queue is full.
    let waiting: Vec<_> = [2, 3]
        .map(|number| {
            let screen = screen.clone();
            tokio::spawn(async move { claim_room(&screen, number, false).await })
        })
        .into();
    tokio::time::sleep(SLOT / 6).await;
    let declared = 4;
    assert_eq!(begin_declaration(&screen, declared, "talk-4"), 2);
    let started = Instant::now();
    assert!(claim_declared(&screen, declared, Duration::from_millis(400)).await);
    assert!(
        started.elapsed() < SLOT / 2,
        "no wait for the next showing slot: {:?}",
        started.elapsed()
    );
    for plate in waiting {
        assert!(
            !plate.await.unwrap(),
            "a plate started before the declaration is superseded, not shown"
        );
    }
    assert_eq!(superseded_by(&screen, 2).as_deref(), Some("talk-4"));
    assert_eq!(
        superseded_by(&screen, 5),
        None,
        "a plate started after it is not"
    );
}

#[tokio::test]
async fn a_plate_started_before_the_declaration_that_is_still_being_made_is_dropped_when_made() {
    let screen = screen_with(1).await;
    begin_declaration(&screen, 3, "talk-3");
    // Plate 2 finishes only now, after the declaration: its claim is refused at once.
    let started = Instant::now();
    assert!(!claim_room(&screen, 2, false).await);
    assert!(started.elapsed() < SLOT / 2);
    let turn = RoomTurn {
        number: 2,
        screen: screen.clone(),
        cast_from: None,
        cued: false,
        emergency: None,
    };
    let shown = not_shown(&turn, "talk-2", true, None);
    assert!(!shown.on_screen);
    assert_eq!(
        shown.summary,
        "plate talk-2: made, not shown, superseded by talk-3"
    );
    // A plate started after the declaration follows the normal rules.
    let later = screen.clone();
    assert!(claim_declared(&later, 3, Duration::from_millis(100)).await);
    let waited = Instant::now();
    assert!(claim_room(&screen, 4, false).await);
    assert!(
        waited.elapsed() >= SLOT / 2,
        "its slot counts from the emergency plate going up: {:?}",
        waited.elapsed()
    );
}

#[tokio::test]
async fn a_newer_declaration_replaces_the_one_waiting_and_only_one_is_shown() {
    let screen = screen_with(1).await;
    // The picture before them has been up too briefly: the first declaration waits for the least
    // time a picture stays up.
    screen.lock().unwrap().minimum_shown = SLOT;
    let first = {
        let screen = screen.clone();
        tokio::spawn(async move { claim_declared(&screen, 2, Duration::from_millis(100)).await })
    };
    tokio::time::sleep(SLOT / 6).await;
    begin_declaration(&screen, 3, "talk-3");
    assert!(claim_declared(&screen, 3, Duration::from_millis(100)).await);
    assert!(!first.await.unwrap(), "the older declaration gave way");
}

#[tokio::test]
async fn an_emergency_plate_waits_for_no_hold_and_no_slot_but_holds_the_screen_itself() {
    let screen = SharedScreen::default();
    screen.lock().unwrap().room_every = Duration::from_secs(30);
    // A room plate went up a moment ago, and a close look holds the screen for a long while.
    assert!(claim_room(&screen, 1, false).await);
    claim_screen(&screen, true, Some(Duration::from_secs(30))).await;
    let started = Instant::now();
    assert!(claim_declared(&screen, 2, Duration::from_millis(300)).await);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "neither the insert's hold nor the 30 s slot is waited for: {:?}",
        started.elapsed()
    );
    // It holds the screen like an insert: the next room plate waits out its hold.
    screen.lock().unwrap().room_every = Duration::ZERO;
    let waited = Instant::now();
    assert!(claim_room(&screen, 3, false).await);
    assert!(
        waited.elapsed() >= Duration::from_millis(200),
        "held for the declare hold: {:?}",
        waited.elapsed()
    );
}

#[tokio::test]
async fn an_emergency_plate_still_waits_the_least_time_the_picture_before_it_stays_up() {
    let screen = SharedScreen::default();
    screen.lock().unwrap().minimum_shown = Duration::from_millis(250);
    claim_screen(&screen, true, None).await;
    let started = Instant::now();
    assert!(claim_declared(&screen, 1, Duration::from_millis(50)).await);
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "nothing changes faster than a person can follow: {:?}",
        started.elapsed()
    );
}

// The route.

async fn declare_with(host: &ScriptedHost, body: Value) -> PersonaReply<Spoken> {
    post_json(host.port, "/dev/persona/declare", &body)
        .await
        .unwrap()
}

fn baker_hands(host: &ScriptedHost) {
    write_persona(&host.directory(), &innkeeper()).unwrap();
    host.running.write_asset(
        "world/inventory/innkeeper.json",
        &json!({"schema": 1, "owner": CHARACTER, "kind": "character", "items": [
            {"id": "ladle", "name": "iron ladle", "look": "A dented ladle.", "use": "kept",
             "count": 1},
            {"id": "flour", "name": "flour", "look": "A sack of flour.", "use": "consumed",
             "count": 3},
            {"id": "apron", "name": "apron", "look": "A stained apron.", "use": "kept", "count": 0}
        ]})
        .to_string(),
    );
}

fn declared_answer(line: &str, doing: &str, items: &[&str]) -> Value {
    let mut answer = line_answer(line);
    answer["declared_action"] = json!({
        "doing": doing, "pose": "bent over the table", "items_in_use": items
    });
    answer
}

#[tokio::test]
async fn a_declaration_is_worded_by_the_line_call_against_the_closed_declared_action() {
    let host = scripted_host(vec![declared_answer(
        "I knead the dough on the table, flour to my elbows.",
        "kneads dough on a table with both floured hands",
        &["flour", "flour"],
    )])
    .await;
    baker_hands(&host);
    let spoken = match declare_with(
        &host,
        json!({"character": CHARACTER, "situation": "The bread has to be ready by dawn."}),
    )
    .await
    {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    };
    assert_eq!(
        spoken.said.line,
        "I knead the dough on the table, flour to my elbows."
    );
    let declared = spoken.said.declared.expect("the action is reported");
    assert_eq!(
        declared.items_in_use,
        ["flour"],
        "a repeated item is kept once"
    );
    assert_eq!(host.calls(), 1);
    let facts = host.facts_of(0);
    assert_eq!(facts["now"]["declare"], json!(true));
    assert!(
        facts["now"]["intent"]
            .as_str()
            .is_some_and(|intent| intent.contains("first person")),
        "the intent asks for the first person and the present tense"
    );
    assert!(host.instructions_of(0).contains(DECLARE_INSTRUCTION));
    let schema = host.asked.lock().unwrap()[0].output_schema.clone();
    assert!(schema["properties"]["declared_action"].is_object());
    assert!(
        schema["required"]
            .as_array()
            .is_some_and(|required| required.contains(&json!("declared_action")))
    );
    assert!(
        host.persona_lines()
            .iter()
            .any(|line| line
                == "PERSONA declaration id=innkeeper verbatim=false items=1 retried=false"),
        "{:?}",
        host.persona_lines()
    );
}

#[tokio::test]
async fn an_ordinary_line_is_asked_without_the_declaration() {
    let host = scripted_host(vec![line_answer("Bread's warm.")]).await;
    baker_hands(&host);
    let said: PersonaReply<Spoken> = post_json(
        host.port,
        persona_path::PERSONA_SAY,
        &json!({"character": CHARACTER, "situation": "Morning."}),
    )
    .await
    .unwrap();
    assert!(matches!(said, DevReply::Done { .. }));
    assert!(host.facts_of(0)["now"].get("declare").is_none());
    let schema = host.asked.lock().unwrap()[0].output_schema.clone();
    assert!(schema["properties"].get("declared_action").is_none());
    assert!(!host.instructions_of(0).contains(DECLARE_INSTRUCTION));
}

#[tokio::test]
async fn a_declaration_of_an_item_not_held_is_worded_again_once_with_why() {
    let host = scripted_host(vec![
        declared_answer(
            "I stir the stock with my big spoon.",
            "stirs stock with a spoon",
            &["spoon"],
        ),
        declared_answer(
            "I stir the stock with my ladle.",
            "stirs stock with a ladle",
            &["ladle"],
        ),
    ])
    .await;
    baker_hands(&host);
    let spoken = match declare_with(
        &host,
        json!({"character": CHARACTER, "situation": "The stock is on."}),
    )
    .await
    {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    };
    assert_eq!(host.calls(), 2, "asked once more");
    let reason = host.facts_of(1)["now"]["retry_reason"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        reason.contains("spoon") && reason.contains("they hold: ladle, flour"),
        "{reason}"
    );
    assert_eq!(spoken.said.line, "I stir the stock with my ladle.");
    assert_eq!(spoken.said.declared.unwrap().items_in_use, ["ladle"]);
    assert!(spoken.said.repeat_retried);
}

#[tokio::test]
async fn a_declaration_still_naming_an_item_not_held_after_the_retry_is_refused() {
    let host = scripted_host(vec![
        declared_answer("I stir with my spoon.", "stirs with a spoon", &["spoon"]),
        declared_answer("I stir with my spoon.", "stirs with a spoon", &["spoon"]),
    ])
    .await;
    baker_hands(&host);
    let reply = declare_with(
        &host,
        json!({"character": CHARACTER, "situation": "The stock is on."}),
    )
    .await;
    let DevReply::Rejected { message, .. } = reply else {
        panic!("a declaration that does not match the facts is refused");
    };
    assert!(
        message.contains("does not match the facts") && message.contains("spoon"),
        "{message}"
    );
    assert_eq!(host.calls(), 2);
    assert!(
        host.persona_lines()
            .iter()
            .any(|line| line == "PERSONA refused verb=declare id=innkeeper error=answer_refused")
    );
}

#[tokio::test]
async fn a_declaration_without_the_action_is_worded_again_and_then_refused() {
    let host = scripted_host(vec![line_answer("I work."), line_answer("I work.")]).await;
    baker_hands(&host);
    let reply = declare_with(
        &host,
        json!({"character": CHARACTER, "situation": "A long night."}),
    )
    .await;
    assert!(matches!(reply, DevReply::Rejected { .. }));
    assert_eq!(host.calls(), 2);
    let reason = host.facts_of(1)["now"]["retry_reason"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(reason.contains("declared_action is null"), "{reason}");
}

#[tokio::test]
async fn text_is_spoken_as_written_with_no_model_and_its_items_are_still_checked() {
    let host = scripted_host(Vec::new()).await;
    baker_hands(&host);
    let spoken = match declare_with(
        &host,
        json!({"character": CHARACTER, "text": "I lift the ladle and skim the pot.",
               "doing": "lifts an iron ladle and skims a pot", "uses": ["ladle"]}),
    )
    .await
    {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    };
    assert_eq!(spoken.said.line, "I lift the ladle and skim the pot.");
    let declared = spoken.said.declared.unwrap();
    assert_eq!(declared.doing, "lifts an iron ladle and skims a pot");
    assert_eq!(declared.items_in_use, ["ladle"]);
    assert!(
        spoken.said.inventory.is_empty(),
        "nothing read the words, so nothing moved"
    );
    assert_eq!(host.calls(), 0, "no model is asked");
    let plain = match declare_with(
        &host,
        json!({"character": CHARACTER, "text": "I sweep the floor."}),
    )
    .await
    {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    };
    assert_eq!(
        plain.said.declared.unwrap().doing,
        "I sweep the floor.",
        "the picture's words are the text itself when none are given"
    );
    let unheld = declare_with(
        &host,
        json!({"character": CHARACTER, "text": "I wipe the table.", "uses": ["rag"]}),
    )
    .await;
    assert!(matches!(unheld, DevReply::Rejected { .. }));
    let empty = declare_with(&host, json!({"character": CHARACTER})).await;
    assert!(
        matches!(empty, DevReply::Rejected { .. }),
        "words or a situation are needed"
    );
    assert_eq!(host.calls(), 0);
}
