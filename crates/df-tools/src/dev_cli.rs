//! Agent command line for the development control surface (DEVCTL-001).
//!
//! `df-transport-fixture dev ...` talks to a game server started with
//! `DF_GAMEPLAY_DEV_CONTROL=on`. It prints one JSON document per command on
//! standard output (compact by default, `--pretty` for people), writes errors
//! as JSON on standard error with a non-zero exit code, and never prompts.

use std::{ffi::OsString, time::Duration};

#[path = "dev_cli/play.rs"]
mod play;

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
        } else if matches!(word.as_str(), "--keep" | "--wait") {
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
    stdout_line(
        "dev: control a development game server (DF_GAMEPLAY_DEV_CONTROL=on)\n\
         \n\
         usage: df-transport-fixture dev [--server <url> --token-file <path>] [--pretty] [--timeout <s>] <command> [args]\n\
         \n\
         health                       server liveness and current revision\n\
         join <seat>                  join the room as seat 1 or 2\n\
         reclaim <seat>               take an away hero back on this control seat\n\
         create <seat> <name>         --choice group=id (repeatable), --identity <JSON>, --identity-choice group=id\n\
         preview <seat>              --appearance <text> --backstory <text>; review returned identity JSON\n\
         act <hero> <action> [...]    play one action: --cell c5r6 --target <foe> --weapon <name> --option <id>\n\
         say <hero> <text>            speak one line; [--context question|social|plan|meta|joke]\n\
         direct plate <text>          re-render the current scene picture; [--keep] [--wait]\n\
         direct clear                 clear held directions and return to the ordinary plate\n\
         state                        room, phase, seats, fight, offers, revision\n\
         view --as tv|<hero>          exactly the view that screen is being sent\n\
         notes <hero>                 print the notebook as that hero sees it\n\
         note <hero> <text>           add a shared party note\n\
         scene <hero>                current contents and item card\n\
         feelings <npc>               current development-only NPC diagnostic\n\
         timeline [--last <n>]         last N audit events with wall and elapsed times\n\
         play --to <phase>            play to room|creation|opening|dialogue|combat|complete|ended\n\
         log [--since <seq>]          the table log and the override log\n\
         wait --until <condition>     turn <hero> | fight-end | phase <phase>\n\
         script <file>                run one dev command per line, stop at first failure\n\
         checkpoint save <name>       name the current point in the session\n\
         checkpoint list              named points saved so far\n\
         restore <name>               return to a named point (new revision)\n\
         back [n]                     state before the last n decisions\n\
         forward [n]                  re-apply stepped-over states\n\
         override hp <hero> <n>       set hit points within bounds (marks edited)\n\
         override budget <n>          set narrative budget within pack cap\n\
         shutdown                     clean stop; the database resumes immediately\n\
         describe                     every command, its arguments and output shapes\n\
         \n\
         override is temporarily disabled server-side\n\
         (direct edits need the milestone 3 proof); see dev describe.\n\
         \n\
         actions: begin ask escort defend move attack second-wind end-turn dash dodge\n\
                  disengage stand pick-up short-rest inn resolve search new-adventure\n\
                  opportunity-attack decline-reaction stabilise-kit stabilise-medicine table-setting\n\
         choose any offer: act <hero> <label-or-id>, or act <hero> choose --option <label-or-id>\n\
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
    match positional.first().map(String::as_str).unwrap_or("") {
        "health" => get(client, "/dev/health").await,
        "join" => {
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
            let mut body = json!({
                "hero": positional.get(1).cloned().unwrap_or_default(),
                "action": positional.get(2).cloned().unwrap_or_default(),
            });
            for (key, value) in [
                ("cell", "--cell"),
                ("target", "--target"),
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
        "state" => get(client, "/dev/state").await,
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
        "timeline" => get(client, &format!("/dev/timeline?last={}", url_escape(&flag("--last").or_else(|| positional.get(1).cloned()).unwrap_or_else(|| "50".into())))).await,
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
            let kind = positional.get(1).cloned().unwrap_or_default();
            let mut body = json!({"kind": kind});
            if let Some(hero) = positional.get(2) {
                body["hero"] = Value::String(hero.clone());
            }
            if let Some(value) = flag("--value").or_else(|| positional.get(3).cloned()) {
                match value.parse::<i64>() {
                    Ok(value) => body["value"] = json!(value),
                    Err(_) => return client.print_error("override value must be a number"),
                }
            }
            post(client, "/dev/override", &body).await
        }
        "shutdown" => post(client, "/dev/shutdown", &json!({})).await,
        "describe" => get(client, "/dev/describe").await,
        other => client.print_error(format!("unknown dev command {other:?}; see dev --help")),
    }
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
        .map_err(|_| "server read timed out".to_owned())?
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
