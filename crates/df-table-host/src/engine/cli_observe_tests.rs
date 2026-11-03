use crate::command::{self, Command, CommandError, split_words};

use super::cli::{EngineCommand, VERBS, verb_usages};
use super::cli_observe::{CONFIG_USAGE, HISTORY_USAGE, REPLAY_USAGE, TRACE_USAGE};
use super::request::{ClassFilter, ConfigControl, HistoryRequest, ReplayRequest};
use super::verb_set::{HostSupport, VerbLine, not_built, verb_lines};
use df_table::inspect::Verb;

fn parsed(line: &str) -> Result<Command, CommandError> {
    command::parse(&split_words(line).expect("the line splits"))
}

fn engine(line: &str) -> EngineCommand {
    match parsed(line) {
        Ok(Command::Engine(command)) => command,
        other => panic!("'{line}' should be an engine command, got {other:?}"),
    }
}

#[test]
fn history_takes_a_limit_an_area_and_a_class_in_any_order() {
    assert_eq!(
        engine("history"),
        EngineCommand::History(HistoryRequest::default())
    );
    assert_eq!(
        engine("history --class b --area INPUT --limit 5"),
        EngineCommand::History(HistoryRequest {
            limit: Some(5),
            area: Some("INPUT".to_owned()),
            class: Some(ClassFilter::Verbose),
        })
    );
    assert_eq!(
        engine("history --class always"),
        EngineCommand::History(HistoryRequest {
            class: Some(ClassFilter::Always),
            ..HistoryRequest::default()
        })
    );
}

#[test]
fn history_refuses_what_it_cannot_read_and_says_how_to_ask() {
    assert_eq!(
        parsed("history --limit"),
        Err(CommandError::Usage(HISTORY_USAGE))
    );
    assert_eq!(
        parsed("history --sideways 1"),
        Err(CommandError::Usage(HISTORY_USAGE))
    );
    assert_eq!(
        parsed("history --limit 1 --limit 2"),
        Err(CommandError::DuplicateKey("--limit".to_owned()))
    );
    assert!(matches!(
        parsed("history --limit many"),
        Err(CommandError::BadCount { .. })
    ));
    assert!(matches!(
        parsed("history --class c"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        parsed("history --area IN&PUT"),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn trace_takes_exactly_one_operation() {
    assert_eq!(
        engine("trace 5"),
        EngineCommand::Trace {
            operation: "5".to_owned()
        }
    );
    assert_eq!(parsed("trace"), Err(CommandError::Usage(TRACE_USAGE)));
    assert_eq!(parsed("trace 1 2"), Err(CommandError::Usage(TRACE_USAGE)));
}

#[test]
fn replay_takes_a_log_then_until_and_diff() {
    assert_eq!(
        engine("replay session --until 40 --diff"),
        EngineCommand::Replay(ReplayRequest {
            log: "session".to_owned(),
            until: Some(40),
            diff: true,
        })
    );
    assert_eq!(
        engine("replay session"),
        EngineCommand::Replay(ReplayRequest {
            log: "session".to_owned(),
            until: None,
            diff: false,
        })
    );
    for bad in [
        "replay",
        "replay --diff",
        "replay session --until",
        "replay session --diff --diff",
        "replay session --faster",
    ] {
        assert!(parsed(bad).is_err(), "'{bad}' should be refused");
    }
    assert_eq!(parsed("replay"), Err(CommandError::Usage(REPLAY_USAGE)));
}

#[test]
fn config_shows_or_sets_one_setting_to_a_whole_number() {
    assert_eq!(
        engine("config show"),
        EngineCommand::Config(ConfigControl::Show)
    );
    assert_eq!(
        engine("config set held_nudge_s 45"),
        EngineCommand::Config(ConfigControl::Set {
            name: "held_nudge_s".to_owned(),
            value: 45,
        })
    );
    for bad in [
        "config",
        "config set",
        "config set held_nudge_s",
        "config show now",
    ] {
        assert_eq!(parsed(bad), Err(CommandError::Usage(CONFIG_USAGE)), "{bad}");
    }
    assert!(matches!(
        parsed("config set held_nudge_s soon"),
        Err(CommandError::BadCount { .. })
    ));
}

#[test]
fn the_new_verbs_are_registered_listed_and_allowed_in_a_script() {
    for verb in ["history", "trace", "replay", "config"] {
        assert!(VERBS.contains(&verb), "{verb} is not in VERBS");
    }
    let usages = verb_usages();
    for usage in [HISTORY_USAGE, TRACE_USAGE, REPLAY_USAGE, CONFIG_USAGE] {
        assert!(usages.iter().any(|listed| listed == usage), "{usage}");
    }
    assert!(
        command::parse_script_line(&split_words("trace 5").expect("splits")).is_ok(),
        "a script may trace"
    );
}

fn lines() -> Vec<VerbLine> {
    let specs: Vec<_> = Verb::ALL.iter().map(|verb| *verb.spec()).collect();
    verb_lines(&specs)
}

fn line<'a>(lines: &'a [VerbLine], words: &str) -> &'a VerbLine {
    lines
        .iter()
        .find(|line| line.words == words)
        .unwrap_or_else(|| panic!("C7's set has no verb '{words}'"))
}

#[test]
fn the_verb_set_is_c7s_with_what_the_host_has_of_each_verb() {
    let lines = lines();

    assert_eq!(
        lines.len(),
        Verb::ALL.len(),
        "one line for every verb of the set"
    );
    for built in [
        "describe",
        "dump",
        "force",
        "input",
        "viable",
        "dice",
        "clock",
        "chaos",
        "health",
        "cost",
        "trace",
        "projection show",
        "queue",
        "broker",
    ] {
        assert_eq!(line(&lines, built).host, HostSupport::Built, "{built}");
    }
    assert_eq!(line(&lines, "replay").host, HostSupport::Seam);
    assert!(
        line(&lines, "replay")
            .needs
            .as_deref()
            .is_some_and(|needs| needs.contains("C7"))
    );
    for missing in [
        "fixture load",
        "sim run",
        "soak",
        "fuzz",
        "pending list",
        "pending show",
        "rulings list",
        "rulings show",
    ] {
        let verb = line(&lines, missing);
        assert_eq!(verb.host, HostSupport::NotBuilt, "{missing}");
        assert!(verb.needs.is_some(), "{missing} says what it waits for");
    }
}

#[test]
fn what_the_host_says_it_has_is_what_the_command_line_dispatches() {
    for line in lines() {
        let first_word = line.words.split(' ').next().expect("a verb has a word");
        let dispatched = VERBS.contains(&first_word);
        match line.host {
            HostSupport::Built | HostSupport::Seam => {
                assert!(
                    dispatched,
                    "{} is said to exist but '{first_word}' is not dispatched",
                    line.words
                );
            }
            HostSupport::NotBuilt => {
                assert!(
                    !dispatched,
                    "{} is said to be missing but '{first_word}' is dispatched",
                    line.words
                );
            }
        }
    }
}

#[test]
fn the_verbs_not_built_are_named_with_what_they_wait_for_and_the_family_verbs_follow() {
    let missing = not_built(&lines());

    assert!(missing.iter().all(|verb| !verb.needs.is_empty()));
    for expected in [
        "sim run",
        "soak",
        "fuzz",
        "fixture load",
        "pending list",
        "replay (seam only)",
        "camera focus",
        "combat choices",
    ] {
        assert!(
            missing.iter().any(|verb| verb.verb.contains(expected)),
            "{expected} is not listed"
        );
    }
    for built in ["history", "trace", "config", "input", "health"] {
        assert!(
            !missing.iter().any(|verb| verb.verb.starts_with(built)),
            "{built} is built and must not be listed as missing"
        );
    }
}
