//! The dev command line of the owner actor: the H.16 verbs the host can serve now.
//!
//! It owns the grammar and usage text of `input`, `viable`, `dump`, `force`, `projection show`,
//! `queue`, `describe` and `inspect` (and the broker's `broker queues`, `broker ledger` and
//! `broker request`), and running one against a host on a port. The event and settings verbs
//! (`history`, `trace`, `replay`, `config`) have their grammar in `cli/observe`, and the verbs of
//! the first playable path theirs in `cli/play`; both are dispatched from here. Each verb has the
//! shape of the canvas commands: words in, one typed `EngineCommand` out, a usage line on a
//! mistake (`CommandError::Usage`), and a numbered answer. They work from the repl and from a
//! script like any other command, because they are parsed by the same `command::parse`.
//!
//! The registry-generated `describe` and every list read from the owner; nothing here knows what
//! a machine is. Hosted builds omit `force` and the broker's `request` (RES43, H.16): they are
//! the operator tools of a local build, and their routes are the only way in.

mod observe;
pub mod play;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::adapters::dev_check::{
    RULES_CHECK_USAGE, RulesCheckRequest, parse as parse_rules_check, run as run_rules_check,
};
use crate::command::CommandError;
use crate::dev::DevError;
use crate::dev_client;

use self::observe::{
    CONFIG_USAGE, HISTORY_USAGE, REPLAY_USAGE, TRACE_USAGE, parse_config, parse_history,
    parse_replay, parse_trace, run_config, run_history, run_replay, run_trace,
};
use self::play::{
    LOBBY_USAGE, PLAY_USAGE, PlayCommand, SCENE_START_USAGE, WIRE_USAGE, parse as parse_play,
    run as run_play,
};
use df_table::identity::SeatId;
use df_table::inspect::InspectAudience;

use super::format::{
    format_broker_ledger, format_broker_queues, format_chaos, format_chaos_standing, format_clock,
    format_description, format_dice, format_force, format_health, format_inspect, format_queue,
    format_requested, format_served, format_submit, format_viable, format_view,
};
use super::report::reach::DumpReport;
use super::report::{ChaosReport, EngineDump, EngineReply};
use super::request::{
    AudienceSpec, BrokerRequest, ChaosRequest, ClockControl, CommitFault, ConfigControl,
    DiceControl, ForceRequest, HistoryRequest, InputBody, InputRequest, InspectName,
    ProposalSource, QueueControl, ReplayRequest, TraceOperation, inspect_audience_word,
};
use super::routes::{AUDIENCE_PARAMETER, MACHINE_PARAMETER, SEAT_PARAMETER, path};

pub const INPUT_USAGE: &str = "input <seat> <text> | input <seat> --tap <action> | --say <text> \
    | --private <seat> <text> | --pause | input engine --timer <name> \
    | --proposal <origin> <proposer>  (any: --op N, --view N)";
pub const VIABLE_USAGE: &str = "viable <seat>";
pub const DUMP_USAGE: &str = "dump [table | <machine>]";
pub const FORCE_USAGE: &str = "force <machine> <state> [<field>=<value> ...]";
pub const PROJECTION_USAGE: &str = "projection show <tv|seat-number>";
pub const QUEUE_USAGE: &str = "queue show | hold | release | step [N]";
pub const DESCRIBE_USAGE: &str = "describe";
pub const BROKER_USAGE: &str = "broker queues | ledger | request <purpose>";
pub const COST_USAGE: &str = "cost";
pub const HEALTH_USAGE: &str = "health";
pub const CLOCK_USAGE: &str = "clock now | advance <ms> | freeze | resume";
pub const DICE_USAGE: &str = "dice seed <number> | force <value> [<value> ...]";
pub const CHAOS_USAGE: &str = "chaos | chaos commit <unavailable|refused|deadline> [count] \
    | chaos interpreter <down|up>";
pub const INSPECT_USAGE: &str = "inspect <table|machines|pending|rulings|requests|results|pipeline\
    |queue|registry|ports|settings> [--host | --seat N]";

/// The verbs this module parses, for the dispatch in `command::parse`.
pub const VERBS: [&str; 22] = [
    "input",
    "viable",
    "dump",
    "inspect",
    "force",
    "projection",
    "queue",
    "describe",
    "broker",
    "cost",
    "health",
    "clock",
    "dice",
    "chaos",
    "rules",
    "history",
    "trace",
    "replay",
    "config",
    "lobby",
    "wire",
    "play",
];

/// How many commits `chaos commit` makes fail when no count is given.
pub const DEFAULT_CHAOS_COUNT: u32 = 1;

/// How many inputs `queue step` serves when no number is given.
pub const DEFAULT_STEP_COUNT: usize = 1;

/// A command for the owner actor.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineCommand {
    Input(InputRequest),
    Viable {
        seat: u64,
    },
    Dump,
    /// `dump table` or `dump <machine>`: the word as typed, checked by the owner.
    DumpNamed {
        target: String,
    },
    Inspect {
        report: InspectName,
        audience: InspectAudience,
    },
    Force(ForceRequest),
    ProjectionShow(AudienceSpec),
    QueueShow,
    Queue(QueueControl),
    Describe,
    BrokerQueues,
    BrokerLedger,
    BrokerRequest {
        purpose: String,
    },
    Health,
    Clock(ClockControl),
    Dice(DiceControl),
    /// `chaos` with no words: the faults in force.
    ChaosShow,
    Chaos(ChaosRequest),
    RulesCheck(RulesCheckRequest),
    History(HistoryRequest),
    Trace {
        operation: TraceOperation,
    },
    Replay(ReplayRequest),
    Config(ConfigControl),
    /// A verb of the first playable path: `lobby`, `scene start`, `wire` or `play`.
    Play(PlayCommand),
}

/// Every verb's usage line and the lines of its help, in the order `describe` and `help` list
/// them.
const VERB_HELP: [(&str, &[&str]); 23] = [
    (
        INPUT_USAGE,
        &[
            "send an input as a seat or the engine through the real owner queue; a line is",
            "interpreted into one of the seat's choices (the dev stand-in reads a line that is \
             exactly",
            "one of them); --op N sends the same operation id again to see a duplicate",
        ],
    ),
    (
        VIABLE_USAGE,
        &["what the active machine would accept from the seat now"],
    ),
    (
        DUMP_USAGE,
        &[
            "the committed table state and the active machine's saved state, as JSON; `table` \
             prints",
            "the table's shared facts alone, and a machine's name every place it holds with its \
             saved state",
        ],
    ),
    (
        INSPECT_USAGE,
        &[
            "one module's report of what it holds, as the operator (the default), the host or one \
             seat",
            "may see it; a hidden value says who could see it. `pending` and `rulings` are the \
             books",
        ],
    ),
    (
        FORCE_USAGE,
        &[
            "put the table in a machine and state (a driven change for local builds); the \
             fields are",
            "the machine's saved state, for a state it does not start in (phase=started scene=1)",
        ],
    ),
    (
        PROJECTION_USAGE,
        &["what the shared screen or one seat is shown now"],
    ),
    (
        QUEUE_USAGE,
        &[
            "the lanes in the order they are served, duplicates and refusals, recent turns and",
            "commits; hold admits input without serving it, step serves one batch, release \
             serves all",
        ],
    ),
    (
        DESCRIBE_USAGE,
        &["the registered machines, lanes, ports and verbs, generated from the registry"],
    ),
    (
        BROKER_USAGE,
        &[
            "the resource broker's queues and what is running, what has been spent (recorded,",
            "never limited), or a dev request for a purpose",
        ],
    ),
    (
        COST_USAGE,
        &["what each purpose has cost so far: the broker's ledger, which records and never limits"],
    ),
    (
        HEALTH_USAGE,
        &["whether the owner is serving, what waits, how commits have gone"],
    ),
    (
        CLOCK_USAGE,
        &["read the owner's clock, move it forward, stop it or let it run (a driven change)"],
    ),
    (
        DICE_USAGE,
        &["restart the seeded dice or queue the values rolled next (a driven change)"],
    ),
    (
        CHAOS_USAGE,
        &[
            "the faults in force, or make the next commits fail, or take the interpreter down, \
             to see",
            "the fallbacks",
        ],
    ),
    (
        RULES_CHECK_USAGE,
        &[
            "run one real skill check through the df-rules adapter with a recorded draw (needs",
            "DF_RULES_BACKEND=df-rules; the DC is 15 when not given)",
        ],
    ),
    (
        HISTORY_USAGE,
        &[
            "the newest events the host kept, oldest first, each with its number, class (A is \
             always",
            "on, B is verbose detail) and line, then the event stream's health; --area is an audit",
            "area such as INPUT, TURN, COMMIT or ENGINE",
        ],
    ),
    (
        TRACE_USAGE,
        &[
            "every event the host kept for one input, with what the queue did with it; the \
             operation",
            "is the number --op took or the 32 digits the answers print",
        ],
    ),
    (
        REPLAY_USAGE,
        &[
            "replay a recorded turn log and compare it with the record; not built yet (it needs \
             C7's",
            "replay), so it says so",
        ],
    ),
    (
        CONFIG_USAGE,
        &["the settings in force with their specs, or change one that the next input will use"],
    ),
    (
        LOBBY_USAGE,
        &[
            "the lobby as a seat's phone drives it: each action is the tap its button sends, \
             through",
            "the real owner queue, answered with the typed outcome; `show` prints who sits and who \
             is ready",
        ],
    ),
    (
        SCENE_START_USAGE,
        &[
            "ready every seat at the table that is not ready, then start the lobby as the first \
             seat",
            "(or the one named); the lobby hands the table to the story scene",
        ],
    ),
    (
        WIRE_USAGE,
        &[
            "what a TV or a phone is served as a MachineView (with its protobuf size), or send a",
            "MachineIntent as a seat and read its receipt",
        ],
    ),
    (
        PLAY_USAGE,
        &[
            "write the engine's own shared screen and one seat's phone as a fixture the playground",
            "shows (`playground go h3-live`), or remove it",
        ],
    ),
];

/// The usage line of every verb, in the order `describe` and `help` list them.
pub fn verb_usages() -> Vec<String> {
    VERB_HELP
        .iter()
        .map(|(usage, _)| (*usage).to_owned())
        .collect()
}

/// The help text of these verbs, to follow the canvas commands' in `help`.
pub fn help_text() -> String {
    VERB_HELP
        .iter()
        .map(|(usage, lines)| format!("\n  {usage}\n      {}\n", lines.join("\n      ")))
        .collect()
}

/// A whole number from 0 the command names as `what`.
pub(super) fn whole_number(what: &'static str, text: &str) -> Result<u64, CommandError> {
    text.parse::<u64>().map_err(|_| CommandError::BadCount {
        what,
        text: text.to_owned(),
    })
}

fn without_words(
    words: &[String],
    text: &'static str,
    command: EngineCommand,
) -> Result<EngineCommand, CommandError> {
    if words.is_empty() {
        Ok(command)
    } else {
        Err(CommandError::Usage(text))
    }
}

/// Parses one engine verb. `verb` is one of `VERBS`.
pub fn parse(verb: &str, words: &[String]) -> Result<EngineCommand, CommandError> {
    match verb {
        "input" => input(words),
        "viable" => match words {
            [seat] => Ok(EngineCommand::Viable {
                seat: whole_number("the seat", seat)?,
            }),
            _ => Err(CommandError::Usage(VIABLE_USAGE)),
        },
        "dump" => match words {
            [] => Ok(EngineCommand::Dump),
            [target] => Ok(EngineCommand::DumpNamed {
                target: query_word(target)?,
            }),
            _ => Err(CommandError::Usage(DUMP_USAGE)),
        },
        "inspect" => inspect(words),
        "force" => force(words),
        "projection" => projection(words),
        "queue" => queue(words),
        "describe" => without_words(words, DESCRIBE_USAGE, EngineCommand::Describe),
        "broker" => broker(words),
        "cost" => without_words(words, COST_USAGE, EngineCommand::BrokerLedger),
        "health" => without_words(words, HEALTH_USAGE, EngineCommand::Health),
        "clock" => clock(words),
        "dice" => dice(words),
        "chaos" => chaos(words),
        "rules" => parse_rules_check(words).map(EngineCommand::RulesCheck),
        "history" => parse_history(words).map(EngineCommand::History),
        "trace" => parse_trace(words).map(|operation| EngineCommand::Trace { operation }),
        "replay" => parse_replay(words).map(EngineCommand::Replay),
        "config" => parse_config(words).map(EngineCommand::Config),
        "lobby" | "wire" | "play" => parse_play(verb, words).map(EngineCommand::Play),
        other => Err(CommandError::Unknown(other.to_owned())),
    }
}

/// The words of an `input` command once `--op` and `--view` are taken out.
struct InputWords {
    rest: Vec<String>,
    operation: Option<u64>,
    view: Option<u64>,
}

/// Takes `--op N` and `--view N` out of the words, wherever they are.
fn options(words: &[String]) -> Result<InputWords, CommandError> {
    let mut taken = InputWords {
        rest: Vec::new(),
        operation: None,
        view: None,
    };
    let mut remaining = words.iter();
    while let Some(word) = remaining.next() {
        let (slot, what) = match word.as_str() {
            "--op" => (&mut taken.operation, "--op"),
            "--view" => (&mut taken.view, "--view"),
            _ => {
                taken.rest.push(word.clone());
                continue;
            }
        };
        let value = remaining.next().ok_or(CommandError::Usage(INPUT_USAGE))?;
        if slot.replace(whole_number(what_of(what), value)?).is_some() {
            return Err(CommandError::DuplicateKey(what.to_owned()));
        }
    }
    Ok(taken)
}

fn what_of(flag: &str) -> &'static str {
    if flag == "--op" {
        "the operation id"
    } else {
        "the view revision"
    }
}

fn input(words: &[String]) -> Result<EngineCommand, CommandError> {
    let [target, rest @ ..] = words else {
        return Err(CommandError::Usage(INPUT_USAGE));
    };
    let InputWords {
        rest,
        operation,
        view,
    } = options(rest)?;
    let body = if target == "engine" {
        engine_body(&rest)?
    } else {
        seat_body(whole_number("the seat", target)?, &rest)?
    };
    Ok(EngineCommand::Input(InputRequest {
        input: body,
        operation,
        operation_id: None,
        view,
        machine: None,
    }))
}

fn seat_body(seat: u64, words: &[String]) -> Result<InputBody, CommandError> {
    let texts = |from: &[String]| from.join(" ");
    match words {
        [] => Err(CommandError::Usage(INPUT_USAGE)),
        [flag, action] if flag == "--tap" => Ok(InputBody::Tap {
            seat,
            action: action.clone(),
        }),
        [flag] if flag == "--pause" => Ok(InputBody::Pause { seat }),
        [flag, line @ ..] if flag == "--say" && !line.is_empty() => Ok(InputBody::Say {
            seat,
            text: texts(line),
        }),
        [flag, to, line @ ..] if flag == "--private" && !line.is_empty() => {
            Ok(InputBody::Private {
                seat,
                to: whole_number("the seat it is for", to)?,
                text: texts(line),
            })
        }
        [flag, ..] if flag.starts_with("--") => Err(CommandError::Usage(INPUT_USAGE)),
        line => Ok(InputBody::Text {
            seat,
            text: texts(line),
        }),
    }
}

fn engine_body(words: &[String]) -> Result<InputBody, CommandError> {
    match words {
        [flag, name] if flag == "--timer" => Ok(InputBody::Timer { name: name.clone() }),
        [flag, origin, proposer] if flag == "--proposal" => Ok(InputBody::Proposal {
            origin: ProposalSource::from_key(origin).ok_or_else(|| CommandError::BadChoice {
                key: "the proposal origin".to_owned(),
                text: origin.clone(),
                allowed: "npc, world, story, encounter, timer or machine".to_owned(),
            })?,
            proposer: proposer.clone(),
        }),
        _ => Err(CommandError::Usage(INPUT_USAGE)),
    }
}

/// The saved state a `force` names as `field=value` words: a value that reads as JSON (a number,
/// true, false, a list) is that, and any other is text, so `phase=started scene=1` is
/// `{"phase":"started","scene":1}`.
fn saved_state(pairs: &[String]) -> Result<serde_json::Value, CommandError> {
    let mut fields = serde_json::Map::new();
    for pair in pairs {
        let (name, value) = pair
            .split_once('=')
            .filter(|(name, _)| !name.is_empty())
            .ok_or_else(|| CommandError::BadPair(pair.clone()))?;
        let parsed = serde_json::from_str(value)
            .unwrap_or_else(|_| serde_json::Value::String(value.to_owned()));
        if fields.insert(name.to_owned(), parsed).is_some() {
            return Err(CommandError::DuplicateKey(name.to_owned()));
        }
    }
    Ok(serde_json::Value::Object(fields))
}

fn force(words: &[String]) -> Result<EngineCommand, CommandError> {
    let [machine, state, fields @ ..] = words else {
        return Err(CommandError::Usage(FORCE_USAGE));
    };
    let payload = if fields.is_empty() {
        None
    } else {
        Some(saved_state(fields)?)
    };
    Ok(EngineCommand::Force(ForceRequest {
        machine: machine.clone(),
        state: state.clone(),
        payload,
    }))
}

fn projection(words: &[String]) -> Result<EngineCommand, CommandError> {
    let [show, who] = words else {
        return Err(CommandError::Usage(PROJECTION_USAGE));
    };
    if show != "show" {
        return Err(CommandError::Usage(PROJECTION_USAGE));
    }
    Ok(EngineCommand::ProjectionShow(audience_word(who)?))
}

/// An audience as the verbs take it: `tv` or a seat number.
pub(super) fn audience_word(word: &str) -> Result<AudienceSpec, CommandError> {
    AudienceSpec::from_parameter(word).map_err(|_| CommandError::BadChoice {
        key: "the audience".to_owned(),
        text: word.to_owned(),
        allowed: "tv or a seat number".to_owned(),
    })
}

fn queue(words: &[String]) -> Result<EngineCommand, CommandError> {
    match words {
        [verb] if verb == "show" => Ok(EngineCommand::QueueShow),
        [verb] if verb == "hold" => Ok(EngineCommand::Queue(QueueControl::Hold)),
        [verb] if verb == "release" => Ok(EngineCommand::Queue(QueueControl::Release)),
        [verb] if verb == "step" => Ok(EngineCommand::Queue(QueueControl::Step {
            count: DEFAULT_STEP_COUNT,
        })),
        [verb, count] if verb == "step" => {
            let count = whole_number("the number of inputs", count)?;
            Ok(EngineCommand::Queue(QueueControl::Step {
                count: usize::try_from(count).unwrap_or(usize::MAX),
            }))
        }
        _ => Err(CommandError::Usage(QUEUE_USAGE)),
    }
}

fn broker(words: &[String]) -> Result<EngineCommand, CommandError> {
    match words {
        [verb] if verb == "queues" => Ok(EngineCommand::BrokerQueues),
        [verb] if verb == "ledger" => Ok(EngineCommand::BrokerLedger),
        [verb, purpose] if verb == "request" => Ok(EngineCommand::BrokerRequest {
            purpose: purpose.clone(),
        }),
        _ => Err(CommandError::Usage(BROKER_USAGE)),
    }
}

fn clock(words: &[String]) -> Result<EngineCommand, CommandError> {
    let control = match words {
        [verb] if verb == "now" => ClockControl::Now,
        [verb] if verb == "freeze" => ClockControl::Freeze,
        [verb] if verb == "resume" => ClockControl::Resume,
        [verb, ms] if verb == "advance" => ClockControl::Advance {
            ms: whole_number("the milliseconds", ms)?,
        },
        _ => return Err(CommandError::Usage(CLOCK_USAGE)),
    };
    Ok(EngineCommand::Clock(control))
}

fn dice(words: &[String]) -> Result<EngineCommand, CommandError> {
    let control = match words {
        [verb, seed] if verb == "seed" => DiceControl::Seed {
            seed: whole_number("the seed", seed)?,
        },
        [verb, values @ ..] if verb == "force" && !values.is_empty() => DiceControl::Force {
            values: values
                .iter()
                .map(|value| {
                    value.parse::<u16>().map_err(|_| CommandError::BadCount {
                        what: "a die value",
                        text: value.clone(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        },
        _ => return Err(CommandError::Usage(DICE_USAGE)),
    };
    Ok(EngineCommand::Dice(control))
}

/// `inspect <report> [--host | --seat N]`.
fn inspect(words: &[String]) -> Result<EngineCommand, CommandError> {
    let (name, audience) = match words {
        [name] => (name, InspectAudience::Operator),
        [name, flag] if flag == "--host" => (name, InspectAudience::Host),
        [name, flag, seat] if flag == "--seat" => (
            name,
            InspectAudience::Seat {
                seat: SeatId::new(whole_number("the seat", seat)?),
            },
        ),
        _ => return Err(CommandError::Usage(INSPECT_USAGE)),
    };
    let report = InspectName::from_key(name).ok_or_else(|| CommandError::BadChoice {
        key: "the report".to_owned(),
        text: name.clone(),
        allowed: InspectName::ALL
            .iter()
            .map(|name| name.key())
            .collect::<Vec<_>>()
            .join(", "),
    })?;
    Ok(EngineCommand::Inspect { report, audience })
}

fn chaos(words: &[String]) -> Result<EngineCommand, CommandError> {
    let request = match words {
        [] => return Ok(EngineCommand::ChaosShow),
        [dependency, fault, rest @ ..] if dependency == "commit" && rest.len() <= 1 => {
            let fault = CommitFault::from_key(fault).ok_or_else(|| CommandError::BadChoice {
                key: "the commit fault".to_owned(),
                text: fault.clone(),
                allowed: "unavailable, refused or deadline".to_owned(),
            })?;
            let count = match rest {
                [count] => u32::try_from(whole_number("the count", count)?).map_err(|_| {
                    CommandError::BadCount {
                        what: "the count",
                        text: count.clone(),
                    }
                })?,
                _ => DEFAULT_CHAOS_COUNT,
            };
            ChaosRequest::Commit { fault, count }
        }
        [dependency, state] if dependency == "interpreter" => match state.as_str() {
            "down" => ChaosRequest::Interpreter { down: true },
            "up" => ChaosRequest::Interpreter { down: false },
            other => {
                return Err(CommandError::BadChoice {
                    key: "the interpreter".to_owned(),
                    text: other.to_owned(),
                    allowed: "down or up".to_owned(),
                });
            }
        },
        _ => return Err(CommandError::Usage(CHAOS_USAGE)),
    };
    Ok(EngineCommand::Chaos(request))
}

/// The value of the owner's reply, or its refusal (the typed `EngineError`) as the command's
/// failure.
fn reply_value<T>(reply: EngineReply<T>) -> Result<T, DevError> {
    dev_client::reply_value(reply)
}

/// GETs an engine route and reads its value.
pub(super) async fn fetch<T: DeserializeOwned>(port: u16, address: &str) -> Result<T, DevError> {
    reply_value(dev_client::get_json(port, address).await?)
}

/// POSTs `body` to an engine route and reads its value.
pub(super) async fn send<T: DeserializeOwned>(
    port: u16,
    address: &str,
    body: &impl Serialize,
) -> Result<T, DevError> {
    reply_value(dev_client::post_json(port, address, body).await?)
}

/// A machine's name as a query value: the owner checks the name, and a word that would need
/// encoding is no machine's name, so it is refused here instead of being sent mangled.
fn query_word(word: &str) -> Result<String, CommandError> {
    if word
        .chars()
        .all(|letter| letter.is_ascii_alphanumeric() || matches!(letter, '_' | '-' | '.'))
    {
        Ok(word.to_owned())
    } else {
        Err(CommandError::BadChoice {
            key: "the machine".to_owned(),
            text: word.to_owned(),
            allowed: "table or a machine's name".to_owned(),
        })
    }
}

/// `address` with one query parameter. Every value the verbs put in a query is a number, `tv`,
/// hexadecimal digits, an audit area (checked by `parse_history`), an inspect audience or a
/// checked machine name (`query_word`), so none needs encoding.
pub(super) fn with_query(address: &str, parameter: &str, value: &str) -> String {
    format!("{address}?{parameter}={value}")
}

fn pretty<T: Serialize>(what: &'static str, value: &T) -> Result<String, DevError> {
    serde_json::to_string_pretty(value).map_err(|error| DevError::Unwritable {
        what,
        reason: error.to_string(),
    })
}

/// Runs one engine command against the host on `port` and returns the text to print.
pub async fn run(port: u16, command: EngineCommand) -> Result<String, DevError> {
    match command {
        EngineCommand::Input(request) => {
            Ok(format_submit(&send(port, path::INPUT, &request).await?))
        }
        EngineCommand::Viable { seat } => {
            let address = with_query(path::VIABLE, SEAT_PARAMETER, &seat.to_string());
            Ok(format_viable(&fetch(port, &address).await?))
        }
        EngineCommand::Dump => {
            let dump: EngineDump = fetch(port, path::DUMP).await?;
            pretty("dump", &dump)
        }
        EngineCommand::DumpNamed { target } => {
            let address = with_query(path::DUMP, MACHINE_PARAMETER, &target);
            let dump: DumpReport = fetch(port, &address).await?;
            pretty("dump", &dump)
        }
        EngineCommand::Inspect { report, audience } => {
            let address = with_query(
                &path::inspect_path(report.key()),
                AUDIENCE_PARAMETER,
                &inspect_audience_word(audience),
            );
            let report = fetch(port, &address).await?;
            format_inspect(&report).map_err(|error| DevError::Unwritable {
                what: "inspect",
                reason: error.to_string(),
            })
        }
        EngineCommand::Force(request) => {
            Ok(format_force(&send(port, path::FORCE, &request).await?))
        }
        EngineCommand::ProjectionShow(audience) => {
            let address = with_query(path::PROJECTION, AUDIENCE_PARAMETER, &audience.parameter());
            let view = fetch(port, &address).await?;
            format_view(&view).map_err(|error| DevError::Unwritable {
                what: "projection",
                reason: error.to_string(),
            })
        }
        EngineCommand::QueueShow => Ok(format_queue(&fetch(port, path::QUEUE).await?)),
        EngineCommand::Queue(control) => {
            Ok(format_served(&send(port, path::QUEUE, &control).await?))
        }
        EngineCommand::Describe => Ok(format_description(&fetch(port, path::DESCRIBE).await?)),
        EngineCommand::BrokerQueues => Ok(format_broker_queues(
            &fetch(port, path::BROKER_QUEUES).await?,
        )),
        EngineCommand::BrokerLedger => Ok(format_broker_ledger(
            &fetch(port, path::BROKER_LEDGER).await?,
        )),
        EngineCommand::BrokerRequest { purpose } => {
            let body = BrokerRequest { purpose };
            Ok(format_requested(
                &send(port, path::BROKER_REQUEST, &body).await?,
            ))
        }
        EngineCommand::Health => Ok(format_health(&fetch(port, path::HEALTH).await?)),
        EngineCommand::Clock(control) => {
            Ok(format_clock(&send(port, path::CLOCK, &control).await?))
        }
        EngineCommand::Dice(control) => Ok(format_dice(&send(port, path::DICE, &control).await?)),
        EngineCommand::ChaosShow => {
            let faults: Vec<ChaosReport> = fetch(port, path::CHAOS).await?;
            Ok(format_chaos_standing(&faults))
        }
        EngineCommand::Chaos(request) => {
            Ok(format_chaos(&send(port, path::CHAOS, &request).await?))
        }
        // The rules-check route answers its own typed refusal (`RulesCheckError`), not
        // `EngineError`, so the verb reads it through its own call.
        EngineCommand::RulesCheck(request) => run_rules_check(port, &request).await,
        EngineCommand::History(request) => run_history(port, request).await,
        EngineCommand::Trace { operation } => run_trace(port, operation).await,
        EngineCommand::Replay(request) => run_replay(port, request).await,
        EngineCommand::Config(control) => run_config(port, control).await,
        EngineCommand::Play(command) => run_play(port, command).await,
    }
}

#[cfg(test)]
mod observe_tests;
#[cfg(test)]
mod play_tests;
