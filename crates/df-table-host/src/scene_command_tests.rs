//! The `scene` commands: parsed, sent, applied by the host and drawn on top of the fixture.

use axum::http::StatusCode;
use df_playground::fixture::{Choice, PartyMember, UnavailableChoice};
use df_playground::overlay::Edit;
use df_playground::scene::SceneChange;

use crate::client::{get, post_json};
use crate::command::{Command, CommandError, parse, split_words, usage_text};
use crate::dev::{DevError, ReplyError, run_dev};
use crate::playground::PlaygroundChangeError;
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::{ControlsReply, EventsReply};

const FIXTURE: &str = r#"{
    "name": "One",
    "tv": {
        "place": "Here", "status": {"kind": "ok"}, "elements": [],
        "caption": {"text": "Original line."},
        "party": [{"name": "Pip", "hit_points": 7, "maximum_hit_points": 9}]
    },
    "phone": {
        "hero": "Pip", "status": {"kind": "ok"},
        "groups": [{"name": "Talk", "choices": [{"label": "Greet"}]}]
    }
}"#;

fn scene(line: &str) -> Result<SceneChange, CommandError> {
    let words = split_words(line).expect("the line splits");
    match parse(&words)? {
        Command::Scene(change) => Ok(change),
        other => panic!("expected a scene change, got {other:?}"),
    }
}

fn choice(label: &str, explain: Option<&str>) -> Choice {
    Choice {
        label: label.to_owned(),
        explain: explain.map(str::to_owned),
    }
}

#[test]
fn a_caption_is_one_text_with_an_optional_speaker() {
    assert_eq!(
        scene(r#"scene caption "You hear a knock." speaker=Marta"#),
        Ok(SceneChange::Caption {
            speaker: Some("Marta".into()),
            text: "You hear a knock.".into()
        })
    );
    assert_eq!(
        scene(r#"scene caption "Plain""#),
        Ok(SceneChange::Caption {
            speaker: None,
            text: "Plain".into()
        })
    );
    assert_eq!(
        scene("scene caption-clear"),
        Ok(SceneChange::ClearCaption {})
    );
    assert!(matches!(
        scene("scene caption"),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        scene(r#"scene caption Hi volume=3"#),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn a_party_member_is_added_with_hit_points_and_optional_details() {
    assert_eq!(
        scene("scene party add Anselm hp=12 max=14 color=#d9a441 conditions=Prone,Frightened"),
        Ok(SceneChange::PartyAdd {
            member: PartyMember {
                name: "Anselm".into(),
                hit_points: 12,
                maximum_hit_points: 14,
                conditions: vec!["Prone".into(), "Frightened".into()],
                color: Some("#d9a441".into()),
            }
        })
    );
    let Ok(SceneChange::PartyAdd { member }) = scene("scene party add Zed hp=6") else {
        panic!("expected a member");
    };
    assert_eq!(member.maximum_hit_points, 6, "max defaults to hp");
    assert!(matches!(
        scene("scene party add Zed"),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn a_party_member_is_changed_removed_or_the_party_cleared() {
    assert_eq!(
        scene("scene party set Pip hp=3 conditions="),
        Ok(SceneChange::PartySet {
            name: "Pip".into(),
            hit_points: Some(3),
            maximum_hit_points: None,
            conditions: Some(Vec::new()),
        })
    );
    assert_eq!(
        scene("scene party remove Pip"),
        Ok(SceneChange::PartyRemove { name: "Pip".into() })
    );
    assert_eq!(scene("scene party clear"), Ok(SceneChange::PartyClear {}));
    assert!(matches!(
        scene("scene party set Pip"),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn hit_points_must_be_whole_numbers_and_keys_may_not_repeat() {
    for bad in ["hp=lots", "hp=-1", "hp=2.5"] {
        assert!(
            matches!(
                scene(&format!("scene party add Zed {bad}")),
                Err(CommandError::BadCount { .. })
            ),
            "{bad}"
        );
    }
    assert_eq!(
        scene("scene party add Zed hp=1 hp=2"),
        Err(CommandError::DuplicateKey("hp".into()))
    );
}

#[test]
fn choices_are_a_list_with_optional_explanations_a_named_group_or_unavailable_with_reasons() {
    assert_eq!(
        scene(r#"scene choices suggested "Sit down=Near the fire" Leave"#),
        Ok(SceneChange::Suggested {
            choices: vec![
                choice("Sit down", Some("Near the fire")),
                choice("Leave", None)
            ]
        })
    );
    assert_eq!(
        scene("scene choices group Talk Greet Haggle"),
        Ok(SceneChange::Group {
            name: "Talk".into(),
            choices: vec![choice("Greet", None), choice("Haggle", None)]
        })
    );
    assert_eq!(
        scene(r#"scene choices unavailable "Cast Light=No slots""#),
        Ok(SceneChange::Unavailable {
            choices: vec![UnavailableChoice {
                label: "Cast Light".into(),
                reason: "No slots".into()
            }]
        })
    );
    assert_eq!(
        scene("scene choices clear"),
        Ok(SceneChange::ClearChoices {})
    );
    assert_eq!(
        scene("scene choices suggested"),
        Ok(SceneChange::Suggested {
            choices: Vec::new()
        })
    );
    assert!(matches!(
        scene("scene choices unavailable NoReason"),
        Err(CommandError::BadPair(_))
    ));
    assert!(matches!(
        scene("scene choices group"),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn reset_and_a_bare_scene_are_handled_and_help_lists_the_verbs() {
    assert_eq!(scene("scene reset"), Ok(SceneChange::Reset {}));
    assert!(matches!(scene("scene"), Err(CommandError::Usage(_))));
    assert!(matches!(
        scene("scene reset now"),
        Err(CommandError::Usage(_))
    ));
    let help = usage_text();
    for text in [
        "scene caption",
        "caption-clear",
        "scene party add",
        "scene choices suggested",
    ] {
        assert!(help.contains(text), "{text}");
    }
}

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

async fn with_fixture(running: &Running) {
    running.write_asset("fixtures/one.json", FIXTURE);
    // Reading the controls chooses the first fixture, as the page does on its first poll.
    get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
}

async fn shown(running: &Running) -> ControlsReply {
    let response = get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    serde_json::from_str(&response.body).expect("controls")
}

#[tokio::test]
async fn a_scene_change_is_applied_numbered_shown_by_playground_and_recorded() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let before = shown(&running).await.revision;
    let said = run(&running, r#"scene caption "A new line." speaker=Narrator"#)
        .await
        .expect("applies");
    assert_eq!(said, format!("ok (revision {})", before + 1));
    let reply = shown(&running).await;
    assert!(matches!(&reply.scene.caption, Edit::Set(caption) if caption.text == "A new line."));
    assert!(
        run(&running, "playground")
            .await
            .expect("shows")
            .contains("scene: changed: caption")
    );
    let events = get(running.port, "/events.json").await.expect("answers");
    let events: EventsReply = serde_json::from_str(&events.body).expect("events");
    assert!(
        events
            .events
            .iter()
            .any(|line| line.text.starts_with("PLAYGROUND caption"))
    );
}

#[tokio::test]
async fn a_party_change_starts_from_the_fixtures_party_and_reset_returns_to_the_file() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    run(&running, "scene party add Vex hp=4 max=10")
        .await
        .expect("adds");
    let party = shown(&running)
        .await
        .scene
        .party
        .set_value()
        .cloned()
        .expect("the party is overridden");
    let names: Vec<String> = party.into_iter().map(|member| member.name).collect();
    assert_eq!(names, vec!["Pip", "Vex"]);
    run(&running, "scene reset").await.expect("resets");
    assert!(shown(&running).await.scene.party.is_keep());
    assert!(
        run(&running, "playground")
            .await
            .expect("shows")
            .contains("scene: as the fixture has it")
    );
}

#[tokio::test]
async fn choosing_another_fixture_drops_the_live_changes() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/a.json", FIXTURE);
    running.write_asset("fixtures/b.json", FIXTURE);
    get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    run(&running, "scene caption Hello").await.expect("applies");
    assert!(!shown(&running).await.scene.caption.is_keep());
    run(&running, "playground set fixture=b.json")
        .await
        .expect("switches");
    assert!(shown(&running).await.scene.caption.is_keep());
}

#[tokio::test]
async fn a_refused_scene_change_says_why_and_changes_nothing() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let before = shown(&running).await;
    let error = run(&running, "scene party remove Ghost")
        .await
        .expect_err("refused");
    assert!(
        refused_change(&error).contains("'Ghost' is not in the party"),
        "{error}"
    );
    let error = run(&running, "scene party add Pip hp=1")
        .await
        .expect_err("refused");
    assert!(
        refused_change(&error).contains("already in the party"),
        "{error}"
    );
    assert_eq!(shown(&running).await, before);
}

/// The sentence of a scene change the playground's checks refused; what it says is what these
/// tests are about.
fn refused_change(error: &DevError) -> &str {
    match error {
        DevError::Rejected {
            error: ReplyError::Playground(PlaygroundChangeError::InvalidChange { detail }),
            ..
        } => detail,
        other => panic!("not a refused change: {other:?}"),
    }
}

#[tokio::test]
async fn with_no_fixture_shown_a_scene_change_says_to_write_one() {
    let running = start(&STARTER_FILES).await;
    let error = run(&running, "scene caption Hi")
        .await
        .expect_err("refused");
    assert!(refused_change(&error).contains("fixture write"), "{error}");
}

#[tokio::test]
async fn a_body_that_is_not_a_scene_change_is_a_400_and_recorded() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    for body in ["", "nope", r#"{"change":"dance"}"#] {
        let response = post_json(running.port, "/dev/playground/scene", body)
            .await
            .expect("answers");
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{body}");
    }
}

#[tokio::test]
async fn a_script_can_stage_a_whole_scene() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let path = directory.path().join("staging.txt");
    std::fs::write(
        &path,
        "# stage the scene\nscene caption \"The door opens.\"\nscene party set Pip hp=2\nscene choices suggested Run Hide\n",
    )
    .expect("writes the script");
    let words = ["script".to_owned(), path.to_string_lossy().into_owned()];
    let said = run_dev(running.port, &words).await.expect("runs");
    assert!(said.ends_with("ran 3 commands, all ok"), "{said}");
    let scene = shown(&running).await.scene;
    assert_eq!(scene.party.set_value().expect("party")[0].hit_points, 2);
    assert_eq!(scene.suggested.set_value().expect("suggested").len(), 2);
}
