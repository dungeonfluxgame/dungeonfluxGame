//! The dev command line of the owner actor: the H.16 verbs the host can serve now.
//!
//! It owns the grammar and usage text of `input`, `viable`, `dump`, `force`, `projection show`,
//! `queue` and `describe` (and the broker's `broker queues`, `broker ledger` and `broker request`),
//! and running one against a host on a port. Each verb has the shape of the canvas commands: words
//! in, one typed `EngineCommand` out, a usage line on a mistake (`CommandError::Usage`), and a
//! numbered answer. They work from the repl and from a script like any other command, because
//! they are parsed by the same `command::parse`.
//!
//! The registry-generated `describe` and every list read from the owner; nothing here knows what
//! a machine is. Hosted builds omit `force` and the broker's `request` (RES43, H.16): they are
//! the operator tools of a local build, and their routes are the only way in.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::client::{self, Response};
use crate::command::CommandError;
use crate::dev::DevError;
use crate::limits::MAXIMUM_ECHOED_ANSWER;

use super::format::{
    format_broker_ledger, format_broker_queues, format_chaos, format_clock, format_description,
    format_dice, format_force, format_health, format_queue, format_requested, format_served,
    format_submit, format_viable, format_view,
};
use super::report::{BrokerView, EngineReply};
use super::request::{
    AudienceSpec, ChaosRequest, ClockControl, CommitFault, DiceControl, ForceRequest, InputBody,
    InputRequest, QueueControl,
};
use super::routes::{AUDIENCE_PARAMETER, BrokerRequestBody, SEAT_PARAMETER, path};

pub const INPUT_USAGE: &str = "input <seat> <text> | input <seat> --tap <action> | --say <text> | --private <seat> <text> | --pause | input engine --timer <name> | --proposal <origin> <proposer>  (any: --op N, --view N)";
pub const VIABLE_USAGE: &str = "viable <seat>";
pub const DUMP_USAGE: &str = "dump";
pub const FORCE_USAGE: &str = "force <machine> <state> [<field>=<value> ...]";
pub const PROJECTION_USAGE: &str = "projection show <tv|seat-number>";
pub const QUEUE_USAGE: &str = "queue show | hold | release | step [N]";
pub const DESCRIBE_USAGE: &str = "describe";
pub const BROKER_USAGE: &str = "broker queues | ledger | request <purpose>";
pub const COST_USAGE: &str = "cost";
pub const HEALTH_USAGE: &str = "health";
pub const CLOCK_USAGE: &str = "clock now | advance <ms> | freeze | resume";
pub const DICE_USAGE: &str = "dice seed <number> | force <value> [<value> ...]";
pub const CHAOS_USAGE: &str =
    "chaos commit <unavailable|refused|deadline> [count] | chaos interpreter <down|up>";

/// The verbs this module parses, for the dispatch in `command::parse`.
pub const VERBS: [&str; 13] = [
    "input",
    "viable",
    "dump",
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
];

/// How many commits `chaos commit` makes fail when no count is given.
pub const DEFAULT_CHAOS_COUNT: u32 = 1;

/// How many inputs `queue step` serves when no number is given.
pub const DEFAULT_STEP_COUNT: usize = 1;

/// A command for the owner actor.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineCommand {
    Input(InputRequest),
    Viable { seat: u64 },
    Dump,
    Force(ForceRequest),
    ProjectionShow(AudienceSpec),
    QueueShow,
    Queue(QueueControl),
    Describe,
    BrokerQueues,
    BrokerLedger,
    BrokerRequest { purpose: String },
    Health,
    Clock(ClockControl),
    Dice(DiceControl),
    Chaos(ChaosRequest),
}

/// The usage line of every verb, in the order `describe` and `help` list them.
pub fn verb_usages() -> Vec<String> {
    [
        INPUT_USAGE,
        VIABLE_USAGE,
        DUMP_USAGE,
        FORCE_USAGE,
        PROJECTION_USAGE,
        QUEUE_USAGE,
        DESCRIBE_USAGE,
        BROKER_USAGE,
        COST_USAGE,
        HEALTH_USAGE,
        CLOCK_USAGE,
        DICE_USAGE,
        CHAOS_USAGE,
    ]
    .iter()
    .map(|usage| (*usage).to_owned())
    .collect()
}

/// The help text of these verbs, to follow the canvas commands' in `help`.
pub fn help_text() -> String {
    format!(
        "\n  {INPUT_USAGE}\n      send an input as a seat or the engine through the real owner queue; a line is\n      interpreted into one of the seat's choices (the dev stand-in reads a line that is exactly\n      one of them); --op N sends the same operation id again to see a duplicate\n\
         \n  {VIABLE_USAGE}\n      what the active machine would accept from the seat now\n\
         \n  {DUMP_USAGE}\n      the committed table state and the active machine's saved state, as JSON\n\
         \n  {FORCE_USAGE}\n      put the table in a machine and state (a driven change for local builds); the fields are\n      the machine's saved state, for a state it does not start in (phase=started scene=1)\n\
         \n  {PROJECTION_USAGE}\n      what the shared screen or one seat is shown now\n\
         \n  {QUEUE_USAGE}\n      the lanes in the order they are served, duplicates and refusals, recent turns and\n      commits; hold admits input without serving it, step serves one batch, release serves all\n\
         \n  {DESCRIBE_USAGE}\n      the registered machines, lanes, ports and verbs, generated from the registry\n\
         \n  {BROKER_USAGE}\n      the resource broker's queues and what is running, what has been spent (recorded,\n      never limited), or a dev request for a purpose\n\
         \n  {COST_USAGE}\n      what each purpose has cost so far: the broker's ledger, which records and never limits\n\
         \n  {HEALTH_USAGE}\n      whether the owner is serving, what waits, how commits have gone\n\
         \n  {CLOCK_USAGE}\n      read the owner's clock, move it forward, stop it or let it run (a driven change)\n\
         \n  {DICE_USAGE}\n      restart the seeded dice or queue the values rolled next (a driven change)\n\
         \n  {CHAOS_USAGE}\n      make the next commits fail, or take the interpreter down, to see the fallbacks\n"
    )
}

fn usage(text: &'static str) -> CommandError {
    CommandError::Usage(text)
}

fn whole_number(what: &'static str, text: &str) -> Result<u64, CommandError> {
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
        Err(usage(text))
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
            _ => Err(usage(VIABLE_USAGE)),
        },
        "dump" => without_words(words, DUMP_USAGE, EngineCommand::Dump),
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
        let value = remaining.next().ok_or_else(|| usage(INPUT_USAGE))?;
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
        return Err(usage(INPUT_USAGE));
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
        view,
    }))
}

fn seat_body(seat: u64, words: &[String]) -> Result<InputBody, CommandError> {
    let texts = |from: &[String]| from.join(" ");
    match words {
        [] => Err(usage(INPUT_USAGE)),
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
        [flag, ..] if flag.starts_with("--") => Err(usage(INPUT_USAGE)),
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
            origin: origin.clone(),
            proposer: proposer.clone(),
        }),
        _ => Err(usage(INPUT_USAGE)),
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
        return Err(usage(FORCE_USAGE));
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
        return Err(usage(PROJECTION_USAGE));
    };
    if show != "show" {
        return Err(usage(PROJECTION_USAGE));
    }
    let audience = match who.as_str() {
        "tv" => AudienceSpec::Tv,
        seat => AudienceSpec::Seat {
            seat: seat.parse::<u64>().map_err(|_| CommandError::BadChoice {
                key: "the audience".to_owned(),
                text: who.clone(),
                allowed: "tv or a seat number".to_owned(),
            })?,
        },
    };
    Ok(EngineCommand::ProjectionShow(audience))
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
        _ => Err(usage(QUEUE_USAGE)),
    }
}

fn broker(words: &[String]) -> Result<EngineCommand, CommandError> {
    match words {
        [verb] if verb == "queues" => Ok(EngineCommand::BrokerQueues),
        [verb] if verb == "ledger" => Ok(EngineCommand::BrokerLedger),
        [verb, purpose] if verb == "request" => Ok(EngineCommand::BrokerRequest {
            purpose: purpose.clone(),
        }),
        _ => Err(usage(BROKER_USAGE)),
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
        _ => return Err(usage(CLOCK_USAGE)),
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
        _ => return Err(usage(DICE_USAGE)),
    };
    Ok(EngineCommand::Dice(control))
}

fn chaos(words: &[String]) -> Result<EngineCommand, CommandError> {
    let request = match words {
        [dependency, fault, rest @ ..] if dependency == "commit" && rest.len() <= 1 => {
            let fault = match fault.as_str() {
                "unavailable" => CommitFault::Unavailable,
                "refused" => CommitFault::Refused,
                "deadline" => CommitFault::Deadline,
                other => {
                    return Err(CommandError::BadChoice {
                        key: "the commit fault".to_owned(),
                        text: other.to_owned(),
                        allowed: "unavailable, refused or deadline".to_owned(),
                    });
                }
            };
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
        _ => return Err(usage(CHAOS_USAGE)),
    };
    Ok(EngineCommand::Chaos(request))
}

fn unreadable(response: &Response, reason: String) -> DevError {
    DevError::UnexpectedAnswer {
        status: response.status,
        body: response.body.chars().take(MAXIMUM_ECHOED_ANSWER).collect(),
        reason: Some(reason),
    }
}

/// Reads an answer as the owner's reply: the value, or the refusal as a `DevError::Rejected`.
fn read<T: DeserializeOwned>(response: &Response) -> Result<T, DevError> {
    let reply: EngineReply<T> = serde_json::from_str(&response.body)
        .map_err(|error| unreadable(response, error.to_string()))?;
    match reply {
        EngineReply::Done { value } => Ok(value),
        EngineReply::Rejected { message, .. } => Err(DevError::Rejected(message)),
    }
}

async fn fetch<T: DeserializeOwned>(port: u16, address: &str) -> Result<T, DevError> {
    let response = client::get(port, address).await.map_err(DevError::Client)?;
    read(&response)
}

async fn send<T: DeserializeOwned>(
    port: u16,
    address: &str,
    body: &impl Serialize,
) -> Result<T, DevError> {
    let text = serde_json::to_string(body).map_err(|error| DevError::Unwritable {
        what: "request",
        reason: error.to_string(),
    })?;
    let response = client::post_json(port, address, &text)
        .await
        .map_err(DevError::Client)?;
    read(&response)
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
            let address = format!("{}?{SEAT_PARAMETER}={seat}", path::VIABLE);
            Ok(format_viable(&fetch(port, &address).await?))
        }
        EngineCommand::Dump => {
            let dump: super::report::EngineDump = fetch(port, path::DUMP).await?;
            pretty("dump", &dump)
        }
        EngineCommand::Force(request) => {
            Ok(format_force(&send(port, path::FORCE, &request).await?))
        }
        EngineCommand::ProjectionShow(audience) => {
            let who = match audience {
                AudienceSpec::Tv => "tv".to_owned(),
                AudienceSpec::Seat { seat } => seat.to_string(),
            };
            let address = format!("{}?{AUDIENCE_PARAMETER}={who}", path::PROJECTION);
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
        EngineCommand::BrokerQueues => match fetch(port, path::BROKER_QUEUES).await? {
            BrokerView::Queues(report) => Ok(format_broker_queues(&report)),
            BrokerView::Ledger(_) => Err(DevError::Rejected(
                "the host answered the queues with a ledger".to_owned(),
            )),
        },
        EngineCommand::BrokerLedger => match fetch(port, path::BROKER_LEDGER).await? {
            BrokerView::Ledger(report) => Ok(format_broker_ledger(&report)),
            BrokerView::Queues(_) => Err(DevError::Rejected(
                "the host answered the ledger with queues".to_owned(),
            )),
        },
        EngineCommand::BrokerRequest { purpose } => {
            let body = BrokerRequestBody { purpose };
            Ok(format_requested(
                &send(port, path::BROKER_REQUEST, &body).await?,
            ))
        }
        EngineCommand::Health => Ok(format_health(&fetch(port, path::HEALTH).await?)),
        EngineCommand::Clock(control) => {
            Ok(format_clock(&send(port, path::CLOCK, &control).await?))
        }
        EngineCommand::Dice(control) => Ok(format_dice(&send(port, path::DICE, &control).await?)),
        EngineCommand::Chaos(request) => {
            Ok(format_chaos(&send(port, path::CHAOS, &request).await?))
        }
    }
}
