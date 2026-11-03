//! The look-ahead verbs: their grammar, their help line, and a run through a host on an ephemeral
//! port (never 7878), which the test stops when it drops.

use df_table::lookahead::Expectation;

use super::cli::{LOOKAHEAD_USAGE, LookaheadCommand};
use super::request::{LookaheadReply, LookaheadRequest};
use crate::command::{Command, CommandError, parse, split_words};
use crate::dev::run_dev;
use crate::scene_builder::SceneBuilderCommand;
use crate::scene_builder::request::{DevStage, SceneReply, SlowStage};
use crate::scene_builder::routes::path;
use crate::test_support::{STARTER_FILES, post_request, raw_exchange, start};

fn command(line: &str) -> Result<LookaheadCommand, CommandError> {
    let words = split_words(line).expect("the line splits");
    match parse(&words)? {
        Command::SceneBuilder(SceneBuilderCommand::Lookahead(command)) => Ok(command),
        other => panic!("'{line}' parsed as {other:?}"),
    }
}

async fn dev(port: u16, line: &str) -> String {
    let words = split_words(line).expect("the line splits");
    run_dev(port, &words)
        .await
        .unwrap_or_else(|error| panic!("'{line}' failed: {error}"))
}

#[test]
fn the_verbs_have_the_shape_of_the_other_scene_verbs() {
    assert_eq!(
        command("scene lookahead show").expect("parses"),
        LookaheadCommand::Show
    );
    assert_eq!(
        command("scene lookahead pause").expect("parses"),
        LookaheadCommand::Change(LookaheadRequest::Pause)
    );
    assert_eq!(
        command("scene lookahead resume").expect("parses"),
        LookaheadCommand::Change(LookaheadRequest::Resume)
    );
    assert_eq!(
        command("scene lookahead clear").expect("parses"),
        LookaheadCommand::Change(LookaheadRequest::Clear)
    );
    assert_eq!(
        command("scene lookahead target \"Harbor market\"").expect("parses"),
        LookaheadCommand::Change(LookaheadRequest::Target {
            place: "Harbor market".to_owned(),
            kind: Expectation::AuthoredBeat,
            likelihood_permille: None,
            fail: None,
            slow: None,
        })
    );
    assert_eq!(
        command("scene lookahead target quay kind=exit_hint likelihood=700 fail=describe slow=background:12")
            .expect("parses"),
        LookaheadCommand::Change(LookaheadRequest::Target {
            place: "quay".to_owned(),
            kind: Expectation::ExitHint,
            likelihood_permille: Some(700),
            fail: Some(DevStage::Describe),
            slow: Some(SlowStage {
                stage: DevStage::Background,
                seconds: 12,
            }),
        })
    );
}

#[test]
fn a_mistake_gets_the_usage_line_or_names_what_was_wrong() {
    let usage = |line: &str| match command(line) {
        Err(CommandError::Usage(text)) => text,
        other => panic!("'{line}' gave {other:?}"),
    };
    assert_eq!(usage("scene lookahead"), LOOKAHEAD_USAGE);
    assert_eq!(usage("scene lookahead show now"), LOOKAHEAD_USAGE);
    assert_eq!(usage("scene lookahead target"), LOOKAHEAD_USAGE);
    assert_eq!(
        usage("scene lookahead target quay surprise=1"),
        LOOKAHEAD_USAGE
    );
    assert!(matches!(
        command("scene lookahead target quay kind=maybe"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        command("scene lookahead target quay fail=plate"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        command("scene lookahead target quay slow=background"),
        Err(CommandError::BadPair(_))
    ));
    assert!(matches!(
        command("scene lookahead target quay likelihood=lots"),
        Err(CommandError::BadCount { .. })
    ));
    assert!(matches!(
        command("scene lookahead target quay likelihood=70000"),
        Err(CommandError::BadCount { .. })
    ));
    assert!(matches!(
        command("scene lookahead target quay fail=describe fail=background"),
        Err(CommandError::DuplicateKey(_))
    ));
}

#[test]
fn help_lists_the_look_ahead_verbs() {
    assert!(crate::command::usage_text().contains(LOOKAHEAD_USAGE));
}

#[tokio::test]
async fn the_look_ahead_runs_from_the_command_line_and_the_next_build_finds_the_scene_ready() {
    let host = start(&STARTER_FILES).await;
    dev(host.port, "scene build \"Harbor market\"").await;

    let targeted = dev(host.port, "scene lookahead target Quay").await;
    assert!(targeted.contains("expected next:"), "{targeted}");
    assert!(
        targeted.contains("scene 2 (Quay): authored_beat, certain"),
        "{targeted}"
    );
    assert!(targeted.contains("LOOKAHEAD started scene=2"), "{targeted}");
    assert!(
        targeted.contains("RESOURCE request purpose=lookahead_describe class=lookahead"),
        "{targeted}"
    );

    dev(host.port, "plates run 5 quiet").await;
    let shown = dev(host.port, "scene lookahead show").await;
    assert!(shown.contains("scene 2 build 1: ready"), "{shown}");
    assert!(shown.contains("built 1"), "{shown}");

    let built = dev(host.port, "scene build Quay").await;
    assert!(built.contains("the look-ahead found it: hit"), "{built}");
    assert!(built.contains("source=lookahead"), "{built}");
    assert!(
        built.contains("LOOKAHEAD taken scene=2 outcome=hit"),
        "{built}"
    );
    let after = dev(host.port, "scene lookahead show").await;
    assert!(after.contains("hit 1"), "{after}");
    assert!(after.contains("book: empty"), "{after}");

    let paused = dev(host.port, "scene lookahead pause").await;
    assert!(paused.contains(": paused,"), "{paused}");
    let cleared = dev(host.port, "scene lookahead clear").await;
    assert!(cleared.contains("expected next: nothing"), "{cleared}");
}

#[tokio::test]
async fn the_route_refuses_a_bad_body_a_foreign_host_and_a_stage_that_is_not_built_ahead() {
    let host = start(&STARTER_FILES).await;
    let bad = raw_exchange(
        host.port,
        &post_request(
            path::LOOKAHEAD,
            r#"{"action":"target","place":"quay","surprise":1}"#,
            "",
        ),
    )
    .await;
    assert!(bad.starts_with("HTTP/1.1 400"), "{bad}");
    assert!(bad.contains("\"outcome\":\"rejected\""), "{bad}");

    let plate = raw_exchange(
        host.port,
        &post_request(
            path::LOOKAHEAD,
            r#"{"action":"target","place":"quay","fail":"plate"}"#,
            "",
        ),
    )
    .await;
    assert!(plate.starts_with("HTTP/1.1 400"), "{plate}");
    assert!(
        plate.contains("only describe and background are built ahead"),
        "{plate}"
    );

    let foreign = raw_exchange(
        host.port,
        &format!(
            "GET {} HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
            path::LOOKAHEAD
        ),
    )
    .await;
    assert!(foreign.starts_with("HTTP/1.1 403"), "{foreign}");
}

#[tokio::test]
async fn a_reply_reads_back_as_the_typed_value() {
    let host = start(&STARTER_FILES).await;
    let response = raw_exchange(
        host.port,
        &post_request(path::LOOKAHEAD, r#"{"action":"pause"}"#, ""),
    )
    .await;
    let body = response.split("\r\n\r\n").nth(1).expect("a body");
    let reply: SceneReply<LookaheadReply> = serde_json::from_str(body).expect("a typed reply");
    let SceneReply::Done { value } = reply else {
        panic!("pause was refused: {body}");
    };
    assert!(value.report.paused && value.report.enabled);
}
