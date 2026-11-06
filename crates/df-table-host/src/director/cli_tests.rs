//! The grammar of `camera focus|wide` and `director ...`: parsed, refused with the choices, and
//! listed in the help; and the verbs run against a host: a verb moves the playground's camera
//! through the same path as the dev camera verbs, with the same numbers, the lines it writes are
//! the lines `director show` prints, and a refusal comes back as the typed `DirectorError`.

use axum::http::StatusCode;
use df_table::director::error::DirectorError as DecisionError;
use df_table::director::reason::Reason;
use df_table::element::ElementId;

use super::cli::{CAMERA_USAGE, DIRECTOR_USAGE, DirectorCommand, VIEW_USAGE};
use super::fixtures::{camera_of, lines_starting, run, said, with_three_avatars};
use super::request::{ActReply, DirectorAct, DirectorError, DirectorReply, StartKind, ViewEdit};
use super::routes::path;
use crate::client::post_json;
use crate::command::{Command, CommandError, parse, parse_script_line, split_words, usage_text};
use crate::dev::{DevError, run_dev};
use crate::dev_client;
use crate::dev_http::{BodyCause, BodyError, DevReply, RouteError};

const TOUR: &str = include_str!("../../../../development/playground-scripts/director-tour.txt");

fn id(name: &str) -> ElementId {
    ElementId::new(name).expect("an element name")
}

fn command(line: &str) -> Result<DirectorCommand, CommandError> {
    let words = split_words(line).expect("the line splits");
    match parse(&words)? {
        Command::Director(command) => Ok(command),
        other => panic!("'{line}' parsed as {other:?}"),
    }
}

fn act(line: &str) -> DirectorAct {
    match command(line).expect("parses") {
        DirectorCommand::Act(act) => act,
        DirectorCommand::Show => panic!("'{line}' is show"),
    }
}

#[test]
fn camera_focus_takes_an_element_and_the_four_keys() {
    assert_eq!(
        act("camera focus finn"),
        DirectorAct::Focus {
            element: id("finn"),
            reason: None,
            zoom_percent: None,
            over_ms: None,
            hold_ms: None,
        }
    );
    assert_eq!(
        act("camera focus sigrun reason=hit zoom=250 over=1500 hold=4000"),
        DirectorAct::Focus {
            element: id("sigrun"),
            reason: Some(Reason::Hit),
            zoom_percent: Some(250),
            over_ms: Some(1_500),
            hold_ms: Some(4_000),
        }
    );
    assert_eq!(act("camera wide"), DirectorAct::Wide { over_ms: None });
    assert_eq!(
        act("camera wide over=300"),
        DirectorAct::Wide { over_ms: Some(300) }
    );
}

#[test]
fn the_director_verbs_have_their_own_shapes() {
    assert_eq!(command("director show"), Ok(DirectorCommand::Show));
    assert_eq!(act("director pause"), DirectorAct::Pause);
    assert_eq!(act("director resume"), DirectorAct::Resume);
    assert_eq!(
        act("director auto camera off"),
        DirectorAct::Auto { on: false }
    );
    assert_eq!(
        act("director auto camera on"),
        DirectorAct::Auto { on: true }
    );
    assert_eq!(
        act("director advance 8000"),
        DirectorAct::Advance { by_ms: 8_000 }
    );
    assert_eq!(
        act("director end speaker finn"),
        DirectorAct::End {
            reason: Reason::Speaker,
            element: id("finn")
        }
    );
    assert_eq!(
        act("director start combat"),
        DirectorAct::Start {
            what: StartKind::Combat
        }
    );
}

#[test]
fn director_view_edits_what_the_director_is_shown() {
    let view = |line: &str| match act(line) {
        DirectorAct::View { edit } => edit,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        view("director view hide lurker"),
        ViewEdit::Hide {
            element: id("lurker")
        }
    );
    assert_eq!(
        view("director view unhide lurker"),
        ViewEdit::Unhide {
            element: id("lurker")
        }
    );
    assert_eq!(
        view("director view hero finn"),
        ViewEdit::Hero {
            element: id("finn")
        }
    );
    assert_eq!(view("director view card"), ViewEdit::Card);
    assert_eq!(view("director view clear"), ViewEdit::Clear);
    assert_eq!(
        view("director view plate width=800 stage=400"),
        ViewEdit::Plate {
            width_px: Some(800),
            stage_px: Some(400)
        }
    );
    assert_eq!(
        view("director view plate"),
        ViewEdit::Plate {
            width_px: None,
            stage_px: None
        }
    );
    assert_eq!(
        view("director view collapsed on"),
        ViewEdit::Collapsed { on: true }
    );
    assert_eq!(
        view("director view reduced off"),
        ViewEdit::Reduced { on: false }
    );
    assert_eq!(
        view("director view beat result-card 3000 element=finn"),
        ViewEdit::Beat {
            reveal: false,
            hold_ms: 3_000,
            element: Some(id("finn"))
        }
    );
    assert_eq!(
        view("director view beat reveal-beat 1500"),
        ViewEdit::Beat {
            reveal: true,
            hold_ms: 1_500,
            element: None
        }
    );
}

#[test]
fn a_mistake_gets_the_usage_or_the_choices() {
    for (line, usage) in [
        ("camera", CAMERA_USAGE),
        ("camera focus", CAMERA_USAGE),
        ("camera pan finn", CAMERA_USAGE),
        ("director", DIRECTOR_USAGE),
        ("director show now", DIRECTOR_USAGE),
        ("director end speaker", DIRECTOR_USAGE),
        ("director view", VIEW_USAGE),
        ("director view hide", VIEW_USAGE),
    ] {
        assert_eq!(command(line), Err(CommandError::Usage(usage)), "{line}");
    }
    let Err(CommandError::BadChoice { key, allowed, .. }) =
        command("camera focus finn reason=zoom")
    else {
        panic!("a reason that is not in the table is refused");
    };
    assert_eq!(key, "reason");
    for reason in ["hit", "speaker", "requested"] {
        assert!(allowed.contains(reason), "{allowed}");
    }
    assert!(matches!(
        command("camera focus finn zoom=big"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        command("camera focus finn volume=3"),
        Err(CommandError::BadChoice { .. })
    ));
    assert_eq!(
        command("camera focus finn zoom=1 zoom=2"),
        Err(CommandError::DuplicateKey("zoom".into()))
    );
    assert!(matches!(
        command("director auto camera maybe"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        command("director end wide finn"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        command("director start battle"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        command("director advance soon"),
        Err(CommandError::BadChoice { .. })
    ));
    // The classes that have no stage yet are refused with what is built.
    let Err(CommandError::BadChoice { allowed, .. }) = command("director auto slide on") else {
        panic!("the slide class is not built");
    };
    assert!(allowed.contains("camera"), "{allowed}");
}

#[test]
fn the_verbs_work_in_a_script_and_the_help_lists_them() {
    let words = split_words("camera focus finn reason=speaker").expect("splits");
    assert!(matches!(
        parse_script_line(&words),
        Ok(Command::Director(DirectorCommand::Act(
            DirectorAct::Focus { .. }
        )))
    ));
    let help = usage_text();
    for text in [
        "camera focus <element>",
        "director show",
        "director view hide",
        "interrupts a higher one",
        "driven=true",
    ] {
        assert!(help.contains(text), "{text}");
    }
}

#[tokio::test]
async fn camera_focus_moves_the_camera_with_the_dev_verbs_numbers_and_writes_a_reasoned_line() {
    let running = with_three_avatars().await;
    let answer = said(&running, "camera focus finn reason=speaker").await;
    assert!(
        answer.contains(
            "CAMERA focus element=finn zoom=200 over=1200 reason=speaker serial=1 driven=true"
        ),
        "{answer}"
    );
    let camera = camera_of(&running).await.expect("a camera");
    assert_eq!(
        (
            camera.target.as_deref(),
            camera.zoom_percent,
            camera.duration_ms,
            camera.serial
        ),
        (Some("finn"), 200, 1_200, 1)
    );
    assert_eq!(
        lines_starting(&running, "CAMERA focus").await,
        ["CAMERA focus element=finn zoom=200 over=1200 reason=speaker serial=1 driven=true"]
    );
    assert_eq!(
        lines_starting(&running, "PLAYGROUND camera").await.len(),
        1,
        "the playground's own audit line is written by the same change"
    );
}

#[tokio::test]
async fn a_second_focus_inside_the_spacing_waits_then_pans_on_without_pulling_out() {
    let running = with_three_avatars().await;
    said(&running, "camera focus finn reason=speaker").await;
    let answer = said(&running, "camera focus sigrun reason=speaker").await;
    assert!(answer.contains("CAMERA queued"), "{answer}");
    assert_eq!(
        camera_of(&running)
            .await
            .expect("a camera")
            .target
            .as_deref(),
        Some("finn")
    );
    said(&running, "director advance 800").await;
    let camera = camera_of(&running).await.expect("a camera");
    assert_eq!(
        (camera.target.as_deref(), camera.serial),
        (Some("sigrun"), 2)
    );
    assert!(lines_starting(&running, "CAMERA wide").await.is_empty());
}

#[tokio::test]
async fn a_creature_that_becomes_hidden_is_pulled_back_and_cannot_be_focused_while_hidden() {
    let running = with_three_avatars().await;
    said(&running, "camera focus lurker reason=speaker").await;
    assert!(camera_of(&running).await.is_some());
    said(&running, "director view hide lurker").await;
    assert_eq!(camera_of(&running).await.expect("a camera").target, None);
    assert_eq!(
        lines_starting(&running, "CAMERA wide").await,
        ["CAMERA wide reason=target_hidden serial=2 over=1200"]
    );
    said(&running, "director advance 1000").await;
    let answer = said(&running, "camera focus lurker reason=speaker").await;
    assert!(
        answer.contains("CAMERA dropped reason=target_hidden direction=speaker element=lurker"),
        "{answer}"
    );
    assert_eq!(camera_of(&running).await.expect("a camera").target, None);
}
#[tokio::test]
async fn a_name_that_is_not_on_the_stage_and_a_number_out_of_range_are_refused() {
    let running = with_three_avatars().await;
    let answer = said(&running, "camera focus nobody").await;
    assert!(answer.contains("reason=unknown_target"), "{answer}");
    assert_eq!(camera_of(&running).await, None);
    for (line, cause) in [
        (
            "camera focus finn zoom=99",
            DecisionError::ZoomOutOfRange { zoom_percent: 99 },
        ),
        (
            "camera focus finn over=100",
            DecisionError::FocusMoveOutOfRange { over_ms: 100 },
        ),
        (
            "camera focus finn hold=0",
            DecisionError::HoldOutOfRange { hold_ms: 0 },
        ),
    ] {
        let reply: DirectorReply<ActReply> =
            dev_client::post_json(running.port, path::ACT, &act(line))
                .await
                .expect("answers");
        let DevReply::Rejected { error, .. } = reply else {
            panic!("'{line}' must be refused");
        };
        assert_eq!(error, DirectorError::Refused { cause }, "{line}");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{line}");
        assert!(
            run(&running, line).await.is_err(),
            "the command line fails too: {line}"
        );
    }
    let Err(DevError::Command(CommandError::BadChoice { key, .. })) =
        run(&running, "camera focus Finn").await
    else {
        panic!("a name that is not an element name is refused before it is sent");
    };
    assert_eq!(key, "element");
    assert_eq!(camera_of(&running).await, None);
}

#[tokio::test]
async fn a_body_that_is_not_a_verb_or_names_no_element_is_refused_as_an_invalid_body() {
    let running = with_three_avatars().await;
    for body in [
        r#"{"verb":"dance"}"#,
        r#"{"verb":"focus","element":"Finn"}"#,
        r#"{"verb":"focus","element":"finn","reason":"zoom"}"#,
        r#"{"verb":"focus","element":"finn","zoom":200}"#,
    ] {
        let response = post_json(running.port, path::ACT, body)
            .await
            .expect("answers");
        assert_eq!(response.status, 400, "{body}");
        let reply: DirectorReply<ActReply> =
            serde_json::from_str(&response.body).expect("the envelope");
        assert!(
            matches!(
                reply,
                DevReply::Rejected {
                    error: DirectorError::InvalidBody(BodyError {
                        cause: BodyCause::Data,
                        ..
                    }),
                    ..
                }
            ),
            "{body}: {}",
            response.body
        );
    }
    assert_eq!(camera_of(&running).await, None);
    assert_eq!(
        lines_starting(&running, "DIRECTOR refused verb=act error=invalid_body")
            .await
            .len(),
        4,
        "one refusal line for each refused body"
    );
}

#[tokio::test]
async fn director_show_lists_what_was_queued_applied_and_dropped_and_matches_the_log() {
    let running = with_three_avatars().await;
    said(&running, "director view hide lurker").await;
    said(&running, "camera focus finn reason=speaker").await;
    said(&running, "camera focus lurker reason=hit").await;
    said(&running, "camera focus sigrun reason=requested").await;
    let text = said(&running, "director show").await;
    assert!(text.contains("holding: finn for speaker"), "{text}");
    assert!(text.contains("hidden lurker"), "{text}");
    assert!(
        text.contains("directions: 1 queued, 1 applied, 2 dropped"),
        "{text}"
    );
    let written = lines_starting(&running, "CAMERA").await;
    assert!(!written.is_empty());
    for line in &written {
        assert!(
            text.contains(line.as_str()),
            "{line} is in the log and not in show:\n{text}"
        );
    }
}

#[tokio::test]
async fn the_shipped_director_tour_runs_and_ends_with_the_camera_pulled_back() {
    let running = with_three_avatars().await;
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let path = directory.path().join("tour.txt");
    std::fs::write(&path, TOUR).expect("writes the script");
    let words = ["script".to_owned(), path.to_string_lossy().into_owned()];
    let answer = run_dev(running.port, &words).await.expect("runs");
    assert!(answer.ends_with("all ok"), "{answer}");
    assert_eq!(camera_of(&running).await.expect("a camera").target, None);
    let focus = lines_starting(&running, "CAMERA focus").await;
    let wide = lines_starting(&running, "CAMERA wide").await;
    assert!(focus.len() >= 5 && wide.len() >= 3, "{focus:?} {wide:?}");
    assert!(
        wide.iter()
            .any(|line| line.contains("reason=target_hidden")),
        "{wide:?}"
    );
    assert!(
        lines_starting(&running, "CAMERA dropped reason=target_hidden")
            .await
            .iter()
            .any(|line| line.contains("element=lurker")),
        "the hidden lurker was refused"
    );
}
