//! The dungeon master: its answer read, what it is told kept apart from what is hidden, and its box
//! up as long as its newest line needs.

use df_content::persona::backfill::NarrativeGuidance;
use serde_json::{Value, json};

use super::routes::rest_of_narration;
use super::story::{
    Story, StoryBeat, StoryCharacter, StoryEvidence, StoryItem, StoryNarration, StoryScene,
};
use super::story_clues::{Finding, Findings};
use super::story_narrate::{Moment, NarrateRequest, known_and_hidden, read_narration};
use super::story_way::{PlayPhase, StoryWay, WayStage};

#[test]
fn the_narrator_speaks_a_line_keeps_silent_or_is_refused_a_word_with_no_line() {
    assert_eq!(
        read_narration(&json!({"speak": false, "line": null})).unwrap(),
        None
    );
    assert_eq!(
        read_narration(&json!({"speak": false, "line": "Unasked."})).unwrap(),
        None,
        "a line it chose not to speak is not spoken"
    );
    assert_eq!(
        read_narration(&json!({"speak": true, "line": "  You stand in the hall.  "})).unwrap(),
        Some("You stand in the hall.".to_owned())
    );
    for empty in [json!(null), json!("   ")] {
        assert!(
            read_narration(&json!({"speak": true, "line": empty})).is_err(),
            "{empty}"
        );
    }
    assert!(read_narration(&json!({"line": "No flag."})).is_err());
}

#[test]
fn every_moment_reads_back_from_its_key_and_its_serde_name() {
    for moment in Moment::ALL {
        assert_eq!(Moment::from_key(moment.key()), Some(moment));
        assert_eq!(serde_json::to_value(moment).unwrap(), json!(moment.key()));
    }
    assert_eq!(Moment::from_key("ending"), None);
}

fn piece(id: &str) -> StoryEvidence {
    StoryEvidence {
        id: id.to_owned(),
        what: format!("the {id}"),
        planted_by: "rival".to_owned(),
        flaw: format!("what the {id} gives away"),
        up_close: format!("how the {id} looks"),
        needs: Vec::new(),
    }
}

/// A vault with a record and a seal on show, a ledger elsewhere, and a keeper with a secret.
fn story() -> Story {
    let scene = |id: &str| StoryScene {
        id: id.to_owned(),
        place: format!("the {id}"),
        dressing: vec![format!("a lamp in the {id}")],
        danger: format!("what lurks in the {id}"),
        music: String::new(),
    };
    let beat = |id: &str, exhibits: &[&str]| StoryBeat {
        id: format!("beat_{id}"),
        scene: id.to_owned(),
        happens: String::new(),
        turns_on: String::new(),
        leads: String::new(),
        moves: Vec::new(),
        exhibits: exhibits.iter().map(|id| (*id).to_owned()).collect(),
    };
    Story {
        title: "The Seal".to_owned(),
        hook: "the hook in the open".to_owned(),
        truth: "the truth of it".to_owned(),
        culprit: "rival".to_owned(),
        motive: "the motive of it".to_owned(),
        scenes: vec![scene("vault"), scene("library")],
        characters: vec![StoryCharacter {
            id: "keeper".to_owned(),
            name: "Oda Venn".to_owned(),
            new: false,
            part: String::new(),
            role: String::new(),
            guidance: NarrativeGuidance::default(),
            secrets: vec![
                serde_json::from_value(json!({"id": "debt", "text": "the keeper's secret"}))
                    .unwrap(),
            ],
        }],
        evidence: vec![piece("record"), piece("seal"), piece("ledger")],
        beats: vec![
            beat("vault", &["record", "seal"]),
            beat("library", &["ledger"]),
        ],
        ways: Vec::new(),
        items: vec![StoryItem {
            id: "seal_sheet".to_owned(),
            name: "a cracked wax disc".to_owned(),
            place: "vault".to_owned(),
            look: String::new(),
            evidence: Some("seal".to_owned()),
            picture: None,
        }],
        narration: vec![
            StoryNarration {
                place: "vault".to_owned(),
                line: "The water rises around you.".to_owned(),
            },
            StoryNarration {
                place: "library".to_owned(),
                line: "Dust hangs over the shelves.".to_owned(),
            },
        ],
        struck: Vec::new(),
        revealable: None,
        findings: Findings::default(),
        phase: None,
    }
}

fn request() -> NarrateRequest {
    NarrateRequest {
        place: "vault".to_owned(),
        moment: Moment::Found,
        recent: vec!["warrior: Look at this.".to_owned()],
        model: None,
    }
}

/// `story` with `seen` and `worked_out` found at the vault, as the clue route keeps them.
fn found(mut story: Story, seen: &[&str], worked_out: &[&str]) -> Story {
    let at = |ids: &[&str]| {
        ids.iter()
            .map(|id| Finding {
                piece: (*id).to_owned(),
                place: "vault".to_owned(),
                line: "warrior: Look at this.".to_owned(),
            })
            .collect()
    };
    story.findings = Findings {
        seen: at(seen),
        worked_out: at(worked_out),
    };
    story
}

fn has(value: &Value, text: &str) -> bool {
    value.to_string().contains(text)
}

#[test]
fn the_narrator_is_told_what_was_found_and_never_what_is_hidden() {
    let story = story();
    let (shown, hidden) = known_and_hidden(&found(story.clone(), &["record"], &[]), &request());
    assert!(has(&shown, "how the record looks"), "{shown}");
    assert!(has(&shown, "the hook in the open"), "{shown}");
    assert!(has(&shown, "warrior: Look at this."), "{shown}");
    assert_eq!(
        shown["known"]["you_told"],
        json!(["The water rises around you."]),
        "the narrator is told its own lines here, not those of another place"
    );
    assert_eq!(
        shown["unseen_here"],
        json!(["a cracked wax disc"]),
        "the seal not yet looked at is named by its sheet, and the ledger is elsewhere"
    );
    for secret in [
        "the truth of it",
        "the motive of it",
        "the keeper's secret",
        "what lurks in the vault",
        "how the seal looks",
        "what the seal gives away",
        "what the record gives away",
    ] {
        assert!(
            !has(&shown, secret),
            "{secret} reached the narrator: {shown}"
        );
        assert!(
            has(&hidden, secret),
            "{secret} is not judged hidden: {hidden}"
        );
    }
    assert!(
        !has(&hidden, "ledger looks"),
        "a piece elsewhere is not one a look here would show"
    );
}

#[test]
fn what_a_found_thing_means_is_told_once_it_is_worked_out() {
    let story = story();
    let (shown, hidden) =
        known_and_hidden(&found(story.clone(), &["record"], &["record"]), &request());
    assert!(has(&shown, "what the record gives away"), "{shown}");
    assert!(
        !has(&hidden["not_yet_worked_out"], "what the record gives away"),
        "{hidden}"
    );
    let (everything, _) =
        known_and_hidden(&found(story.clone(), &["record", "seal"], &[]), &request());
    assert_eq!(everything["unseen_here"], json!([]));
}

#[test]
fn which_found_pieces_belong_together_is_judged_hidden_and_never_told_to_the_narrator() {
    let mut story = story();
    story.evidence[1].needs = vec!["record".to_owned()];
    let (shown, hidden) = known_and_hidden(
        &found(story.clone(), &["record", "seal"], &["record"]),
        &request(),
    );
    let keys: Vec<&String> = shown.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        ["known", "unseen_here"],
        "the narrator gets no pairing data: {shown}"
    );
    assert!(
        !has(&shown, "needs") && !has(&shown, "with"),
        "no link between pieces reaches the narrator: {shown}"
    );
    assert_eq!(
        hidden["links_not_worked_out"],
        json!([{"piece": "a cracked wax disc", "worked_out_with": ["the record"]}]),
        "the hidden judge holds the seal's link to the record as not yet worked out"
    );
    let (_, worked) = known_and_hidden(
        &found(story.clone(), &["record", "seal"], &["record", "seal"]),
        &request(),
    );
    assert_eq!(
        worked["links_not_worked_out"],
        json!([]),
        "a link the party worked out is no longer hidden"
    );
}

/// `story` with the party on the second stretch of the way from the vault to the library, and a
/// line the narrator said on the first.
fn on_the_way(mut story: Story) -> Story {
    let stage = |id: &str, worth_a_look: Option<&str>| WayStage {
        id: id.to_owned(),
        sight: format!("the sight of the {id}"),
        worth_a_look: worth_a_look.map(str::to_owned),
        plate: None,
    };
    story.ways = vec![StoryWay {
        from: "vault".to_owned(),
        to: "library".to_owned(),
        stages: vec![
            stage("stair", None),
            stage("arch", Some("fresh scratches on the arch")),
        ],
    }];
    story.phase = Some(PlayPhase::Way {
        from: "vault".to_owned(),
        to: "library".to_owned(),
        stage: 1,
        moves: 0,
    });
    story.narration.push(StoryNarration {
        place: "stair".to_owned(),
        line: "The stair winds down.".to_owned(),
    });
    story
}

#[test]
fn on_the_way_the_narrator_is_told_the_stretch_and_never_the_danger_ahead() {
    let story = on_the_way(story());
    let approach = NarrateRequest {
        moment: Moment::Approach,
        ..request()
    };
    let (shown, hidden) = known_and_hidden(&story, &approach);
    assert_eq!(
        shown["known"]["on_the_way"],
        json!({"sight": "the sight of the arch", "worth_a_look": "fresh scratches on the arch"})
    );
    assert_eq!(
        shown["known"]["you_told"],
        json!(["The stair winds down."]),
        "its lines on this way, not those of the place left"
    );
    assert_eq!(
        shown["unseen_here"],
        json!([]),
        "nothing of the place left is on show on the way"
    );
    assert!(!has(&shown, "what lurks in the library"), "{shown}");
    assert!(
        has(&hidden["danger"], "what lurks in the library"),
        "the danger ahead is hidden: {hidden}"
    );
    let (at_the_vault, _) = known_and_hidden(&story, &request());
    assert_eq!(
        at_the_vault["known"]["on_the_way"],
        Value::Null,
        "another moment is told nothing of the way"
    );
}

/// A host showing a plain fixture, on the clock `start_paused` stops.
async fn shown_host() -> crate::test_support::Running {
    use crate::test_support::{STARTER_FILES, start};
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/plate.png", "plate.png");
    running.write_asset(
        "fixtures/one.json",
        concat!(
            r#"{"name": "One", "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [], "#,
            r#""backdrop": {"media": "plate.png", "caption": null}}, "#,
            r#""phone": {"hero": "Pip", "status": {"kind": "ok"}}}"#
        ),
    );
    running
}

/// Moves the paused clock on by `ms` and lets the clearing task, and the blocking work it waits
/// for, finish.
async fn pass(ms: u128) {
    tokio::time::advance(std::time::Duration::from_millis(u64::try_from(ms).unwrap())).await;
    for _ in 0..200 {
        tokio::task::yield_now().await;
        std::thread::yield_now();
    }
}

#[tokio::test(start_paused = true)]
async fn a_newer_line_keeps_the_box_up_past_the_first_lines_rest_and_then_it_clears() {
    let running = shown_host().await;
    let port = running.port;
    let run = |line: &'static str| async move {
        let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        crate::dev::run_dev(port, &words).await
    };
    let shown = || {
        running
            .state
            .playground()
            .snapshot()
            .scene
            .narration
            .set_value()
            .map(|narration| narration.text.clone())
    };
    let (first, second) = ("The rain has stopped.", "A bell rings below.");
    let said = run("persona narrate The rain has stopped.").await.unwrap();
    assert!(said.contains(first), "{said}");
    assert_eq!(shown().as_deref(), Some(first));
    let first_rest = rest_of_narration(first).as_millis();
    pass(first_rest - 2_000).await;
    run("persona narrate A bell rings below.").await.unwrap();
    // Past the first line's rest, before the second's.
    pass(3_000).await;
    assert_eq!(shown().as_deref(), Some(second));
    pass(rest_of_narration(second).as_millis()).await;
    assert_eq!(shown(), None, "the box leaves after the newest line's rest");
    assert!(
        run("persona narrate").await.is_err(),
        "a narration needs its words"
    );
}
