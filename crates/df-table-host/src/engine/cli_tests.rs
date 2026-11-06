use crate::command::{self, Command, CommandError, split_words, usage_text};

use df_table::identity::SeatId;
use df_table::inspect::InspectAudience;

use super::cli::{
    BROKER_USAGE, CHAOS_USAGE, DEFAULT_STEP_COUNT, DUMP_USAGE, EngineCommand, FORCE_USAGE,
    INPUT_USAGE, INSPECT_USAGE, PROJECTION_USAGE, QUEUE_USAGE, VERBS, VIABLE_USAGE, verb_usages,
};
use super::request::{
    AudienceSpec, ForceRequest, InputBody, InputRequest, InspectName, ProposalSource, QueueControl,
};

fn words(line: &str) -> Vec<String> {
    split_words(line).expect("the line splits")
}

fn parsed(line: &str) -> Result<Command, CommandError> {
    command::parse(&words(line))
}

fn engine(line: &str) -> EngineCommand {
    match parsed(line) {
        Ok(Command::Engine(command)) => command,
        other => panic!("'{line}' should be an engine command, got {other:?}"),
    }
}

fn input(body: InputBody) -> EngineCommand {
    EngineCommand::Input(InputRequest {
        input: body,
        operation: None,
        operation_id: None,
        view: None,
        machine: None,
    })
}

#[test]
fn a_tap_a_line_a_spoken_line_a_private_line_and_a_pause_each_have_a_form() {
    assert_eq!(
        engine("input 1 --tap join"),
        input(InputBody::Tap {
            seat: 1,
            action: "join".to_owned()
        })
    );
    assert_eq!(
        engine("input 2 \"I pick the lock\""),
        input(InputBody::Text {
            seat: 2,
            text: "I pick the lock".to_owned()
        })
    );
    assert_eq!(
        engine("input 2 I pick the lock"),
        input(InputBody::Text {
            seat: 2,
            text: "I pick the lock".to_owned()
        })
    );
    assert_eq!(
        engine("input 3 --say \"hello there\""),
        input(InputBody::Say {
            seat: 3,
            text: "hello there".to_owned()
        })
    );
    assert_eq!(
        engine("input 3 --private 4 psst"),
        input(InputBody::Private {
            seat: 3,
            to: 4,
            text: "psst".to_owned()
        })
    );
    assert_eq!(
        engine("input 5 --pause"),
        input(InputBody::Pause { seat: 5 })
    );
}

#[test]
fn the_engine_can_send_a_timer_or_a_proposal() {
    assert_eq!(
        engine("input engine --timer idle_nudge"),
        input(InputBody::Timer {
            name: "idle_nudge".to_owned()
        })
    );
    assert_eq!(
        engine("input engine --proposal npc guard"),
        input(InputBody::Proposal {
            origin: ProposalSource::Npc,
            proposer: "guard".to_owned()
        })
    );
}

#[test]
fn an_operation_id_and_a_view_revision_can_come_anywhere_after_the_seat() {
    let EngineCommand::Input(request) = engine("input 1 --op 42 --tap join --view 7") else {
        panic!("an input");
    };
    assert_eq!((request.operation, request.view), (Some(42), Some(7)));
    assert!(matches!(request.input, InputBody::Tap { .. }));
}

#[test]
fn a_mistaken_input_command_answers_with_the_usage_line_or_the_bad_value() {
    assert_eq!(parsed("input"), Err(CommandError::Usage(INPUT_USAGE)));
    assert_eq!(parsed("input 1"), Err(CommandError::Usage(INPUT_USAGE)));
    assert_eq!(
        parsed("input 1 --tap"),
        Err(CommandError::Usage(INPUT_USAGE))
    );
    assert_eq!(
        parsed("input 1 --nothing"),
        Err(CommandError::Usage(INPUT_USAGE))
    );
    assert_eq!(
        parsed("input 1 --op"),
        Err(CommandError::Usage(INPUT_USAGE))
    );
    assert_eq!(
        parsed("input engine"),
        Err(CommandError::Usage(INPUT_USAGE))
    );
    assert!(matches!(
        parsed("input first --tap join"),
        Err(CommandError::BadCount { .. })
    ));
    assert!(matches!(
        parsed("input 1 --op nope --tap join"),
        Err(CommandError::BadCount { .. })
    ));
    assert_eq!(
        parsed("input 1 --op 1 --op 2 --tap join"),
        Err(CommandError::DuplicateKey("--op".to_owned()))
    );
}

#[test]
fn the_read_verbs_take_exactly_what_their_usage_says() {
    assert_eq!(engine("viable 3"), EngineCommand::Viable { seat: 3 });
    assert_eq!(parsed("viable"), Err(CommandError::Usage(VIABLE_USAGE)));
    assert_eq!(parsed("viable 1 2"), Err(CommandError::Usage(VIABLE_USAGE)));
    assert_eq!(engine("dump"), EngineCommand::Dump);
    assert_eq!(parsed("dump a b"), Err(CommandError::Usage(DUMP_USAGE)));
    assert_eq!(engine("describe"), EngineCommand::Describe);
    assert_eq!(
        engine("projection show tv"),
        EngineCommand::ProjectionShow(AudienceSpec::Tv)
    );
    assert_eq!(
        engine("projection show 4"),
        EngineCommand::ProjectionShow(AudienceSpec::Seat { seat: 4 })
    );
    assert_eq!(
        parsed("projection"),
        Err(CommandError::Usage(PROJECTION_USAGE))
    );
    assert_eq!(
        parsed("projection hide tv"),
        Err(CommandError::Usage(PROJECTION_USAGE))
    );
    assert!(matches!(
        parsed("projection show everyone"),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn force_takes_a_machine_a_state_and_optionally_the_saved_state_as_fields() {
    assert_eq!(
        engine("force lobby gathering"),
        EngineCommand::Force(ForceRequest {
            machine: "lobby".to_owned(),
            state: "gathering".to_owned(),
            payload: None
        })
    );
    let EngineCommand::Force(request) = engine("force lobby started phase=started scene=1") else {
        panic!("a force");
    };
    assert_eq!(
        request.payload,
        Some(serde_json::json!({"phase": "started", "scene": 1}))
    );
    assert_eq!(parsed("force lobby"), Err(CommandError::Usage(FORCE_USAGE)));
    assert!(matches!(
        parsed("force lobby started phase"),
        Err(CommandError::BadPair(_))
    ));
    assert_eq!(
        parsed("force lobby started phase=a phase=b"),
        Err(CommandError::DuplicateKey("phase".to_owned()))
    );
}

#[test]
fn queue_and_broker_verbs_have_their_forms() {
    assert_eq!(engine("queue show"), EngineCommand::QueueShow);
    assert_eq!(
        engine("queue hold"),
        EngineCommand::Queue(QueueControl::Hold)
    );
    assert_eq!(
        engine("queue release"),
        EngineCommand::Queue(QueueControl::Release)
    );
    assert_eq!(
        engine("queue step"),
        EngineCommand::Queue(QueueControl::Step {
            count: DEFAULT_STEP_COUNT
        })
    );
    assert_eq!(
        engine("queue step 5"),
        EngineCommand::Queue(QueueControl::Step { count: 5 })
    );
    assert_eq!(parsed("queue"), Err(CommandError::Usage(QUEUE_USAGE)));
    assert_eq!(
        parsed("queue sideways"),
        Err(CommandError::Usage(QUEUE_USAGE))
    );
    assert_eq!(engine("broker queues"), EngineCommand::BrokerQueues);
    assert_eq!(engine("broker ledger"), EngineCommand::BrokerLedger);
    assert_eq!(
        engine("broker request plate"),
        EngineCommand::BrokerRequest {
            purpose: "plate".to_owned()
        }
    );
    assert_eq!(parsed("broker"), Err(CommandError::Usage(BROKER_USAGE)));
}

#[test]
fn every_engine_verb_is_dispatched_listed_in_help_and_allowed_in_a_script() {
    let help = usage_text();
    for usage in verb_usages() {
        assert!(help.contains(&usage), "help is missing: {usage}");
    }
    for verb in VERBS {
        assert!(
            !matches!(parsed(verb), Err(CommandError::Unknown(_))),
            "{verb} is not dispatched"
        );
    }
    assert_eq!(
        command::parse_script_line(&words("input 1 --tap join"))
            .map(|command| matches!(command, Command::Engine(_))),
        Ok(true)
    );
}

#[test]
fn health_cost_clock_dice_and_chaos_have_their_forms_and_refuse_the_rest() {
    use super::request::{ChaosRequest, ClockControl, CommitFault, DiceControl};

    assert_eq!(engine("health"), EngineCommand::Health);
    assert_eq!(engine("cost"), EngineCommand::BrokerLedger);
    assert_eq!(engine("clock now"), EngineCommand::Clock(ClockControl::Now));
    assert_eq!(
        engine("clock advance 250"),
        EngineCommand::Clock(ClockControl::Advance { ms: 250 })
    );
    assert_eq!(
        engine("clock freeze"),
        EngineCommand::Clock(ClockControl::Freeze)
    );
    assert_eq!(
        engine("clock resume"),
        EngineCommand::Clock(ClockControl::Resume)
    );
    assert_eq!(
        engine("dice seed 12"),
        EngineCommand::Dice(DiceControl::Seed { seed: 12 })
    );
    assert_eq!(
        engine("dice force 20 1"),
        EngineCommand::Dice(DiceControl::Force {
            values: vec![20, 1]
        })
    );
    assert_eq!(
        engine("chaos commit deadline 3"),
        EngineCommand::Chaos(ChaosRequest::Commit {
            fault: CommitFault::Deadline,
            count: 3
        })
    );
    assert_eq!(
        engine("chaos interpreter down"),
        EngineCommand::Chaos(ChaosRequest::Interpreter { down: true })
    );
    for bad in [
        "health now",
        "cost 1",
        "clock",
        "clock advance",
        "clock advance soon",
        "dice",
        "dice force",
        "dice force x",
        "chaos commit",
        "chaos commit sideways",
        "chaos interpreter sideways",
    ] {
        assert!(
            parsed(bad).is_err(),
            "'{bad}' should be refused: {:?}",
            parsed(bad)
        );
    }
}

#[test]
fn dump_names_the_table_or_one_machine_and_refuses_a_word_no_machine_has() {
    assert_eq!(
        engine("dump table"),
        EngineCommand::DumpNamed {
            target: "table".to_owned()
        }
    );
    assert_eq!(
        engine("dump lobby"),
        EngineCommand::DumpNamed {
            target: "lobby".to_owned()
        }
    );
    assert!(matches!(
        parsed("dump lob&by"),
        Err(CommandError::BadChoice { .. })
    ));
    assert_eq!(
        parsed("dump lobby now"),
        Err(CommandError::Usage(DUMP_USAGE))
    );
    assert!(DUMP_USAGE.contains("table"), "{DUMP_USAGE}");
}

#[test]
fn inspect_takes_every_report_and_an_audience() {
    for report in InspectName::ALL {
        assert_eq!(
            engine(&format!("inspect {}", report.key())),
            EngineCommand::Inspect {
                report,
                audience: InspectAudience::Operator
            }
        );
        assert!(
            INSPECT_USAGE.contains(report.key()),
            "{INSPECT_USAGE} lacks {}",
            report.key()
        );
    }
    assert_eq!(
        engine("inspect pending --host"),
        EngineCommand::Inspect {
            report: InspectName::Pending,
            audience: InspectAudience::Host
        }
    );
    assert_eq!(
        engine("inspect pending --seat 2"),
        EngineCommand::Inspect {
            report: InspectName::Pending,
            audience: InspectAudience::Seat {
                seat: SeatId::new(2)
            }
        }
    );
    for bad in [
        "inspect",
        "inspect pending --seat",
        "inspect pending --guest",
        "inspect pending --host 2",
    ] {
        assert_eq!(
            parsed(bad),
            Err(CommandError::Usage(INSPECT_USAGE)),
            "{bad}"
        );
    }
    assert!(matches!(
        parsed("inspect pending --seat two"),
        Err(CommandError::BadCount { .. })
    ));
    assert!(matches!(
        parsed("inspect everything"),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn chaos_with_no_words_reads_the_faults_in_force() {
    assert_eq!(engine("chaos"), EngineCommand::ChaosShow);
    assert!(CHAOS_USAGE.starts_with("chaos | "), "{CHAOS_USAGE}");
    assert_eq!(
        parsed("chaos sideways"),
        Err(CommandError::Usage(CHAOS_USAGE))
    );
}
