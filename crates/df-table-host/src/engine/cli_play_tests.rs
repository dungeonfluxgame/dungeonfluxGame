use crate::command::{self, Command, CommandError, split_words};

use super::cli::{EngineCommand, VERBS, verb_usages};
use super::cli_play::{
    LOBBY_USAGE, LobbyCommand, PLAY_USAGE, PlayCommand, SCENE_START_USAGE, WIRE_USAGE,
};
use super::machine_io::WireIntentKind;
use super::request::AudienceSpec;

fn parsed(line: &str) -> Result<Command, CommandError> {
    command::parse(&split_words(line).expect("the line splits"))
}

fn play(line: &str) -> PlayCommand {
    match parsed(line) {
        Ok(Command::Engine(EngineCommand::Play(command))) => command,
        other => panic!("'{line}' should be a play command, got {other:?}"),
    }
}

#[test]
fn the_lobby_verbs_are_the_taps_a_phone_sends_and_refuse_the_rest() {
    assert_eq!(
        play("lobby join 3"),
        PlayCommand::Lobby(LobbyCommand::Act {
            action: "join",
            seat: 3
        })
    );
    for action in ["leave", "ready", "unready", "start"] {
        assert!(matches!(
            play(&format!("lobby {action} 1")),
            PlayCommand::Lobby(LobbyCommand::Act { .. })
        ));
    }
    assert_eq!(play("lobby show"), PlayCommand::Lobby(LobbyCommand::Show));
    for bad in ["lobby", "lobby join", "lobby dance 1", "lobby show 1"] {
        assert_eq!(parsed(bad), Err(CommandError::Usage(LOBBY_USAGE)), "{bad}");
    }
    assert!(matches!(
        parsed("lobby join first"),
        Err(CommandError::BadCount { .. })
    ));
}

#[test]
fn scene_start_takes_an_optional_seat() {
    assert_eq!(play("scene start"), PlayCommand::SceneStart { seat: None });
    assert_eq!(
        play("scene start 2"),
        PlayCommand::SceneStart { seat: Some(2) }
    );
    assert_eq!(
        parsed("scene start 1 2"),
        Err(CommandError::Usage(SCENE_START_USAGE))
    );
}

#[test]
fn the_wire_verbs_name_an_audience_or_an_intent_and_the_play_verbs_a_seat() {
    assert_eq!(
        play("wire view tv"),
        PlayCommand::WireView(AudienceSpec::Tv)
    );
    assert_eq!(
        play("wire view 2"),
        PlayCommand::WireView(AudienceSpec::Seat { seat: 2 })
    );
    let PlayCommand::WireSend(body) = play("wire send 1 --op 9 --tap ready") else {
        panic!("a wire send");
    };
    assert_eq!(body.seat, 1);
    assert_eq!(body.operation, Some(9));
    assert_eq!(
        body.what,
        WireIntentKind::Tap {
            offer: "ready".to_owned()
        }
    );
    let PlayCommand::WireSend(body) = play("wire send 1 --say hello there") else {
        panic!("a wire send");
    };
    assert_eq!(
        body.what,
        WireIntentKind::Say {
            text: "hello there".to_owned()
        }
    );
    assert!(matches!(
        play("wire send 2 --pause"),
        PlayCommand::WireSend(_)
    ));
    assert_eq!(parsed("wire"), Err(CommandError::Usage(WIRE_USAGE)));
    assert_eq!(parsed("wire send 1"), Err(CommandError::Usage(WIRE_USAGE)));
    assert_eq!(play("play publish"), PlayCommand::Publish { seat: None });
    assert_eq!(
        play("play publish 3"),
        PlayCommand::Publish { seat: Some(3) }
    );
    assert_eq!(play("play remove"), PlayCommand::Remove);
    assert_eq!(parsed("play"), Err(CommandError::Usage(PLAY_USAGE)));
}

#[test]
fn the_new_verbs_are_registered_listed_and_allowed_in_a_script() {
    for verb in ["lobby", "wire", "play"] {
        assert!(VERBS.contains(&verb), "{verb} is not in VERBS");
    }
    let usages = verb_usages();
    for usage in [LOBBY_USAGE, SCENE_START_USAGE, WIRE_USAGE, PLAY_USAGE] {
        assert!(usages.iter().any(|listed| listed == usage), "{usage}");
    }
    for line in ["lobby join 1", "scene start", "wire view tv", "play remove"] {
        assert!(
            command::parse_script_line(&split_words(line).expect("splits")).is_ok(),
            "a script may run '{line}'"
        );
    }
}
