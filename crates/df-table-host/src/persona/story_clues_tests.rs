//! Clue discovery: a line finds only what play could reach, and research builds on what was found.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use df_ai::text::{TextOutcome, TextRequest};
use df_ai::text_fixture::ScriptedBackend;
use df_content::persona::backfill::NarrativeGuidance;
use serde_json::{Value, json};

use super::story::{
    PERSONA_STORY_RESET, Story, StoryBeat, StoryCharacter, StoryEvidence, StoryItem,
    StoryNarration, StoryReply, StoryScene, read_story, router_with, story_path, without_play,
    write_story,
};
use super::story_clues::{
    CluesFound, CluesReply, CluesRequest, Finding, Findings, PERSONA_STORY_CLUES,
    STALLED_PLAYER_LINES, Stretch, board, candidates, read_clues, record_clues,
};
use super::story_narrate::{Moment, NarrateReply, NarrateRequest, PERSONA_STORY_NARRATE};
use super::story_next::PLAYER;
use crate::dev_client::{post, post_json, read_answer};
use crate::dev_http::DevReply;
use crate::test_support::{Running, STARTER_FILES, start};
use crate::text::{BackendChoice, TextRuntime};

fn piece(id: &str, needs: &[&str]) -> StoryEvidence {
    StoryEvidence {
        id: id.to_owned(),
        what: format!("the {id}"),
        planted_by: "rival".to_owned(),
        flaw: format!("what the {id} gives away"),
        up_close: format!("how the {id} looks"),
        needs: needs.iter().map(|id| (*id).to_owned()).collect(),
    }
}

fn place(id: &str, exhibits: &[&str]) -> (StoryScene, StoryBeat) {
    let scene = StoryScene {
        id: id.to_owned(),
        place: format!("the {id}"),
        dressing: Vec::new(),
        danger: String::new(),
        music: String::new(),
    };
    let beat = StoryBeat {
        id: format!("beat_{id}"),
        scene: id.to_owned(),
        happens: String::new(),
        turns_on: String::new(),
        leads: String::new(),
        moves: Vec::new(),
        exhibits: exhibits.iter().map(|id| (*id).to_owned()).collect(),
    };
    (scene, beat)
}

/// A record in the library is needed to read the seal in the vault; the warrior is a player.
fn story() -> Story {
    let (library, library_beat) = place("library", &["record", "ledger"]);
    let (vault, vault_beat) = place("vault", &["seal"]);
    Story {
        title: "The Seal".to_owned(),
        hook: String::new(),
        truth: String::new(),
        culprit: "rival".to_owned(),
        motive: String::new(),
        scenes: vec![library, vault],
        characters: vec![StoryCharacter {
            id: "warrior".to_owned(),
            name: "Brann".to_owned(),
            new: false,
            part: PLAYER.to_owned(),
            role: String::new(),
            guidance: NarrativeGuidance::default(),
            secrets: Vec::new(),
        }],
        evidence: vec![
            piece("record", &[]),
            piece("ledger", &[]),
            piece("seal", &["record"]),
        ],
        beats: vec![library_beat, vault_beat],
        ways: Vec::new(),
        items: Vec::new(),
        narration: Vec::new(),
        struck: Vec::new(),
        revealable: None,
        findings: Findings::default(),
        phase: None,
    }
}

fn ids(clues: &[super::story_clues::Clue]) -> Vec<&str> {
    clues.iter().map(|clue| clue.id.as_str()).collect()
}

/// The pieces of `found`, oldest first.
fn pieces(found: &[Finding]) -> Vec<&str> {
    found.iter().map(|found| found.piece.as_str()).collect()
}

/// What the clue judge's `answer` finds for `line` at `place`, committed to `story`, the line said
/// by a player (`by_player`) or anyone else.
fn line_finds(
    story: &mut Story,
    (place, line): (&str, &str),
    answer: &Value,
    by_player: bool,
) -> CluesFound {
    let said = read_clues(answer, (story, place)).expect("the answer touches only what it may");
    record_clues(story, (place, line), &said, by_player)
}

#[test]
fn a_line_touches_what_is_on_show_and_what_was_seen_before_but_not_yet_worked_out() {
    let mut story = story();
    assert_eq!(candidates(&story, "library"), ["record", "ledger"]);
    let answer = json!({"seen": ["record", "ledger"], "worked_out": ["ledger"]});
    line_finds(&mut story, ("library", "warrior: Both."), &answer, true);
    assert_eq!(candidates(&story, "vault"), ["record", "seal"]);
}

#[test]
fn a_close_look_is_committed_once_with_its_place_and_line_and_makes_its_flaw_workable() {
    let mut story = story();
    let answer = json!({"seen": ["record"], "worked_out": []});
    let line = "warrior: The ink on the record is fresh.";
    let found = line_finds(&mut story, ("library", line), &answer, true);
    assert_eq!(ids(&found.seen), ["record"]);
    assert_eq!(found.seen[0].text, "how the record looks");
    assert_eq!(
        ids(&found.workable),
        ["record"],
        "a piece needing nothing is workable once seen"
    );
    assert_eq!(
        story.findings.seen,
        [Finding {
            piece: "record".to_owned(),
            place: "library".to_owned(),
            line: line.to_owned(),
        }]
    );
    let again = line_finds(&mut story, ("library", "warrior: Again."), &answer, true);
    assert!(again.seen.is_empty(), "a piece seen twice is new once");
    assert_eq!(ids(&again.workable), ["record"]);
    assert_eq!(pieces(&story.findings.seen), ["record"], "and kept once");
    assert_eq!(
        story.findings.seen[0].line, line,
        "with the line that found it first"
    );
}

#[test]
fn a_piece_is_worked_out_only_once_everything_it_needs_was_seen() {
    let mut story = story();
    let answer = json!({"seen": ["seal"], "worked_out": ["seal"]});
    let guessed = line_finds(&mut story, ("vault", "warrior: Forged."), &answer, true);
    assert_eq!(ids(&guessed.seen), ["seal"]);
    assert!(
        guessed.worked_out.is_empty(),
        "the seal is read only with the record"
    );
    assert!(guessed.workable.is_empty());
    assert!(story.findings.worked_out.is_empty());
    let look = json!({"seen": ["record"], "worked_out": []});
    line_finds(&mut story, ("library", "warrior: A record."), &look, true);
    let researched = line_finds(&mut story, ("vault", "warrior: It differs."), &answer, true);
    assert!(researched.seen.is_empty(), "the seal was seen already");
    assert_eq!(ids(&researched.worked_out), ["seal"]);
    assert_eq!(researched.worked_out[0].text, "what the seal gives away");
    assert_eq!(
        ids(&researched.workable),
        ["record"],
        "the record itself is still to work out"
    );
    assert_eq!(pieces(&story.findings.worked_out), ["seal"]);
    assert_eq!(story.findings.worked_out[0].place, "vault");
    let twice = line_finds(&mut story, ("vault", "warrior: Still."), &answer, true);
    assert!(twice.worked_out.is_empty(), "a piece is worked out once");
    assert_eq!(pieces(&story.findings.worked_out), ["seal"]);
}

#[test]
fn a_piece_is_seen_only_where_it_is_on_show_and_a_stray_id_is_refused() {
    let mut story = story();
    let look = json!({"seen": ["record"], "worked_out": []});
    line_finds(&mut story, ("library", "warrior: A record."), &look, true);
    let elsewhere = line_finds(&mut story, ("vault", "warrior: That record."), &look, true);
    assert!(
        elsewhere.seen.is_empty(),
        "the record was seen already, in the library"
    );
    assert_eq!(story.findings.seen[0].place, "library");
    let stray = json!({"seen": ["seal"], "worked_out": []});
    assert!(
        read_clues(&stray, (&story, "library")).is_err(),
        "the seal is not on show in the library"
    );
}

#[test]
fn only_a_player_works_a_piece_out_though_anyone_may_say_what_a_look_shows() {
    let mut story = story();
    let answer = json!({"seen": ["record"], "worked_out": ["record"]});
    let rival = line_finds(&mut story, ("library", "rival: Fresh ink."), &answer, false);
    assert_eq!(ids(&rival.seen), ["record"]);
    assert!(rival.worked_out.is_empty(), "the rival works nothing out");
    assert!(story.findings.worked_out.is_empty());
    let player = line_finds(
        &mut story,
        ("library", "warrior: Fresh ink."),
        &answer,
        true,
    );
    assert_eq!(ids(&player.worked_out), ["record"]);
    assert_eq!(story.findings.worked_out[0].line, "warrior: Fresh ink.");
}

#[test]
fn the_board_lists_every_piece_seen_oldest_first_by_its_sheet() {
    let mut story = story();
    story.items.push(StoryItem {
        id: "old_record_book".to_owned(),
        name: "Old record book".to_owned(),
        place: "library".to_owned(),
        look: "A swollen book.".to_owned(),
        evidence: Some("record".to_owned()),
        picture: Some("record-closeup.png".to_owned()),
    });
    let look = json!({"seen": ["record"], "worked_out": []});
    line_finds(&mut story, ("library", "warrior: A record."), &look, true);
    let answer = json!({"seen": ["seal"], "worked_out": ["seal"]});
    let found = line_finds(&mut story, ("vault", "warrior: Forged."), &answer, true);
    let notes = board(&story, &found);
    let ids: Vec<&str> = notes.iter().map(|note| note.id.as_str()).collect();
    assert_eq!(ids, ["record", "seal"], "oldest first");
    assert_eq!(notes[0].name, "Old record book", "named by its sheet");
    assert_eq!(notes[0].picture.as_deref(), Some("record-closeup.png"));
    assert!(!notes[0].worked_out);
    assert_eq!(notes[0].detail, "how the record looks");
    assert_eq!(notes[1].name, "the seal", "by what it is without a sheet");
    assert!(notes[1].worked_out);
    assert_eq!(notes[1].detail, "what the seal gives away");
}

#[test]
fn a_reset_clears_the_runs_play_and_keeps_the_plot_and_its_sheets() {
    let mut story = story();
    story.items.push(StoryItem {
        id: "old_record_book".to_owned(),
        name: "Old record book".to_owned(),
        place: "library".to_owned(),
        look: "A swollen book.".to_owned(),
        evidence: Some("record".to_owned()),
        picture: None,
    });
    let plot = story.clone();
    let answer = json!({"seen": ["record"], "worked_out": ["record"]});
    line_finds(
        &mut story,
        ("library", "warrior: Fresh ink."),
        &answer,
        true,
    );
    story.narration.push(StoryNarration {
        place: "library".to_owned(),
        line: "Dust hangs here.".to_owned(),
    });
    story.struck.push("vault".to_owned());
    assert_eq!(without_play(story), plot);
}

/// What a test host's text service was asked, in order.
type Asked = Arc<std::sync::Mutex<Vec<TextRequest>>>;

/// A host serving the story routes on a port of its own, with `story` stored and its text
/// service answering `answers` in order.
async fn story_host(story: &Story, answers: Vec<Value>) -> (Running, u16, Asked) {
    let running = start(&STARTER_FILES).await;
    running.write_asset("persona/story.json", "{}");
    write_story(&story_path(&running.state), story).expect("keeps the story");
    let asked = Asked::default();
    let script = ScriptedBackend::new(
        answers.into_iter().map(TextOutcome::Answered),
        Arc::new(AtomicUsize::new(0)),
    )
    .recording(Arc::clone(&asked));
    let recorder = Arc::clone(&running.state);
    let text = TextRuntime::new(
        BackendChoice::Scripted(Arc::new(script), None),
        Arc::new(move |line| recorder.record_line(line)),
    );
    let app = router_with(Arc::clone(&running.state), Arc::new(text));
    let listener = crate::server::bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        // The server runs until the test's runtime stops; its end is the test's end.
        let _ = axum::serve(listener, app).await;
    });
    (running, port, asked)
}

fn stored(running: &Running) -> Story {
    read_story(&story_path(&running.state)).expect("the story is kept")
}

#[tokio::test]
async fn the_clue_route_commits_what_a_line_found_once_and_reads_it_back() {
    let look = json!({"seen": ["record"], "worked_out": []});
    let (running, port, asked) = story_host(&story(), vec![look.clone(), look]).await;
    let request = CluesRequest {
        place: "library".to_owned(),
        line: "warrior: The ink on the record is fresh.".to_owned(),
        model: None,
    };
    let first: CluesReply = post_json(port, PERSONA_STORY_CLUES, &request)
        .await
        .unwrap();
    let DevReply::Done { value: first } = first else {
        panic!("first: {first:?}");
    };
    assert_eq!(ids(&first.seen), ["record"]);
    assert_eq!(
        stored(&running).findings.seen,
        [Finding {
            piece: "record".to_owned(),
            place: "library".to_owned(),
            line: request.line.clone(),
        }]
    );
    let again = CluesRequest {
        line: "warrior: Look again.".to_owned(),
        ..request.clone()
    };
    let second: CluesReply = post_json(port, PERSONA_STORY_CLUES, &again).await.unwrap();
    let DevReply::Done { value: second } = second else {
        panic!("second: {second:?}");
    };
    assert!(second.seen.is_empty(), "seen once");
    assert_eq!(ids(&second.workable), ["record"], "read from the record");
    assert_eq!(pieces(&stored(&running).findings.seen), ["record"]);
    assert_eq!(
        asked.lock().unwrap().len(),
        2,
        "each line is judged, the second against the record"
    );
}

#[tokio::test]
async fn the_narrator_is_told_what_the_record_holds() {
    let mut kept = story();
    kept.findings.seen.push(Finding {
        piece: "record".to_owned(),
        place: "library".to_owned(),
        line: "warrior: Fresh ink.".to_owned(),
    });
    let silent = json!({"speak": false, "line": null});
    let (_running, port, asked) = story_host(&kept, vec![silent]).await;
    let request = NarrateRequest {
        place: "library".to_owned(),
        moment: Moment::Found,
        recent: Vec::new(),
        model: None,
    };
    let reply: NarrateReply = post_json(port, PERSONA_STORY_NARRATE, &request)
        .await
        .unwrap();
    assert!(matches!(reply, DevReply::Done { .. }), "{reply:?}");
    let asked = asked.lock().unwrap();
    let found = &asked[0].facts["known"]["found"];
    assert_eq!(
        found[0]["what_a_look_showed"], "how the record looks",
        "{found}"
    );
    assert_eq!(asked[0].facts["unseen_here"], json!(["the ledger"]));
}

#[tokio::test]
async fn the_reset_route_clears_the_stored_play_and_keeps_the_plot() {
    let mut kept = story();
    let plot = kept.clone();
    let answer = json!({"seen": ["record"], "worked_out": ["record"]});
    line_finds(&mut kept, ("library", "warrior: Fresh ink."), &answer, true);
    kept.struck.push("vault".to_owned());
    let (running, port, _) = story_host(&kept, Vec::new()).await;
    let reply: StoryReply =
        read_answer(&post(port, PERSONA_STORY_RESET, "").await.unwrap()).unwrap();
    assert!(matches!(reply, DevReply::Done { .. }), "{reply:?}");
    assert_eq!(stored(&running), plot);
}

/// Player lines at `place` with `found` pieces found after each; whether each stalled the stretch.
fn player_lines(stretch: &mut Stretch, place: &str, found: &[usize]) -> Vec<bool> {
    found
        .iter()
        .map(|found| stretch.stalls_with(place, *found, true))
        .collect()
}

#[test]
fn the_party_is_stuck_once_after_its_stretch_of_player_lines_with_no_progress() {
    let mut stretch = Stretch::default();
    let quiet = [1; STALLED_PLAYER_LINES + 3];
    let stalled = player_lines(&mut stretch, "library", &quiet);
    let fired: Vec<usize> = (0..stalled.len()).filter(|line| stalled[*line]).collect();
    assert_eq!(
        fired,
        [STALLED_PLAYER_LINES],
        "the line opening the stretch is not counted, and the hint is asked for once"
    );
}

#[test]
fn only_player_lines_count_toward_a_stuck_party() {
    let mut stretch = Stretch::default();
    assert!(!stretch.stalls_with("library", 0, true));
    for _ in 0..STALLED_PLAYER_LINES * 2 {
        assert!(!stretch.stalls_with("library", 0, false));
    }
    let stalled = player_lines(&mut stretch, "library", &[0; STALLED_PLAYER_LINES]);
    assert_eq!(stalled.last(), Some(&true));
}

#[test]
fn progress_or_a_new_place_starts_a_new_stretch() {
    let mut stretch = Stretch::default();
    let short = [0; STALLED_PLAYER_LINES];
    assert!(!player_lines(&mut stretch, "library", &short).contains(&true));
    assert!(
        !stretch.stalls_with("library", 1, true),
        "a piece newly seen resets the count"
    );
    assert!(!player_lines(&mut stretch, "library", &[1; STALLED_PLAYER_LINES - 1]).contains(&true));
    assert!(
        !stretch.stalls_with("vault", 1, true),
        "a new place resets the count"
    );
    let stalled = player_lines(&mut stretch, "vault", &[1; STALLED_PLAYER_LINES]);
    assert_eq!(stalled.last(), Some(&true));
    assert!(
        !stretch.stalls_with("vault", 2, true),
        "progress after a hint starts a stretch"
    );
    let stalled = player_lines(&mut stretch, "vault", &[2; STALLED_PLAYER_LINES]);
    assert_eq!(stalled.last(), Some(&true), "a new stretch may hint again");
}
