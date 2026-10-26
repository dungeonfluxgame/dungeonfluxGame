//! Agent command line for the development control surface (DEVCTL-001).
//!
//! `df-transport-fixture dev ...` talks to a game server started with
//! `DF_GAMEPLAY_DEV_CONTROL=on`. It prints one JSON document per command on
//! standard output (compact by default, `--pretty` for people), writes errors
//! as JSON on standard error with a non-zero exit code, and never prompts.

use std::{ffi::OsString, time::Duration};

#[path = "dev_cli/dice.rs"]
mod dice;
#[path = "dev_cli/layout.rs"]
mod layout;
#[path = "dev_cli/place.rs"]
mod place;
#[path = "dev_cli/play.rs"]
mod play;
#[path = "dev_cli/roleplay.rs"]
mod roleplay;

use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::timeout,
};

/// Writes one line to standard output, ignoring a closed pipe: agents often
/// truncate output (`| head`), which must not turn a delivered document into a
/// crash. The exit code is decided before printing and is unaffected.
fn stdout_line(text: &str) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{text}");
}

/// Runs the `dev` command line; the process exit code.
pub async fn run(arguments: &[OsString]) -> i32 {
    match run_inner(arguments).await {
        CommandResult::Done => 0,
        CommandResult::Reported => 1,
        CommandResult::Transport(message) => {
            eprintln!("{}", json!({"error": message}));
            2
        }
    }
}

enum CommandResult {
    /// Success document already printed on stdout.
    Done,
    /// A domain failure document already printed on stdout (refused, timed
    /// out waiting, script stopped): still a non-zero exit.
    Reported,
    /// A client-side failure; the message is printed on stderr as JSON.
    Transport(String),
}

struct Client {
    host: String,
    port: u16,
    token: String,
    pretty: bool,
    timeout: Duration,
}

impl Client {
    fn print(&self, document: &Value, failed: bool) -> CommandResult {
        let status = document.get("status").and_then(Value::as_str).unwrap_or("");
        let failed = failed
            || matches!(
                status,
                "refused" | "unavailable" | "ambiguous" | "observation" | "error"
            );
        if self.pretty {
            stdout_line(
                &serde_json::to_string_pretty(document).unwrap_or_else(|_| document.to_string()),
            );
        } else {
            stdout_line(&document.to_string());
        }
        if failed {
            CommandResult::Reported
        } else {
            CommandResult::Done
        }
    }

    fn print_error(&self, message: impl Into<String>) -> CommandResult {
        eprintln!("{}", json!({"error": message.into()}));
        CommandResult::Reported
    }
}

async fn run_inner(arguments: &[OsString]) -> CommandResult {
    let words: Vec<String> = arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    // words[0] is `dev` when dispatched from main; accept a bare call too.
    let words: &[String] = match words.first().map(String::as_str) {
        Some("dev") => &words[1..],
        _ => &words[..],
    };
    let mut client = Client {
        host: String::new(),
        port: 0,
        token: String::new(),
        pretty: false,
        timeout: Duration::from_secs(60),
    };
    let mut positional: Vec<String> = Vec::new();
    let mut options: Vec<(String, String)> = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let word = &words[index];
        if word == "--pretty" {
            client.pretty = true;
        } else if matches!(word.as_str(), "--keep" | "--wait" | "--summary") {
            options.push((word.clone(), "true".to_owned()));
        } else if word == "--help" || word == "-h" {
            print_help();
            return CommandResult::Done;
        } else if word.starts_with("--") {
            // Any --flag value or --flag=value pair is collected; each command
            // reads the flags it understands (unknown ones are ignored).
            if let Some((name, value)) = word.split_once('=') {
                options.push((name.to_owned(), value.to_owned()));
            } else {
                index += 1;
                let value = words.get(index).cloned().unwrap_or_default();
                options.push((word.clone(), value));
            }
        } else {
            positional.push(word.clone());
        }
        index += 1;
    }
    let option = |name: &str| {
        options
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    let server = option("--server").or_else(|| std::env::var("DF_DEV_CONTROL_URL").ok());
    let token_file =
        option("--token-file").or_else(|| std::env::var("DF_DEV_CONTROL_TOKEN_FILE").ok());
    if let Some(seconds) = option("--timeout") {
        match seconds.parse::<u64>() {
            Ok(seconds) if (1..=600).contains(&seconds) => {
                client.timeout = Duration::from_secs(seconds);
            }
            _ => return CommandResult::Transport("--timeout must be 1 to 600 seconds".to_owned()),
        }
    }
    let command = positional.first().cloned().unwrap_or_default();
    if command.is_empty() || command == "help" {
        print_help();
        return CommandResult::Done;
    }
    if command == "script" {
        return run_script(&client, &positional, &options).await;
    }
    // `describe` without a server prints nothing: it is the server's live spec.
    let server = match server {
        Some(server) => server,
        None => {
            return CommandResult::Transport(
                "no server: pass --server <url> or set DF_DEV_CONTROL_URL (the server prints DEV CONTROL on startup)".to_owned(),
            );
        }
    };
    let (host, port) = match parse_server(&server) {
        Ok(endpoint) => endpoint,
        Err(message) => return CommandResult::Transport(message),
    };
    client.host = host;
    client.port = port;
    let token_file = match token_file {
        Some(path) => path,
        None => {
            return CommandResult::Transport(
                "no token file: pass --token-file <path> or set DF_DEV_CONTROL_TOKEN_FILE (the server prints its path)".to_owned(),
            );
        }
    };
    client.token = match read_token(&token_file) {
        Ok(token) => token,
        Err(message) => return CommandResult::Transport(message),
    };
    dispatch(&client, &positional, &options).await
}

fn print_help() {
    roleplay::help();
    stdout_line(
        "dev: control a development game server (DF_GAMEPLAY_DEV_CONTROL=on)\n\
         \n\
         usage: df-transport-fixture dev [--server <url> --token-file <path>] [--pretty] [--timeout <s>] <command> [args]\n\
         \n\
         camera [home|shot|reset|save]  live TV/home pose; home --pitch N --yaw N --distance N --fov N; shot <name>\n\
         health                       server liveness and current revision\n\
         photo-check <path>           --kind canonical_room|scene_plate|character; re-judge a saved image\n\
         join|seat <seat>             join the room as seat 1 to 6; a refusal says why\n\
         reclaim <seat>               take back the away hero of a seat, including one a phone joined (silent 30 s)\n\
         create <seat> <name>         --choice group=id (repeatable), --identity <JSON>, --identity-choice group=id\n\
         preview <seat>              --appearance <text> --backstory <text>; review returned identity JSON\n\
         act <hero> <action> [...]    play one action: --cell c5r6 --target <foe> --weapon <name> --option <id> --knockout yes|no\n\
                                      help-attack: --ally <hero> --target <foe> (SRD 2024 Help, Attack)\n\
                                      scene-assault: --target <scene person> starts a fight in a freeplay scene\n\
         say <hero> <text>            speak one line; [--context question|social|plan|meta|joke|clarify]\n\
         direct plate <text>          re-render the current scene picture; [--keep] [--wait]\n\
         direct clear                 clear held directions and return to the ordinary plate\n\
         state                        room, phase, seats, fight, offers, revision\n\
         view --as tv|<hero>          TV uses a pure checkpoint/cache read; hero uses its bound view\n\
         notes <hero>                 print the notebook as that hero sees it\n\
         note <hero> <text>           add a shared party note\n\
         scene <hero>                current contents and item card\n\
         free-roam                    city, current place, known places, and prepared neighbours\n\
         feelings <npc>               current development-only NPC diagnostic\n\
         timeline [--last <n>] [--type <resource>] [--component <kind>] [--summary]  bounded resource and presentation lifecycles\n\
         merchant-quotes [--last <n>]  exact source-bound quote status and closed outcomes\n\
         ledger                       canonical coin, item custody, merchant purses and stock for every hero\n\
         gates                        which paid capabilities this run may use, and the switch for each\n\
         play --to <phase> [--heroes <1-6>] [--max-steps <n>] play to room|creation|opening|dialogue|combat|complete|ended\n\
         facts [<npc>]               recorded NPC facts and blank decisions (read)\n\
         lore [--scene <key|current>] the run's local canon, oldest fact first (read)\n\
         lore add <scene> <kind> <name> <content>  add one fact through the reply validator; marks edited\n\
         lore edit|delete ...         unsupported here: answers unavailable with the reason\n\
         social <npc> [--hero <name>] attitude, trust, emotion and grievances with their causes\n\
         social edit <npc> <hero> attitude friendly|indifferent|hostile | trust <-1..1>  set one standing (marks edited)\n\
         npc say <npc> <text>         the NPC speaks one line on the table; it must be on stage\n\
         npc act <npc> <action> [--target <foe>] [--weapon <name>] [--to <col,row>]  the NPC's fight action: attack move dash dodge disengage end-turn stand drop-prone nimble-disengage doff-shield death-save opportunity decline-reaction policy\n\
         npc hold <npc>|all on|off    hold NPC fight turns for the CLI; off lets the policy take a turn that waits\n\
         fight                        the open scene fight, its people and the latest roster\n\
         clips <name>                 character clip resources, states, costs, timings and file paths\n\
         clips request <name> [idle|attack|last-hit]  force one clip or all three\n\
         fight start --foe <srd id> --count <1-4> [--level 1|5] [--scene <id>] [--layout <id>]\n\
         fight end victory|defeat|flee | restore <person> | add <srd id> [--cell c5r6] | remove <name>\n\
         fight scene <id> [--layout <id>] | log [--last n] | offers <hero> | initiative\n\
         present pause|resume|step|replay | speed <0.25-4> | timing <name> <1-60000>|reset  display-only; pause holds new plans, queued beats continue; replay selects last submitted plan; 3D timing pending\n\
         scene-changes [--last <n>]   committed scene changes with their receipts\n\
         economy purse|coin <name> --cp <n> | offer add|remove|restock <offer>  unsupported here\n\
         audit [--type <KIND>] [--last <n>]  recent audit lines (IMAGERY, SCENE-FIGHT, ...)\n\
         decisions [--kind <kind>] [--last <n>]  Jev classifications; neutral marks the rules fallback\n\
         latency [--buckets 1]        owner call queue and reduce times and hero-line stage times: p50/p95 estimates, max and counts; buckets are opt-in\n\
         art [list <prefix>]          generated depictions on the hub; art regenerate|clear character:<id>|place:<id>\n\
         heroes list|show <id>|revisions <id>|delete <id>  inspect or delete local library heroes\n\
         fx [<effect> on|off]         the TV's screen-space effects: blur drift scrim grain vignette firelight dust grade\n\
         place [list|go <exit|place>]  where the party stands, its exits and the places the run knows; go moves the whole party (an exit to a new place creates it)\n\
         dice [next <face…>|clear] [--die d20]  forced faces for the next die draws (d4 d6 d8 d10 d12 d20 d100), first in first out\n\
         music                        place spec and cache status; explicit regenerate|clear|request|override\n\
         cost [--compare <bill.csv>]  what the run and this session cost: by provider, model and category, by cause, top spenders; --compare lines a provider,model,category,usd bill up against the run\n\
         memory [status]              story memory: counts by kind, embedding backlog and model, spend, queue\n\
         memory search <query> [--entity <id>] [--kind <kind>] [--audience public|party|member:<hero>] [--limit <n>]  the recall path the generators use\n\
         memory entity <id> [refresh] [--audience <a>] [--superseded true]  every event naming it, and its dossier\n\
         memory event <id>            one event with its supersession links (any audience)\n\
         memory supersede <id> --by <id>  mark an event superseded by another (nothing is deleted)\n\
         memory reindex               write the rebuildable sources again and fill in vectors\n\
         memory delete-run-index [all]  drop rebuildable events (all: every event and recall record)\n\
         memory erase <hero>          erase one hero's own words and what was derived from them\n\
         log [--since <seq>]          the table log and the override log\n\
         wait --until <condition>     turn <hero> | fight-end | phase <phase>\n\
         script <file>                run one dev command per line, stop at first failure\n\
         checkpoint save <name>       name the current point in the session\n\
         checkpoint list              named points saved so far\n\
         restore <name>               return to a named point (new revision)\n\
         back [n]                     state before the last n decisions\n\
         forward [n]                  re-apply stepped-over states\n\
         override hp <hero> <n>       set hit points within bounds (marks edited)\n\
         override condition <name> add|remove <condition> | cell <name> <cell>\n\
         override resource <name> second-wind|action-surge|ammo <n> | turn <name> | initiative <name> <n>\n\
         override budget <n>          set narrative budget within pack cap\n\
         shutdown                     clean stop; the database resumes immediately\n\
         describe                     every command, its arguments and output shapes\n\
         \n\
         (direct edits need the milestone 3 proof); see dev describe.\n\
         \n\
         actions: begin ask escort defend scene-assault move attack second-wind action-surge end-turn dash dodge\n\
                  disengage stand pick-up short-rest inn resolve search new-adventure\n\
                  opportunity-attack decline-reaction stabilise-kit stabilise-medicine help-attack table-setting\n\
         choose any offer: act <hero> <label-or-id>, or act <hero> choose --option <label-or-id>\n\
         reaction answer: act <hero> reaction take [--weapon <name>] | decline\n\
         table-setting: --choice-hints-hidden true|false\n\
         every choice group: --choice group=id (repeatable) or --choices <JSON>\n\
         one JSON document per command on stdout; errors are JSON on stderr.",
    );
}

fn parse_server(url: &str) -> Result<(String, u16), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("server {url:?} must be an http://127.0.0.1:<port> URL"))?;
    let (host, port) = rest
        .split_once(':')
        .ok_or_else(|| format!("server {url:?} must name a port"))?;
    if host != "127.0.0.1" && host != "localhost" {
        return Err(format!("server {url:?} must be loopback"));
    }
    let port: u16 = port
        .parse()
        .map_err(|_| format!("server {url:?} must name a numeric port"))?;
    Ok((host.to_owned(), port))
}

fn read_token(path: &str) -> Result<String, String> {
    let text =
        std::fs::read_to_string(path).map_err(|_| format!("cannot read token file {path:?}"))?;
    let token = text.trim().to_owned();
    if token.len() < 16 {
        return Err(format!("token file {path:?} holds no token"));
    }
    Ok(token)
}

fn camera_body(
    positional: &[String],
    options: &[(String, String)],
) -> Result<Option<Value>, String> {
    let command = positional.get(1).map(String::as_str);
    match command {
        None if positional.len() == 1 => Ok(None),
        Some("home") if positional.len() == 2 => {
            let mut body = json!({"command":"home"});
            for key in ["pitch", "yaw", "distance", "fov"] {
                let flag = format!("--{key}");
                let text = &options.iter().rev().find(|(name, _)| name == &flag)
                    .ok_or_else(|| format!("camera home needs {flag}"))?.1;
                let value = text.parse::<f32>()
                    .map_err(|_| format!("{flag} must be a finite number"))?;
                if !value.is_finite() {
                    return Err(format!("{flag} must be a finite number"));
                }
                body[key] = json!(value);
            }
            Ok(Some(body))
        }
        Some("shot") if positional.len() == 3 => Ok(Some(json!({"command":"shot","shot":positional[2]}))),
        Some(command @ ("reset" | "save")) if positional.len() == 2 => Ok(Some(json!({"command":command}))),
        _ => Err("usage: dev camera [home --pitch N --yaw N --distance N --fov N | shot <name> | reset | save]".to_owned()),
    }
}

async fn dispatch(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    let flag = |name: &str| {
        options
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    if roleplay::handles(positional) {
        return roleplay::run(client, positional, options).await;
    }
    match positional.first().map(String::as_str).unwrap_or("") {
        "camera" => match camera_body(positional, options) {
            Ok(None) => get(client, "/dev/camera").await,
            Ok(Some(body)) => post(client, "/dev/camera", &body).await,
            Err(message) => client.print_error(message),
        },
        "health" => get(client, "/dev/health").await,
        "photo-check" => match photo_check_path(positional, options) {
            Ok(path) => get(client, &path).await,
            Err(message) => client.print_error(message),
        },
        "join" | "seat" => {
            let seat = positional.get(1).cloned().unwrap_or_default();
            post(client, "/dev/join", &json!({"seat": seat})).await
        }
        "reclaim" => {
            let seat = positional.get(1).cloned().unwrap_or_default();
            post(client, "/dev/reclaim", &json!({"seat": seat})).await
        }
        "create" => {
            let (seat, name) = (
                positional.get(1).cloned().unwrap_or_default(),
                positional.get(2).cloned().unwrap_or_default(),
            );
            let mut body = json!({"seat":seat, "name":name});
            if let Err(message) = selection_flags(&mut body, options) { return client.print_error(message); }
            post(client, "/dev/create", &body).await
        }
        "preview" => {
            let mut body = json!({"seat":positional.get(1).cloned().unwrap_or_default(),
                "appearance_text":flag("--appearance").unwrap_or_default(), "backstory_text":flag("--backstory").unwrap_or_default()});
            if let Err(message) = selection_flags(&mut body, options) { return client.print_error(message); }
            post(client, "/dev/preview", &body).await
        }
        "act" | "confirm" => {
            if positional.first().is_some_and(|command| command == "confirm")
                && (flag("--input-id").is_none() || flag("--confirmation-token").is_none()) {
                return client.print_error("confirm needs --input-id and --confirmation-token from say");
            }
            let mut body = match act_body(positional, options) {
                Ok(body) => body,
                Err(message) => return client.print_error(message),
            };
            for (key, value) in [
                ("cell", "--cell"),
                ("target", "--target"),
                ("ally", "--ally"),
                ("weapon", "--weapon"),
                ("option", "--option"),
                ("input_id", "--input-id"),
                ("confirmation_token", "--confirmation-token"),
            ] {
                if let Some(option) = flag(value) {
                    body[key] = Value::String(option);
                }
            }
            if let Err(message) = selection_flags(&mut body, options) { return client.print_error(message); }
            if let Some(hidden) = flag("--choice-hints-hidden") {
                match hidden.parse::<bool>() {
                    Ok(hidden) => body["choice_hints_hidden"] = json!(hidden),
                    Err(_) => return client.print_error("--choice-hints-hidden must be true or false"),
                }
            }
            post(client, "/dev/act", &body).await
        }
        "cancel" => post(client, "/dev/cancel", &json!({"hero":positional.get(1).cloned().unwrap_or_default(),
            "input_id":flag("--input-id").unwrap_or_default(), "confirmation_token":flag("--confirmation-token").unwrap_or_default()})).await,
        "say" => {
            let text = positional.get(2..).unwrap_or(&[]).join(" ");
            let mut body = json!({
                "hero": positional.get(1).cloned().unwrap_or_default(),
                "text": text,
            });
            if let Some(context) = flag("--context") {
                body["context"] = Value::String(context);
            }
            post(client, "/dev/say", &body).await
        }
        "notes" => match positional.get(1) {
            Some(hero) => get(client, &format!("/dev/notes?hero={}", url_escape(hero))).await,
            None => client.print_error("notes needs a hero name"),
        },
        "note" => {
            let hero = positional.get(1).cloned().unwrap_or_default();
            let text = positional.get(2..).unwrap_or(&[]).join(" ");
            post(client, "/dev/note", &json!({"hero": hero, "text": text})).await
        }
        "facts" => match round_trip(client, "GET", "/dev/state", None).await {
            Ok((status, document)) if (200..300).contains(&status) => {
                client.print(&fact_decisions_document(&document, positional.get(1).map(String::as_str)), false)
            }
            Ok((_, document)) => client.print(&document, true),
            Err(message) => CommandResult::Transport(message),
        },
        "state" => get(client, "/dev/state").await,
        "free-roam" => get(client, "/dev/place").await,
        command @ ("scene" | "feelings") => match positional.get(1) {
            Some(name) => get(client, &format!("/dev/{command}?name={}", url_escape(name))).await,
            None => client.print_error("scene/feelings needs a name"),
        },
        "direct" => direct(client, positional, options).await,
        "view" => match flag("--as") {
            Some(who) => get(client, &format!("/dev/view?as={}", url_escape(&who))).await,
            None => client.print_error("view needs --as tv|<hero>"),
        },
        "log" => match flag("--since") {
            Some(since) => get(client, &format!("/dev/log?since={}", url_escape(&since))).await,
            None => get(client, "/dev/log").await,
        },
        "timeline" => {
            let mut path = format!(
                "/dev/timeline?last={}",
                url_escape(&flag("--last").or_else(|| positional.get(1).cloned()).unwrap_or_else(|| "50".into()))
            );
            if let Some(resource_type) = flag("--type") {
                path.push_str("&type=");
                path.push_str(&url_escape(&resource_type));
            }
            if let Some(component) = flag("--component") {
                path.push_str("&component=");
                path.push_str(&url_escape(&component));
            }
            if flag("--summary").as_deref() == Some("true") {
                path.push_str("&summary=true");
            }
            for name in ["since", "npc"] {
                if let Some(value) = flag(&format!("--{name}")) { path.push_str(&format!("&{name}={}",url_escape(&value))); }
            }
            get(client, &path).await
        }
        "merchant-quotes" => {
            let last = flag("--last")
                .or_else(|| positional.get(1).cloned())
                .unwrap_or_else(|| "50".into());
            get(
                client,
                &format!("/dev/merchant-quotes?last={}", url_escape(&last)),
            )
            .await
        }
        "ledger" => get(client, "/dev/ledger").await,
        "gates" => get(client, "/dev/gates").await,
        "lore" => lore(client, positional, options).await,
        "social" => social(client, positional, options).await,
        "fight" => fight(client, positional, options).await,
        "present" => present(client, positional, options).await,
        "npc" => npc(client, positional, options).await,
        "economy" => economy(client, positional, options).await,
        "art" => art(client, positional).await,
        "heroes" => match heroes_path(positional) {
            Ok((true, path)) => post(client, &path, &json!({})).await,
            Ok((false, path)) => get(client, &path).await,
            Err(message) => client.print_error(message),
        },
        "clips" => clips(client, positional).await,
        "fx" => fx(client, positional).await,
        "dice" => dice::run(client, positional, options).await,
        "place" => place::run(client, positional, options).await,
        "tempo" => {
            let last = flag("--last").unwrap_or_else(|| "50".into());
            get(client, &format!("/dev/tempo?last={}", url_escape(&last))).await
        }
        "memory" => memory(client, positional, options).await,
        "cost" => cost(client, options).await,
        "scene-changes" => {
            let last = flag("--last")
                .or_else(|| positional.get(1).cloned())
                .unwrap_or_else(|| "50".into());
            get(
                client,
                &format!("/dev/scene-changes?last={}", url_escape(&last)),
            )
            .await
        }
        "decisions" => {
            let mut path = format!(
                "/dev/decisions?last={}",
                url_escape(&flag("--last").unwrap_or_else(|| "50".into()))
            );
            if let Some(kind) = flag("--kind") {
                path.push_str("&kind=");
                path.push_str(&url_escape(&kind));
            }
            for name in ["since", "npc"] {
                if let Some(value) = flag(&format!("--{name}")) { path.push_str(&format!("&{name}={}",url_escape(&value))); }
            }
            get(client, &path).await
        }
        "latency" => match flag("--buckets") {
            Some(value) => get(client, &format!("/dev/latency?buckets={}", url_escape(&value))).await,
            None => get(client, "/dev/latency").await,
        },
        "audit" => {
            let mut path = format!(
                "/dev/audit?last={}",
                url_escape(&flag("--last").unwrap_or_else(|| "50".into()))
            );
            if let Some(line_type) = flag("--type") {
                path.push_str("&type=");
                path.push_str(&url_escape(&line_type));
            }
            for name in ["since", "npc"] {
                if let Some(value) = flag(&format!("--{name}")) { path.push_str(&format!("&{name}={}",url_escape(&value))); }
            }
            get(client, &path).await
        }
        "layout" => layout::run(client, positional, options).await,
        "play" => play::run(client, options).await,
        "wait" => wait(client, options).await,
        "checkpoint" => match positional.get(1).map(String::as_str).unwrap_or("") {
            "save" => {
                let name = positional.get(2).cloned().unwrap_or_default();
                post(client, "/dev/checkpoint", &json!({"name": name})).await
            }
            "list" => get(client, "/dev/checkpoints").await,
            _ => client.print_error("checkpoint needs save <name> or list"),
        },
        "restore" => {
            let name = positional.get(1).cloned().unwrap_or_default();
            post(client, "/dev/restore", &json!({"name": name})).await
        }
        "back" => {
            let n = positional.get(1).cloned().unwrap_or_else(|| "1".to_owned());
            match n.parse::<u64>() {
                Ok(n) => post(client, "/dev/back", &json!({"n": n})).await,
                Err(_) => client.print_error("back needs a step count, e.g. back 2"),
            }
        }
        "forward" => {
            let n = positional.get(1).cloned().unwrap_or_else(|| "1".to_owned());
            match n.parse::<u64>() {
                Ok(n) => post(client, "/dev/forward", &json!({"n": n})).await,
                Err(_) => client.print_error("forward needs a step count, e.g. forward 1"),
            }
        }
        "override" => {
            match override_body(positional) {
                Ok(body) => post(client, "/dev/override", &body).await,
                Err(reason) => client.print_error(reason),
            }
        }
        "shutdown" => post(client, "/dev/shutdown", &json!({})).await,
        "describe" => get(client, "/dev/describe").await,
        other => client.print_error(format!("unknown dev command {other:?}; see dev --help")),
    }
}

fn act_body(positional: &[String], options: &[(String, String)]) -> Result<Value, String> {
    let flag = |name: &str| {
        options
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    let hero = positional.get(1).cloned().unwrap_or_default();
    let reaction_answer = positional
        .get(2)
        .is_some_and(|action| action.eq_ignore_ascii_case("reaction"));
    let action = if reaction_answer {
        match positional.get(3) {
            Some(choice) if choice.eq_ignore_ascii_case("take") => "opportunity-attack".to_owned(),
            Some(choice) if choice.eq_ignore_ascii_case("decline") => "decline-reaction".to_owned(),
            _ => return Err("act <hero> reaction needs take or decline".to_owned()),
        }
    } else {
        positional.get(2).cloned().unwrap_or_default()
    };
    let mut body = json!({"hero": hero, "action": action});
    if let Some(knockout) = flag("--knockout") {
        body["knockout"] = Value::String(knockout);
    }
    if reaction_answer
        && positional
            .get(3)
            .is_some_and(|choice| choice.eq_ignore_ascii_case("take"))
        && flag("--option").is_none()
        && let Some(weapon) = flag("--weapon")
    {
        body["option"] = Value::String(weapon);
    }
    Ok(body)
}

fn selection_flags(body: &mut Value, options: &[(String, String)]) -> Result<(), String> {
    if let Some((_, value)) = options
        .iter()
        .rev()
        .find(|(key, _)| key == "--choice-hints-hidden")
    {
        let hidden = value
            .parse::<bool>()
            .map_err(|_| "--choice-hints-hidden must be true or false".to_owned())?;
        body["choice_hints_hidden"] = json!(hidden);
    }
    for (name, field) in [
        ("--choices", "choices"),
        ("--identity", "identity"),
        ("--identity-choices", "identity_choices"),
    ] {
        if let Some((_, value)) = options.iter().rev().find(|(key, _)| key == name) {
            body[field] =
                serde_json::from_str(value).map_err(|_| format!("{name} needs a JSON object"))?;
            if !body[field].is_object() {
                return Err(format!("{name} needs a JSON object"));
            }
        }
    }
    for (flag, field) in [
        ("--choice", "choices"),
        ("--identity-choice", "identity_choices"),
    ] {
        for (_, value) in options.iter().filter(|(key, _)| key == flag) {
            let (group, option) = value
                .split_once('=')
                .filter(|(group, option)| !group.is_empty() && !option.is_empty())
                .ok_or_else(|| format!("{flag} needs group=option"))?;
            if body.get(field).is_none() {
                body[field] = json!({});
            }
            let values = body[field]
                .as_object_mut()
                .ok_or_else(|| format!("{flag} needs an object"))?;
            if values.insert(group.into(), json!(option)).is_some() {
                return Err(format!("duplicate choice group {group:?}"));
            }
        }
    }
    Ok(())
}

fn heroes_path(positional: &[String]) -> Result<(bool, String), String> {
    let usage =
        "usage: dev heroes list | show <id> | revisions <id> | delete <id> (positive i64 id)";
    match positional.get(1).map(String::as_str) {
        Some("list") if positional.len() == 2 => Ok((false, "/dev/heroes".into())),
        Some(command @ ("show" | "revisions" | "delete")) if positional.len() == 3 => {
            let id = positional[2]
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or(usage)?;
            let suffix = match command {
                "revisions" => "/revisions",
                "delete" => "/delete",
                _ => "",
            };
            Ok((command == "delete", format!("/dev/heroes/{id}{suffix}")))
        }
        _ => Err(usage.into()),
    }
}

fn photo_check_path(positional: &[String], options: &[(String, String)]) -> Result<String, String> {
    let path = positional
        .get(1)
        .filter(|path| !path.is_empty())
        .ok_or("photo-check needs a saved image path")?;
    if positional.len() != 2 {
        return Err("photo-check takes exactly one saved image path".to_owned());
    }
    let kind = options
        .iter()
        .rev()
        .find(|(name, _)| name == "--kind")
        .map(|(_, value)| value.as_str())
        .ok_or("photo-check needs --kind canonical_room|scene_plate|character")?;
    if !matches!(kind, "canonical_room" | "scene_plate" | "character") {
        return Err("kind must be canonical_room, scene_plate or character".to_owned());
    }
    Ok(format!(
        "/dev/photo-check?path={}&kind={}",
        url_escape(path),
        url_escape(kind)
    ))
}

fn url_escape(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

async fn post(client: &Client, path: &str, body: &Value) -> CommandResult {
    let text = body.to_string();
    match round_trip(client, "POST", path, Some(&text)).await {
        Ok((status, document)) => client.print(&document, !(200..300).contains(&status)),
        Err(message) => CommandResult::Transport(message),
    }
}

async fn direct(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    let kind = positional.get(1).map(String::as_str).unwrap_or("");
    if !matches!(kind, "plate" | "clear") {
        return client.print_error("direct needs plate <text> or clear");
    }
    let enabled = |name: &str| {
        options
            .iter()
            .any(|(key, value)| key == name && value == "true")
    };
    let body = json!({"kind": kind, "text": positional.get(2..).unwrap_or(&[]).join(" "),
        "keep": enabled("--keep")})
    .to_string();
    let deadline = tokio::time::Instant::now() + client.timeout;
    let poll = async {
        let mut response = round_trip(client, "POST", "/dev/direct", Some(&body)).await?;
        loop {
            if !(200..300).contains(&response.0)
                || !enabled("--wait")
                || response.1.get("state").and_then(Value::as_str) != Some("pending")
            {
                return Ok::<_, String>(response);
            }
            let key = response
                .1
                .get("key")
                .and_then(Value::as_str)
                .ok_or_else(|| "pending plate has no key".to_owned())?
                .to_owned();
            tokio::time::sleep(Duration::from_millis(250)).await;
            response = round_trip(
                client,
                "GET",
                &format!("/dev/direct/{}", url_escape(&key)),
                None,
            )
            .await?;
        }
    };
    match tokio::time::timeout_at(deadline, poll).await {
        Ok(Ok((status, document))) => {
            let failed = !(200..300).contains(&status)
                || document.get("state").and_then(Value::as_str) == Some("failed");
            client.print(&document, failed)
        }
        Ok(Err(message)) => CommandResult::Transport(message),
        Err(_) => client.print(&json!({"status": "wait-timeout", "kind": kind}), true),
    }
}

/// The value of the last `name` option that was given.
fn option_value(options: &[(String, String)], name: &str) -> Option<String> {
    options
        .iter()
        .rev()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
}

/// The words from `from` on, joined with single spaces, as free text.
fn rest_text(positional: &[String], from: usize) -> String {
    positional.get(from..).unwrap_or(&[]).join(" ")
}

/// The body of `dev lore add <scene> <kind> <name> <content…>`.
fn lore_add_body(positional: &[String]) -> Result<Value, String> {
    let (Some(scene), Some(kind), Some(name)) =
        (positional.get(2), positional.get(3), positional.get(4))
    else {
        return Err("lore add needs <scene> <kind> <name> <content>".to_owned());
    };
    Ok(json!({"scene": scene, "kind": kind, "name": name, "content": rest_text(positional, 5)}))
}

async fn lore(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    match positional.get(1).map(String::as_str) {
        None => {
            let path = match option_value(options, "--scene") {
                Some(scene) => format!("/dev/lore?scene={}", url_escape(&scene)),
                None => "/dev/lore".to_owned(),
            };
            get(client, &path).await
        }
        Some("add") => match lore_add_body(positional) {
            Ok(body) => post(client, "/dev/lore", &body).await,
            Err(message) => client.print_error(message),
        },
        Some("edit") => match (positional.get(2), positional.get(3)) {
            (Some(scene), Some(name)) => {
                let body =
                    json!({"scene": scene, "name": name, "content": rest_text(positional, 4)});
                post(client, "/dev/lore/edit", &body).await
            }
            _ => client.print_error("lore edit needs <scene> <name> <content>"),
        },
        Some("delete") => match (positional.get(2), positional.get(3)) {
            (Some(scene), Some(name)) => {
                post(
                    client,
                    "/dev/lore/delete",
                    &json!({"scene": scene, "name": name}),
                )
                .await
            }
            _ => client.print_error("lore delete needs <scene> <name>"),
        },
        Some(_) => client.print_error("lore takes no word to read, or add|edit|delete"),
    }
}

/// `dev cost [--compare <bill.csv>]`: the run's and session's cost, or the run beside a bill.
async fn cost(client: &Client, options: &[(String, String)]) -> CommandResult {
    match option_value(options, "--compare") {
        None => get(client, "/dev/cost").await,
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(csv) => post(client, "/dev/cost/compare", &json!({"csv": csv})).await,
            Err(_) => client.print_error("cost --compare needs a readable bill file"),
        },
    }
}

/// `dev memory ...`: the story memory of the run.
async fn memory(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    let mut path = String::new();
    let mut add = |name: &str, value: Option<String>| {
        if let Some(value) = value {
            path.push(if path.contains('?') { '&' } else { '?' });
            path.push_str(name);
            path.push('=');
            path.push_str(&url_escape(&value));
        }
    };
    match positional.get(1).map(String::as_str) {
        None | Some("status") => get(client, "/dev/memory").await,
        Some("search") => {
            let query = rest_text(positional, 2);
            if query.trim().is_empty() {
                return client.print_error("memory search needs a query");
            }
            add("q", Some(query));
            add("entity", option_value(options, "--entity"));
            add("kind", option_value(options, "--kind"));
            add("audience", option_value(options, "--audience"));
            add("limit", option_value(options, "--limit"));
            get(client, &format!("/dev/memory/search{path}")).await
        }
        Some("entity") => {
            let Some(id) = positional.get(2) else {
                return client.print_error("memory entity needs an entity id");
            };
            add("id", Some(id.clone()));
            add("audience", option_value(options, "--audience"));
            add("superseded", option_value(options, "--superseded"));
            if positional.get(3).map(String::as_str) == Some("refresh") {
                add("refresh", Some("true".to_owned()));
            }
            get(client, &format!("/dev/memory/entity{path}")).await
        }
        Some("event") => match positional.get(2) {
            Some(id) => get(client, &format!("/dev/memory/event?id={}", url_escape(id))).await,
            None => client.print_error("memory event needs an event id"),
        },
        Some("supersede") => {
            let old = positional.get(2).and_then(|id| id.parse::<i64>().ok());
            let by = option_value(options, "--by").and_then(|id| id.parse::<i64>().ok());
            match (old, by) {
                (Some(old), Some(by)) => {
                    post(client, "/dev/memory/supersede", &json!({"old": old, "by": by})).await
                }
                _ => client.print_error("memory supersede needs <id> --by <id>"),
            }
        }
        Some("reindex") => post(client, "/dev/memory/reindex", &json!({})).await,
        Some("delete-run-index") => {
            let all = positional.get(2).map(String::as_str) == Some("all");
            post(client, "/dev/memory/delete-run-index", &json!({"all": all})).await
        }
        Some("erase") => match positional.get(2) {
            Some(hero) => post(client, "/dev/memory/erase", &json!({"hero": hero})).await,
            None => client.print_error("memory erase needs a hero name"),
        },
        Some(_) => client.print_error(
            "memory takes status, search, entity, event, supersede, reindex, delete-run-index or erase",
        ),
    }
}

/// The body of `dev social edit <npc> <hero> attitude|trust <value>`.
fn social_edit_body(positional: &[String]) -> Result<Value, String> {
    let (Some(npc), Some(hero), Some(op), Some(value)) = (
        positional.get(2),
        positional.get(3),
        positional.get(4),
        positional.get(5),
    ) else {
        return Err("social edit needs <npc> <hero> attitude|trust <value>".to_owned());
    };
    Ok(json!({"npc": npc, "hero": hero, "op": op, "value": value}))
}

/// The body of `dev npc say <npc> <text…>`.
fn npc_say_body(positional: &[String]) -> Result<Value, String> {
    let Some(npc) = positional.get(2) else {
        return Err("npc say needs <npc> <text>".to_owned());
    };
    let text = rest_text(positional, 3);
    if text.trim().is_empty() {
        return Err("npc say needs <npc> <text>".to_owned());
    }
    Ok(json!({"npc": npc, "text": text}))
}

/// The body of `dev npc act <npc> <action> [--target] [--weapon] [--to]`.
fn npc_act_body(positional: &[String], options: &[(String, String)]) -> Result<Value, String> {
    let (Some(npc), Some(action)) = (positional.get(2), positional.get(3)) else {
        return Err("npc act needs <npc> <action>".to_owned());
    };
    Ok(json!({
        "npc": npc,
        "action": action,
        "target": option_value(options, "--target"),
        "weapon": option_value(options, "--weapon"),
        "to": option_value(options, "--to"),
    }))
}

/// The body of `dev npc hold <npc> on|off`.
fn npc_hold_body(positional: &[String]) -> Result<Value, String> {
    let (Some(npc), Some(state)) = (positional.get(2), positional.get(3)) else {
        return Err("npc hold needs <npc> on|off".to_owned());
    };
    let on = match state.as_str() {
        "on" => true,
        "off" => false,
        _ => return Err("npc hold takes on or off".to_owned()),
    };
    Ok(json!({"npc": npc, "on": on}))
}

async fn npc(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    match positional.get(1).map(String::as_str) {
        Some("say") => match npc_say_body(positional) {
            Ok(body) => post(client, "/dev/npc/say", &body).await,
            Err(message) => client.print_error(message),
        },
        Some("act") => match npc_act_body(positional, options) {
            Ok(body) => post(client, "/dev/npc/act", &body).await,
            Err(message) => client.print_error(message),
        },
        Some("hold") => match npc_hold_body(positional) {
            Ok(body) => post(client, "/dev/npc/hold", &body).await,
            Err(message) => client.print_error(message),
        },
        _ => client.print_error(
            "npc takes show|last <npc>, say <npc> <text>, act <npc> <action> [...], or hold <npc> on|off",
        ),
    }
}

async fn social(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    if positional.get(1).map(String::as_str) == Some("edit") {
        return match social_edit_body(positional) {
            Ok(body) => post(client, "/dev/social/edit", &body).await,
            Err(message) => client.print_error(message),
        };
    }
    let Some(npc) = positional.get(1) else {
        return client.print_error("social needs an NPC name or id, or edit <op> <npc> <hero>");
    };
    let mut path = format!("/dev/social?name={}", url_escape(npc));
    if let Some(hero) = option_value(options, "--hero") {
        path.push_str("&hero=");
        path.push_str(&url_escape(&hero));
    }
    get(client, &path).await
}

async fn fight(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    match fight_request(positional, options) {
        Ok((path, Some(body))) => post(client, &path, &body).await,
        Ok((path, None)) => get(client, &path).await,
        Err(reason) => client.print_error(reason),
    }
}
fn fight_request(
    positional: &[String],
    options: &[(String, String)],
) -> Result<(String, Option<Value>), String> {
    let flag = |name: &str| option_value(options, name);
    let required = |index: usize, message: &str| {
        positional
            .get(index)
            .cloned()
            .ok_or_else(|| message.to_owned())
    };
    let body = match positional.get(1).map(String::as_str) {
        None => return Ok(("/dev/fight".to_owned(), None)),
        Some("start") => {
            let foe = flag("--foe").ok_or("fight start needs --foe <srd id>")?;
            let count = flag("--count")
                .and_then(|text| text.parse::<u8>().ok())
                .filter(|count| (1..=4).contains(count))
                .ok_or("fight start --count must be 1..=4")?;
            let level = flag("--level")
                .map(|text| {
                    text.parse::<u8>()
                        .map_err(|_| "fight start --level must be 1 or 5")
                })
                .transpose()?;
            if level.is_some_and(|level| !matches!(level, 1 | 5)) {
                return Err("fight start --level must be 1 or 5".to_owned());
            }
            json!({"op":"start","foe":foe,"count":count,"level":level,"scene":flag("--scene"),"layout":flag("--layout")})
        }
        Some("end") => {
            let outcome = required(2, "fight end needs victory, defeat or flee")?;
            if !matches!(outcome.as_str(), "victory" | "defeat" | "flee") {
                return Err("fight end needs victory, defeat or flee".to_owned());
            }
            json!({"op":"end","outcome":outcome})
        }
        Some("restore") => {
            json!({"op":"restore","person":required(2,"fight restore needs <person>")?})
        }
        Some("add") => {
            json!({"op":"add","foe":required(2,"fight add needs <srd id>")?,"cell":flag("--cell")})
        }
        Some("remove") => json!({"op":"remove","name":required(2,"fight remove needs <name>")?}),
        Some("scene") => {
            json!({"op":"scene","scene":required(2,"fight scene needs <battle scene id>")?,"layout":flag("--layout")})
        }
        Some("initiative") => return Ok(("/dev/fight/initiative".to_owned(), None)),
        Some("log") => {
            return Ok((
                format!(
                    "/dev/fight/log?last={}",
                    url_escape(&flag("--last").unwrap_or_else(|| "50".to_owned()))
                ),
                None,
            ));
        }
        Some("offers") => {
            return Ok((
                format!(
                    "/dev/fight/offers?hero={}",
                    url_escape(&required(2, "fight offers needs <hero>")?)
                ),
                None,
            ));
        }
        _ => {
            return Err(
                "fight takes start, end, restore, add, remove, scene, log, offers or initiative"
                    .to_owned(),
            );
        }
    };
    Ok(("/dev/fight/edit".to_owned(), Some(body)))
}

fn override_body(positional: &[String]) -> Result<Value, String> {
    let words = positional.iter().map(String::as_str).collect::<Vec<_>>();
    let number = |text: &str| {
        text.parse::<i64>()
            .map_err(|_| "override value must be a whole number".to_owned())
    };
    match words.as_slice() {
        ["override", "hp", name, value] => Ok(json!({"kind":"hp","hero":name,"value":number(value)?})),
        ["override", "budget", value] => Ok(json!({"kind":"budget","value":number(value)?})),
        ["override", "condition", name, mode, condition] if matches!(*mode,"add"|"remove") => Ok(json!({"kind":"condition","hero":name,"condition":condition,"present":*mode=="add"})),
        ["override", "cell", name, cell] => Ok(json!({"kind":"cell","hero":name,"cell":cell})),
        ["override", "resource", name, resource, value] => Ok(json!({"kind":"resource","hero":name,"resource":resource,"value":number(value)?})),
        ["override", "turn", name] => Ok(json!({"kind":"turn","hero":name})),
        ["override", "initiative", name, value] => Ok(json!({"kind":"initiative","hero":name,"value":number(value)?})),
        _ => Err("override takes hp <name> <n>, condition <name> add|remove <condition>, cell <name> <cell>, resource <name> <resource> <n>, turn <name>, initiative <name> <n>, or budget <n>".to_owned()),
    }
}

async fn present(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    match presentation_body(positional, options) {
        Ok(body) => post(client, "/dev/presentation", &body).await,
        Err(error) => client.print_error(error),
    }
}
fn presentation_body(positional: &[String], options: &[(String, String)]) -> Result<Value, String> {
    match positional.get(1).map(String::as_str) {
        Some(op @ ("pause" | "resume" | "step" | "replay")) => Ok(json!({"op":op})),
        Some("speed") => {
            let value = positional
                .get(2)
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| (0.25..=4.0).contains(value))
                .ok_or("present speed needs a value from 0.25 to 4")?;
            Ok(json!({"op":"speed","value":value}))
        }
        Some("timing") => match positional.get(2).map(String::as_str) {
            Some("reset") => Ok(json!({"op":"timing-reset"})),
            Some(name) => {
                let ms = positional
                    .get(3)
                    .cloned()
                    .or_else(|| option_value(options, "--ms"))
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|ms| (1..=60000).contains(ms))
                    .ok_or("present timing needs <name> <ms> from 1 to 60000")?;
                Ok(json!({"op":"timing","name":name,"ms":ms}))
            }
            None => Err("present timing needs <name> <ms> or reset".to_owned()),
        },
        _ => Err("present takes pause, resume, step, replay, speed or timing".to_owned()),
    }
}

/// The body of `dev economy purse|coin <name> --cp <n>` or `dev economy offer <op> <offer>`.
fn economy_body(positional: &[String], options: &[(String, String)]) -> Result<Value, String> {
    match positional.get(1).map(String::as_str) {
        Some(what @ ("purse" | "coin")) => {
            let name = positional
                .get(2)
                .ok_or_else(|| format!("economy {what} needs <name> --cp <n>"))?;
            let text = option_value(options, "--cp")
                .ok_or_else(|| format!("economy {what} needs --cp <n>"))?;
            let cp: u32 = text
                .parse()
                .map_err(|_| "--cp must be a whole number of copper".to_owned())?;
            Ok(json!({"op": what, "target": name, "cp": cp}))
        }
        Some("offer") => match (positional.get(2), positional.get(3)) {
            (Some(op), Some(offer)) if matches!(op.as_str(), "add"|"remove"|"restock") => Ok(json!({"op": format!("offer-{op}"), "target": offer})),
            _ => Err("economy offer needs add|remove|restock <offer>".to_owned()),
        },
        _ => Err(
            "economy needs purse <npc> --cp <n>, coin <hero> --cp <n>, or offer add|remove|restock <offer>"
                .to_owned(),
        ),
    }
}

async fn economy(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    match economy_body(positional, options) {
        Ok(body) => post(client, "/dev/economy/edit", &body).await,
        Err(message) => client.print_error(message),
    }
}

async fn fx(client: &Client, positional: &[String]) -> CommandResult {
    match (positional.get(1), positional.get(2).map(String::as_str)) {
        (None, None) => get(client, "/dev/fx").await,
        (Some(effect), Some(state @ ("on" | "off"))) => {
            post(
                client,
                "/dev/fx/edit",
                &json!({"effect": effect, "on": state == "on"}),
            )
            .await
        }
        _ => client.print_error(
            "fx takes nothing to list, or <effect> on|off (blur, drift, scrim, grain, vignette, firelight, dust, grade)",
        ),
    }
}

async fn art(client: &Client, positional: &[String]) -> CommandResult {
    match positional.get(1).map(String::as_str) {
        None => get(client, "/dev/art").await,
        Some("list") => match positional.get(2) {
            Some(subject) => {
                get(client, &format!("/dev/art?subject={}", url_escape(subject))).await
            }
            None => get(client, "/dev/art").await,
        },
        Some(op @ ("regenerate" | "clear")) => match positional.get(2) {
            Some(subject) => {
                post(
                    client,
                    "/dev/art/edit",
                    &json!({"op": op, "subject": subject}),
                )
                .await
            }
            None => client.print_error(format!("art {op} needs character:<id> or place:<id>")),
        },
        Some(_) => client.print_error(
            "art takes list [subject-prefix], or regenerate|clear character:<id>|place:<id>",
        ),
    }
}

fn clips_request_body(positional: &[String]) -> Result<Value, &'static str> {
    let name = positional
        .get(2)
        .filter(|name| !name.is_empty())
        .ok_or("clips request needs <name> [idle|attack|last-hit]")?;
    let kind = positional.get(3);
    if positional.len() > 4
        || kind.is_some_and(|kind| !matches!(kind.as_str(), "idle" | "attack" | "last-hit"))
    {
        return Err("clips request takes <name> [idle|attack|last-hit]");
    }
    Ok(json!({"name": name, "kind": kind}))
}

async fn clips(client: &Client, positional: &[String]) -> CommandResult {
    match positional.get(1).map(String::as_str) {
        Some("request") => match clips_request_body(positional) {
            Ok(body) => post(client, "/dev/clips/request", &body).await,
            Err(message) => client.print_error(message),
        },
        Some(name) if positional.len() == 2 => {
            get(client, &format!("/dev/clips?name={}", url_escape(name))).await
        }
        _ => client.print_error("clips takes <name>, or request <name> [idle|attack|last-hit]"),
    }
}

#[cfg(test)]
mod clips_command_tests {
    use super::*;

    #[test]
    fn clips_request_sends_the_name_and_optional_closed_kind() {
        let words = |text: &str| {
            text.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            clips_request_body(&words("clips request Brakka")).unwrap(),
            json!({"name": "Brakka", "kind": null})
        );
        for kind in ["idle", "attack", "last-hit"] {
            let body = clips_request_body(&words(&format!("clips request Brakka {kind}"))).unwrap();
            assert_eq!(body["kind"], kind);
        }
        assert!(clips_request_body(&words("clips request")).is_err());
        assert!(clips_request_body(&words("clips request Brakka jump")).is_err());
        assert!(clips_request_body(&words("clips request Brakka idle extra")).is_err());
    }
}

fn fact_decisions_document(state: &Value, npc: Option<&str>) -> Value {
    let decisions: Vec<&Value> = state["npc_facts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|record| {
            npc.is_none_or(|npc| {
                ["npc_id", "speaker"].into_iter().any(|key| {
                    record[key]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(npc))
                })
            })
        })
        .collect();
    json!({"npc":npc, "count":decisions.len(), "decisions":decisions})
}

async fn get(client: &Client, path: &str) -> CommandResult {
    match round_trip(client, "GET", path, None).await {
        Ok((status, document)) => client.print(&document, !(200..300).contains(&status)),
        Err(message) => CommandResult::Transport(message),
    }
}

async fn round_trip(
    client: &Client,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<(u16, Value), String> {
    let length = body.map(str::len).unwrap_or(0);
    let request = format!(
        "{method} {path} HTTP/1.1\r\nhost: {}:{}\r\nauthorization: Bearer {}\r\ncontent-type: application/json\r\ncontent-length: {length}\r\nconnection: close\r\n\r\n{}",
        client.host,
        client.port,
        client.token,
        body.unwrap_or(""),
    );
    let stream = timeout(
        client.timeout,
        tokio::net::TcpStream::connect((client.host.as_str(), client.port)),
    )
    .await
    .map_err(|_| format!("server {}:{} did not answer", client.host, client.port))?
    .map_err(|error| {
        format!(
            "cannot reach server {}:{} ({error})",
            client.host, client.port
        )
    })?;
    let (mut reader, mut writer) = stream.into_split();
    timeout(client.timeout, writer.write_all(request.as_bytes()))
        .await
        .map_err(|_| "server write timed out".to_owned())?
        .map_err(|error| format!("server write failed ({error})"))?;
    let mut raw = Vec::new();
    timeout(client.timeout, reader.read_to_end(&mut raw))
        .await
        .map_err(|_| {
            "server read timed out; a write may still have committed, so run dev state before retrying"
                .to_owned()
        })?
        .map_err(|error| format!("server read failed ({error})"))?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| "server answer is not HTTP".to_owned())?;
    let status: u16 = head
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| "server answer has no status".to_owned())?;
    let document: Value = serde_json::from_str(body)
        .map_err(|_| format!("server answer is not JSON: {body:.120}"))?;
    Ok((status, document))
}

async fn wait(client: &Client, options: &[(String, String)]) -> CommandResult {
    match roleplay::wait_seconds(options) {
        Ok(Some(seconds)) => {
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            return client.print(&json!({"status":"waited","seconds":seconds}), false);
        }
        Err(error) => return client.print_error(error),
        Ok(None) => {}
    }
    let until: Vec<String> = options
        .iter()
        .filter(|(key, _)| key == "--until")
        .map(|(_, value)| value.clone())
        .collect();
    let words: Vec<&str> = until
        .iter()
        .flat_map(|text| text.split_whitespace())
        .collect();
    if words.is_empty() {
        return client.print_error("wait needs --until <turn <hero>|fight-end|phase <phase>>");
    }
    let deadline = std::time::Instant::now() + client.timeout;
    loop {
        let state = match round_trip(client, "GET", "/dev/state", None).await {
            Ok((status, document)) if (200..300).contains(&status) => document,
            // The owner was busy for this one read: keep waiting until the deadline.
            Ok((503, _)) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(1000)).await;
                continue;
            }
            Ok((_, document)) => return client.print(&document, true),
            Err(message) => return CommandResult::Transport(message),
        };
        if condition_met(&state, &words) {
            return client.print(
                &json!({"status": "condition-met", "until": words.join(" "), "state": state}),
                false,
            );
        }
        if std::time::Instant::now() >= deadline {
            return client.print(
                &json!({"status": "wait-timeout", "until": words.join(" "), "state": state}),
                true,
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn condition_met(state: &Value, words: &[&str]) -> bool {
    match words {
        ["turn", hero] => {
            let active = state
                .pointer("/combat/active_actor")
                .and_then(Value::as_str)
                .unwrap_or("");
            active.eq_ignore_ascii_case(hero)
        }
        ["fight-end"] => {
            state.pointer("/combat").is_none()
                || state
                    .pointer("/combat/finished")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        }
        ["phase", phase] => state
            .get("phase")
            .and_then(Value::as_str)
            .is_some_and(|current| current.eq_ignore_ascii_case(phase)),
        _ => false,
    }
}

async fn run_script(
    client: &Client,
    positional: &[String],
    options: &[(String, String)],
) -> CommandResult {
    let path = positional.get(1).cloned().unwrap_or_default();
    if path.is_empty() {
        return CommandResult::Transport("script needs a file".to_owned());
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return CommandResult::Transport(format!("cannot read script {path:?}")),
    };
    // The script inherits the outer --server/--token-file/--pretty/--timeout;
    // a script line carries only the dev command itself.
    let mut server = options
        .iter()
        .rev()
        .find(|(key, _)| key == "--server")
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var("DF_DEV_CONTROL_URL").ok());
    let mut token_file = options
        .iter()
        .rev()
        .find(|(key, _)| key == "--token-file")
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var("DF_DEV_CONTROL_TOKEN_FILE").ok());
    if server.is_none() || token_file.is_none() {
        return CommandResult::Transport(
            "script needs --server and --token-file (or DF_DEV_CONTROL_URL / DF_DEV_CONTROL_TOKEN_FILE)".to_owned(),
        );
    }
    let _ = (&mut server, &mut token_file);
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words = split_words(line);
        if words.is_empty() {
            continue;
        }
        let mut merged = vec!["dev".to_owned()];
        for (key, value) in options {
            merged.push(key.clone());
            if !matches!(key.as_str(), "--keep" | "--wait") {
                merged.push(value.clone());
            }
        }
        if client.pretty {
            merged.push("--pretty".to_owned());
        }
        merged.extend(words);
        let arguments: Vec<OsString> = merged.into_iter().map(OsString::from).collect();
        match Box::pin(run_inner(&arguments)).await {
            CommandResult::Done => {}
            CommandResult::Reported | CommandResult::Transport(_) => {
                eprintln!(
                    "{}",
                    json!({"error": format!("script stopped at line {}", number + 1)})
                );
                return CommandResult::Reported;
            }
        }
    }
    stdout_line(&json!({"status": "script-done", "file": path}).to_string());
    CommandResult::Done
}

fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escape = false;
    let mut in_word = false;
    for point in line.chars() {
        if escape {
            current.push(point);
            escape = false;
            in_word = true;
            continue;
        }
        if point == '\\' && quote != Some('\'') {
            escape = true;
            continue;
        }
        if let Some(mark) = quote {
            if point == mark {
                quote = None;
            } else {
                current.push(point);
            }
            in_word = true;
            continue;
        }
        if point == '"' || point == '\'' {
            quote = Some(point);
            in_word = true;
            continue;
        }
        if point.is_whitespace() {
            if in_word {
                words.push(std::mem::take(&mut current));
                in_word = false;
            }
            continue;
        }
        current.push(point);
        in_word = true;
    }
    if in_word {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heroes_commands_parse_routes_and_require_positive_i64_ids() {
        let words = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
        for (args, post, path) in [
            (vec!["heroes", "list"], false, "/dev/heroes"),
            (vec!["heroes", "show", "42"], false, "/dev/heroes/42"),
            (
                vec!["heroes", "revisions", "42"],
                false,
                "/dev/heroes/42/revisions",
            ),
            (
                vec!["heroes", "delete", "42"],
                true,
                "/dev/heroes/42/delete",
            ),
        ] {
            assert_eq!(heroes_path(&words(&args)).unwrap(), (post, path.into()));
        }
        for args in [
            vec!["heroes"],
            vec!["heroes", "list", "1"],
            vec!["heroes", "show"],
            vec!["heroes", "show", "0"],
            vec!["heroes", "delete", "-1"],
            vec!["heroes", "revisions", "9223372036854775808"],
            vec!["heroes", "show", "1/other"],
            vec!["heroes", "unknown", "1"],
        ] {
            assert!(heroes_path(&words(&args)).is_err(), "{args:?}");
        }
    }

    #[test]
    fn photo_check_parses_a_saved_path_and_requires_a_closed_kind() {
        let positional = vec![
            "photo-check".into(),
            "C:\\saved images\\candidate.image".into(),
        ];
        for kind in ["canonical_room", "scene_plate", "character"] {
            assert_eq!(
                photo_check_path(&positional, &[("--kind".into(), kind.into())]).unwrap(),
                format!(
                    "/dev/photo-check?path=C%3A%5Csaved%20images%5Ccandidate.image&kind={kind}"
                )
            );
        }
        assert!(photo_check_path(&positional, &[]).is_err());
        assert!(photo_check_path(&positional, &[("--kind".into(), "room".into())]).is_err());
        assert!(photo_check_path(&["photo-check".into()], &[]).is_err());
    }

    #[test]
    fn server_url_must_be_loopback_http() {
        assert_eq!(
            parse_server("http://127.0.0.1:63359"),
            Ok(("127.0.0.1".to_owned(), 63359))
        );
        assert!(parse_server("https://127.0.0.1:63359").is_err());
        assert!(parse_server("http://192.168.0.24:63359").is_err());
        assert!(parse_server("http://127.0.0.1").is_err());
    }

    #[test]
    fn script_words_split_quotes_and_escapes() {
        assert_eq!(
            split_words("say Brynn \"hello there\""),
            ["say", "Brynn", "hello there"]
        );
        assert_eq!(split_words("say Brynn 'a b'"), ["say", "Brynn", "a b"]);
        assert_eq!(
            split_words("  act   Brynn  move  "),
            ["act", "Brynn", "move"]
        );
    }

    #[test]
    fn wait_conditions_read_plain_state() {
        let state =
            json!({"phase": "combat", "combat": {"active_actor": "Brynn", "finished": false}});
        assert!(condition_met(&state, &["turn", "brynn"]));
        assert!(!condition_met(&state, &["turn", "Neris"]));
        assert!(!condition_met(&state, &["fight-end"]));
        assert!(condition_met(&state, &["phase", "combat"]));
        let ended = json!({"phase": "complete", "combat": {"finished": true}});
        assert!(condition_met(&ended, &["fight-end"]));
    }
}

#[cfg(test)]
mod selection_flag_tests {
    use super::*;
    #[test]
    fn choices_preserve_all_values_and_identity_is_a_separate_object() {
        let mut body = json!({});
        selection_flags(
            &mut body,
            &[
                ("--choice".into(), "skill-1=perception".into()),
                ("--choice".into(), "skill-2=survival".into()),
                ("--identity-choice".into(), "pronouns=she-her".into()),
            ],
        )
        .unwrap();
        assert_eq!(body["choices"].as_object().unwrap().len(), 2);
        assert_eq!(body["identity_choices"]["pronouns"], "she-her");
        assert!(
            selection_flags(
                &mut body,
                &[("--choice".into(), "skill-1=athletics".into())]
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod act_reaction_tests {
    use super::*;

    #[test]
    fn act_body_includes_knockout_only_when_the_flag_is_given() {
        let body = act_body(
            &split_words("act Brakka attack"),
            &[("--knockout".to_owned(), "no".to_owned())],
        )
        .unwrap();
        assert_eq!(body["knockout"], "no");

        let body = act_body(&split_words("act Brakka attack"), &[]).unwrap();
        assert!(body.get("knockout").is_none());
    }

    #[test]
    fn reaction_answers_use_the_existing_player_action_kinds_and_weapon_option() {
        let take = act_body(
            &split_words("act Brakka reaction take"),
            &[("--weapon".to_owned(), "greatsword".to_owned())],
        )
        .unwrap();
        assert_eq!(take["hero"], "Brakka");
        assert_eq!(take["action"], "opportunity-attack");
        assert_eq!(take["option"], "greatsword");

        let decline = act_body(&split_words("act Brakka reaction decline"), &[]).unwrap();
        assert_eq!(decline["hero"], "Brakka");
        assert_eq!(decline["action"], "decline-reaction");
        assert!(decline.get("option").is_none());
        assert!(act_body(&split_words("act Brakka reaction"), &[]).is_err());
    }
}

#[cfg(test)]
mod dev_write_request_tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        split_words(text)
    }

    #[test]
    fn lore_add_names_scene_kind_and_name_and_keeps_the_content_words() {
        let body = lore_add_body(&words(
            "lore add current place Salt-Quay The quay runs along the wall.",
        ))
        .unwrap();
        assert_eq!(body["scene"], "current");
        assert_eq!(body["kind"], "place");
        assert_eq!(body["name"], "Salt-Quay");
        assert_eq!(body["content"], "The quay runs along the wall.");
        assert!(lore_add_body(&words("lore add current place")).is_err());
    }

    #[test]
    fn social_edit_and_economy_requests_name_their_target_and_refuse_missing_parts() {
        let edit =
            social_edit_body(&words("social edit merchant Brynn attitude friendly")).unwrap();
        assert_eq!(edit["op"], "attitude");
        assert_eq!(edit["npc"], "merchant");
        assert_eq!(edit["hero"], "Brynn");
        assert_eq!(edit["value"], "friendly");
        assert!(social_edit_body(&words("social edit merchant Brynn attitude")).is_err());
        assert!(social_edit_body(&words("social edit merchant")).is_err());

        let options = [("--cp".to_owned(), "200".to_owned())];
        let purse = economy_body(&words("economy purse merchant"), &options).unwrap();
        assert_eq!(purse["op"], "purse");
        assert_eq!(purse["cp"], 200);
        assert!(economy_body(&words("economy purse merchant"), &[]).is_err());
        assert!(
            economy_body(
                &words("economy coin Brynn"),
                &[("--cp".to_owned(), "two hundred".to_owned())]
            )
            .is_err()
        );
        let offer =
            economy_body(&words("economy offer restock item-clothes-travelers"), &[]).unwrap();
        assert_eq!(offer["target"], "item-clothes-travelers");
        assert_eq!(offer["op"], "offer-restock");
        assert!(economy_body(&words("economy"), &[]).is_err());
    }

    #[test]
    fn npc_say_act_and_hold_name_their_npc_and_refuse_missing_parts() {
        let say = npc_say_body(&words("npc say merchant Keep the lamp lit.")).unwrap();
        assert_eq!(say["npc"], "merchant");
        assert_eq!(say["text"], "Keep the lamp lit.");
        assert!(npc_say_body(&words("npc say merchant")).is_err());

        let options = [
            ("--target".to_owned(), "Brynn".to_owned()),
            ("--weapon".to_owned(), "scimitar".to_owned()),
        ];
        let act = npc_act_body(&words("npc act bandit attack"), &options).unwrap();
        assert_eq!(act["npc"], "bandit");
        assert_eq!(act["action"], "attack");
        assert_eq!(act["target"], "Brynn");
        assert_eq!(act["weapon"], "scimitar");
        assert!(act["to"].is_null());
        assert!(npc_act_body(&words("npc act bandit"), &[]).is_err());

        let hold = npc_hold_body(&words("npc hold bandit on")).unwrap();
        assert_eq!(hold["on"], true);
        assert_eq!(
            npc_hold_body(&words("npc hold bandit off")).unwrap()["on"],
            false
        );
        assert!(npc_hold_body(&words("npc hold bandit maybe")).is_err());
    }
}

#[cfg(test)]
mod table_setting_flag_tests {
    use super::*;

    #[test]
    fn table_hint_setting_sends_a_typed_boolean_and_rejects_an_invalid_value() {
        for hidden in [true, false] {
            let mut body = json!({});
            selection_flags(
                &mut body,
                &[("--choice-hints-hidden".into(), hidden.to_string())],
            )
            .unwrap();
            assert_eq!(body["choice_hints_hidden"], hidden);
        }
        assert!(
            selection_flags(
                &mut json!({}),
                &[("--choice-hints-hidden".into(), "maybe".into())]
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod battle_command_parse_tests {
    use super::*;

    #[test]
    fn fight_flags_use_the_same_split_options_as_dispatch() {
        let options = vec![
            ("--foe".to_owned(), "srd.bandit".to_owned()),
            ("--count".to_owned(), "2".to_owned()),
            ("--level".to_owned(), "5".to_owned()),
            ("--scene".to_owned(), "salt-quay".to_owned()),
        ];
        let (path, body) = fight_request(&split_words("fight start"), &options).unwrap();
        assert_eq!(path, "/dev/fight/edit");
        let body = body.unwrap();
        assert_eq!(body["count"], 2);
        assert_eq!(body["level"], 5);
        assert_eq!(body["op"], "start");
        for text in [
            "fight end victory",
            "fight end defeat",
            "fight end flee",
            "fight restore Brynn",
            "fight add wolf",
            "fight remove wolf",
            "fight scene salt-quay",
        ] {
            let (path, body) = fight_request(&split_words(text), &[]).unwrap();
            assert_eq!(path, "/dev/fight/edit");
            assert!(body.unwrap()["op"].is_string());
        }
        for (text, path) in [
            ("fight", "/dev/fight"),
            ("fight log", "/dev/fight/log?last=50"),
            ("fight offers Brynn", "/dev/fight/offers?hero=Brynn"),
            ("fight initiative", "/dev/fight/initiative"),
        ] {
            assert_eq!(
                fight_request(&split_words(text), &[]).unwrap(),
                (path.to_owned(), None)
            );
        }
        assert!(fight_request(&split_words("fight start"), &[]).is_err());
        let (_, body) = fight_request(
            &split_words("fight scene merchant-shop"),
            &[("--layout".to_owned(), "rack-aisle".to_owned())],
        )
        .unwrap();
        assert_eq!(body.unwrap()["layout"], "rack-aisle");
        let (_, body) = fight_request(
            &split_words("fight add wolf"),
            &[("--cell".to_owned(), "c3r4".to_owned())],
        )
        .unwrap();
        assert_eq!(body.unwrap()["cell"], "c3r4");
        assert_eq!(
            fight_request(
                &split_words("fight log"),
                &[("--last".to_owned(), "7".to_owned())]
            )
            .unwrap()
            .0,
            "/dev/fight/log?last=7"
        );
    }
    #[test]
    fn combat_overrides_parse_typed_values_without_treating_conditions_or_cells_as_numbers() {
        for (text, kind) in [
            ("override hp Brynn 3", "hp"),
            ("override budget 10", "budget"),
            ("override condition Brynn add prone", "condition"),
            ("override condition Brynn remove prone", "condition"),
            ("override cell Brynn c2r3", "cell"),
            ("override resource Brynn ammo 20", "resource"),
            ("override turn Brynn", "turn"),
            ("override initiative Brynn 15", "initiative"),
        ] {
            assert_eq!(override_body(&split_words(text)).unwrap()["kind"], kind);
        }
    }

    #[test]
    fn presentation_commands_use_the_dispatch_body() {
        for op in ["pause", "resume", "step", "replay"] {
            assert_eq!(
                presentation_body(&split_words(&format!("present {op}")), &[]).unwrap()["op"],
                op
            );
        }
        assert_eq!(
            presentation_body(&split_words("present speed 0.25"), &[]).unwrap()["value"],
            0.25
        );
        assert!(presentation_body(&split_words("present speed NaN"), &[]).is_err());
        assert_eq!(
            presentation_body(&split_words("present timing DICE_READ 500"), &[]).unwrap()["ms"],
            500
        );
        assert_eq!(
            presentation_body(&split_words("present timing reset"), &[]).unwrap()["op"],
            "timing-reset"
        );
    }
    #[test]
    fn npc_hold_all_is_a_table_scope_request() {
        let body = npc_hold_body(&split_words("npc hold all on")).unwrap();
        assert_eq!(body["npc"], "all");
        assert!(body.get("all").is_none());
        assert_eq!(body["on"], true);
    }
}

#[cfg(test)]
mod camera_command_tests {
    use super::*;
    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }
    #[test]
    fn camera_commands_parse_read_home_named_shot_save_and_reset() {
        assert!(camera_body(&words(&["camera"]), &[]).unwrap().is_none());
        let flags = [
            ("--pitch", "15"),
            ("--yaw", "-25"),
            ("--distance", "8"),
            ("--fov", "38"),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned()));
        assert_eq!(
            camera_body(&words(&["camera", "home"]), &flags)
                .unwrap()
                .unwrap(),
            json!({"command":"home","pitch":15.0,"yaw":-25.0,"distance":8.0,"fov":38.0})
        );
        assert_eq!(
            camera_body(&words(&["camera", "shot", "combat_est"]), &[])
                .unwrap()
                .unwrap()["shot"],
            "combat_est"
        );
        for command in ["save", "reset"] {
            assert_eq!(
                camera_body(&words(&["camera", command]), &[])
                    .unwrap()
                    .unwrap()["command"],
                command
            );
        }
        assert!(camera_body(&words(&["camera", "home"]), &[]).is_err());
        assert!(camera_body(&words(&["camera", "shot"]), &[]).is_err());
        let invalid = [("--pitch".to_owned(), "NaN".to_owned())];
        assert!(camera_body(&words(&["camera", "home"]), &invalid).is_err());
    }
}

#[cfg(test)]
mod fact_decision_tests {
    use super::*;

    #[test]
    fn hs50_ref069_dev_facts_lists_both_dispositions_and_filters_exact_npc() {
        let state = json!({"npc_facts":[
            {"npc_id":"bram","speaker":"Bram Weller","decision":"backfill","reason":"harmless_missing_detail","fact":{"content":"The Last Stitch"}},
            {"npc_id":"courier","speaker":"Finn","decision":"leave_blank","reason":"secret","fact":null}
        ]});
        let all = fact_decisions_document(&state, None);
        assert_eq!(all["count"], 2);
        assert_eq!(all["decisions"][1]["decision"], "leave_blank");
        let bram = fact_decisions_document(&state, Some("Bram Weller"));
        assert_eq!(bram["count"], 1);
        assert_eq!(bram["decisions"][0]["npc_id"], "bram");
        assert_eq!(fact_decisions_document(&state, Some("Bram"))["count"], 1);
        assert_eq!(fact_decisions_document(&state, Some("Br"))["count"], 0);
    }
}
