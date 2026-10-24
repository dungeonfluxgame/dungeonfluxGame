//! Request construction for roleplay inspection and explicit media commands.
use super::{Client, CommandResult, get, option_value, post, url_escape};
use serde_json::{Value, json};

type Request = (String, Option<Value>);
fn query(path: &str, options: &[(String, String)], names: &[&str]) -> Result<String, String> {
    let mut path = path.to_owned();
    for name in names {
        if let Some(value) = option_value(options, &format!("--{name}")) {
            if *name == "since" && value.parse::<u64>().is_err() {
                return Err("--since must be an unsigned sequence".into());
            }
            path.push(if path.contains('?') { '&' } else { '?' });
            path.push_str(name);
            path.push('=');
            path.push_str(&url_escape(&value));
        }
    }
    Ok(path)
}
pub(super) fn handles(words: &[String]) -> bool {
    matches!(
        words.first().map(String::as_str),
        Some("transcript" | "trade-terms" | "voice" | "mood" | "narrative" | "screen" | "music")
    ) || (words.first().is_some_and(|word| word == "npc")
        && words
            .get(1)
            .is_some_and(|word| matches!(word.as_str(), "last" | "show")))
        || (words.first().is_some_and(|word| word == "present") && words.len() == 1)
}
fn request(words: &[String], options: &[(String, String)]) -> Result<Request, String> {
    let command = words.first().map(String::as_str).unwrap_or("");
    let valid_length = match command {
        "transcript" | "voice" | "mood" | "narrative" | "present" => words.len() == 1,
        "trade-terms" | "screen" => words.len() == 2,
        "npc" => words.len() == 3,
        "music" => words.len() <= 3,
        _ => false,
    };
    if !valid_length {
        return Err("invalid or missing roleplay command arguments; see dev help".into());
    }
    for (key, _) in options {
        if matches!(key.as_str(), "--server" | "--token-file" | "--timeout") {
            continue;
        }
        let allowed = match command {
            "transcript" => matches!(key.as_str(), "--since" | "--hero" | "--npc" | "--out"),
            "trade-terms" => key == "--npc",
            "npc" => key == "--hero",
            "music" => key == "--spec" && words.get(1).is_some_and(|op| op == "override"),
            _ => false,
        };
        if !allowed {
            return Err(format!("{command} does not take {key}"));
        }
    }
    let required = |index: usize, message: &str| {
        words
            .get(index)
            .filter(|word| !word.trim().is_empty())
            .cloned()
            .ok_or_else(|| message.to_owned())
    };
    let request = match words.first().map(String::as_str) {
        Some("transcript") => (
            query("/dev/transcript", options, &["since", "hero", "npc"])?,
            None,
        ),
        Some("trade-terms") => (
            query(
                &format!(
                    "/dev/trade-terms?hero={}",
                    url_escape(&required(1, "trade-terms needs a hero")?)
                ),
                options,
                &["npc"],
            )?,
            None,
        ),
        Some(command @ ("voice" | "mood" | "narrative")) if words.len() == 1 => {
            (format!("/dev/{command}"), None)
        }
        Some("present") if words.len() == 1 => ("/dev/presentation".into(), None),
        Some("screen") => {
            let seat = required(1, "screen needs seat 1 to 6")?;
            if !seat.parse::<u8>().is_ok_and(|seat| (1..=6).contains(&seat)) {
                return Err("screen needs seat 1 to 6".into());
            }
            ("/dev/screens/ticket".into(), Some(json!({"seat":seat})))
        }
        Some("npc") => {
            let op = required(1, "npc needs last or show")?;
            if !matches!(op.as_str(), "last" | "show") {
                return Err("npc needs last or show".into());
            }
            let name = required(2, "npc last/show needs an NPC")?;
            (
                query(
                    &format!("/dev/npc/{op}?name={}", url_escape(&name)),
                    options,
                    &["hero"],
                )?,
                None,
            )
        }
        Some("music") => match words.get(1).map(String::as_str) {
            Some(op @ ("regenerate" | "clear" | "request" | "override" | "clear-override")) => {
                let place = words.get(2).cloned().unwrap_or_else(|| "current".into());
                let mut body = json!({"op":op,"place":place});
                if op == "override" {
                    let spec = option_value(options, "--spec")
                        .ok_or("music override needs --spec <pack music JSON>")?;
                    body["spec"] =
                        serde_json::from_str(&spec).map_err(|_| "--spec must be JSON")?;
                }
                ("/dev/music/edit".into(), Some(body))
            }
            place if words.len() <= 2 => (
                format!(
                    "/dev/music?place={}",
                    url_escape(place.unwrap_or("current"))
                ),
                None,
            ),
            _ => return Err("music takes [place|list] or an explicit edit operation".into()),
        },
        _ => return Err("invalid roleplay command or extra arguments".into()),
    };
    Ok(request)
}
pub(super) async fn run(
    client: &Client,
    words: &[String],
    options: &[(String, String)],
) -> CommandResult {
    let (path, body) = match request(words, options) {
        Ok(request) => request,
        Err(error) => return client.print_error(error),
    };
    if words.first().is_some_and(|word| word == "transcript")
        && let Some(file) = option_value(options, "--out")
    {
        return match super::round_trip(client, "GET", &path, None).await {
            Ok((status, document)) if (200..300).contains(&status) => {
                let bytes = match serde_json::to_vec_pretty(&document) {
                    Ok(bytes) => bytes,
                    Err(error) => return client.print_error(error.to_string()),
                };
                match tokio::fs::write(&file,bytes).await {
                    Ok(()) => client.print(&json!({"status":"saved","file":file,"count":document.as_array().map(Vec::len)}),false),
                    Err(error)=>client.print_error(format!("cannot write transcript: {error}")),
                }
            }
            Ok((_, document)) => client.print(&document, true),
            Err(error) => CommandResult::Transport(error),
        };
    }
    match body {
        Some(body) => post(client, &path, &body).await,
        None => get(client, &path).await,
    }
}
pub(super) fn wait_seconds(options: &[(String, String)]) -> Result<Option<u64>, String> {
    let Some(value) = option_value(options, "--for") else {
        return Ok(None);
    };
    if option_value(options, "--until").is_some() {
        return Err("wait takes --for or --until, not both".into());
    }
    let seconds = value
        .parse::<u64>()
        .map_err(|_| "--for must be 1 to 86400 seconds")?;
    if !(1..=86400).contains(&seconds) {
        return Err("--for must be 1 to 86400 seconds".into());
    }
    Ok(Some(seconds))
}
pub(super) fn help() {
    super::stdout_line(
        "Roleplay commands (JSON; server refusals include reason and edited:false):\n\
transcript [--since seq] [--hero name] [--npc name] [--out file] -> [{unix_ms,seq,place,speaker,role,text,exchange_id,reply_ms,voice{start,end},subtitle{shown,hidden}}]; timings null until observed; --out -> {status,file,count}\n\
trade-terms <hero> [--npc merchant] -> [{direction:buys|sells,item,merchant,min_cp,max_cp,available,stock_rev,purse_rev}]\n\
npc show <npc> [--hero name] -> {npc,npc_id,dossier,profile,relationships[{hero,attitude,trust,emotion,details,grievances}]}\n\
npc last <npc> [--hero name] -> {npc_id,exchange,decisions,hero_line,line_route,emotion_before,checks,judgement,reply,timings}; uncorrelated observations null; --hero returns unavailable with reason and recent conversation memory\n\
music [place|list] -> {place,source,spec,set_key,tracks[{track,status,detail}]} or {places}; pure read\n\
music regenerate|clear|request|clear-override [place]; music override [place] --spec <JSON> -> {status,op,place,edited}; unknown/undecided place refused\n\
voice -> [{speaker,voice_id,lines[{key,state,ms}]}]; mood -> {mood,shot,lighting,reason,faces}; unknown observations null\n\
narrative -> {phase,beat,budget,spend}; present -> {presentation,edited:false,display_only:true}\n\
screen <1..6> -> {status:issued,path,expires_seconds}; issues a one-use ticket; requires joined seat\n\
place travel <destination words> | place go landmark:<name> -> {status,from,to,place,created,revision,edited}; owner validates and commits\n\
wait --for <1..86400 seconds> -> {status:waited,seconds}; wall time only, no game-time mutation\n\
timeline|audit|decisions --since <seq> [--npc name] -> existing shape + next_since,oldest_available,cursor_gap; cursor returns next page\n",
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }
    #[test]
    fn roleplay_reads_build_real_routes_and_preserve_escaped_filters() {
        for (command, path) in [
            ("voice", "/dev/voice"),
            ("mood", "/dev/mood"),
            ("narrative", "/dev/narrative"),
            ("present", "/dev/presentation"),
            ("music", "/dev/music?place=current"),
            ("npc show Marek", "/dev/npc/show?name=Marek"),
            ("npc last Marek", "/dev/npc/last?name=Marek"),
            ("trade-terms Brynn", "/dev/trade-terms?hero=Brynn"),
        ] {
            assert_eq!(
                request(&words(command), &[]).unwrap(),
                (path.to_owned(), None)
            );
        }
        assert_eq!(
            request(
                &words("transcript"),
                &[
                    ("--since".into(), "4".into()),
                    ("--npc".into(), "Marek & Co".into())
                ]
            )
            .unwrap()
            .0,
            "/dev/transcript?since=4&npc=Marek%20%26%20Co"
        );
        assert!(request(&words("trade-terms"), &[]).is_err());
        assert!(request(&words("npc last"), &[]).is_err());
        assert!(request(&words("transcript"), &[("--since".into(), "bad".into())]).is_err());
    }
    #[test]
    fn media_and_ticket_mutations_use_explicit_posts() {
        for op in ["clear", "regenerate", "request", "clear-override"] {
            let (path, body) = request(&words(&format!("music {op} shop")), &[]).unwrap();
            assert_eq!(path, "/dev/music/edit");
            assert_eq!(body.unwrap(), json!({"op":op,"place":"shop"}));
        }
        assert!(request(&words("music override shop"), &[]).is_err());
        assert_eq!(
            request(&words("screen 6"), &[]).unwrap().1.unwrap()["seat"],
            "6"
        );
        assert!(request(&words("screen 7"), &[]).is_err());
        assert_eq!(
            wait_seconds(&[("--for".into(), "1800".into())]).unwrap(),
            Some(1800)
        );
        assert!(wait_seconds(&[("--for".into(), "0".into())]).is_err());
    }
}
