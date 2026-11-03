//! The grammar of `camera focus|wide` and `director ...`: parsed, refused with the choices, and
//! listed in the help.

use crate::command::{Command, CommandError, parse, parse_script_line, split_words, usage_text};

use super::cli::{CAMERA_USAGE, DIRECTOR_USAGE, DirectorCommand, VIEW_USAGE};
use super::request::{DirectorAct, StartKind, ViewEdit};

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
            element: "finn".into(),
            reason: None,
            zoom: None,
            over: None,
            hold: None,
        }
    );
    assert_eq!(
        act("camera focus sigrun reason=hit zoom=250 over=1500 hold=4000"),
        DirectorAct::Focus {
            element: "sigrun".into(),
            reason: Some("hit".into()),
            zoom: Some(250),
            over: Some(1_500),
            hold: Some(4_000),
        }
    );
    assert_eq!(act("camera wide"), DirectorAct::Wide { over: None });
    assert_eq!(
        act("camera wide over=300"),
        DirectorAct::Wide { over: Some(300) }
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
        DirectorAct::Advance { ms: 8_000 }
    );
    assert_eq!(
        act("director end speaker finn"),
        DirectorAct::End {
            reason: "speaker".into(),
            element: "finn".into()
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
            element: "lurker".into()
        }
    );
    assert_eq!(
        view("director view unhide lurker"),
        ViewEdit::Unhide {
            element: "lurker".into()
        }
    );
    assert_eq!(
        view("director view hero finn"),
        ViewEdit::Hero {
            element: "finn".into()
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
            ms: 3_000,
            element: Some("finn".into())
        }
    );
    assert_eq!(
        view("director view beat reveal-beat 1500"),
        ViewEdit::Beat {
            reveal: true,
            ms: 1_500,
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
