//! Bounded play driver using only the same dev views and admitted player commands as a tester.
use super::{Client, CommandResult, round_trip, url_escape};
use serde_json::{Value, json};
use std::time::Duration;

async fn request(
    client: &Client,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let encoded = body.map(|value| value.to_string());
    let (status, result) = round_trip(client, method, path, encoded.as_deref()).await?;
    if !(200..300).contains(&status) {
        return Err(result.to_string());
    }
    Ok(result)
}

pub(super) async fn run(client: &Client, flags: &[(String, String)]) -> CommandResult {
    let flag = |name| {
        flags
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let target = flag("--to").unwrap_or("ended");
    if ![
        "room", "creation", "opening", "dialogue", "combat", "complete", "ended",
    ]
    .contains(&target)
    {
        return client.print_error(
            "--to must be room, creation, opening, dialogue, combat, complete or ended",
        );
    }
    let maximum = match flag("--max-steps").unwrap_or("256").parse::<usize>() {
        Ok(count) if (1..=512).contains(&count) => count,
        _ => return client.print_error("--max-steps must be 1 to 512"),
    };
    let mut steps = Vec::new();
    match tokio::time::timeout(client.timeout, drive(client, target, maximum, &mut steps)).await {
        Ok(Ok(state)) => client.print(
            &json!({"status":"reached", "target":target, "state":state,"steps":steps}),
            false,
        ),
        Ok(Err(reason)) => client.print(
            &json!({"status":"play-stopped", "target":target,"reason":reason,"steps":steps}),
            true,
        ),
        Err(_) => client.print(
            &json!({"status":"play-timeout", "target":target,"steps":steps}),
            true,
        ),
    }
}

async fn drive(
    client: &Client,
    target: &str,
    maximum: usize,
    steps: &mut Vec<Value>,
) -> Result<Value, String> {
    loop {
        let state = request(client, "GET", "/dev/state", None).await?;
        if state["phase"] == target {
            return Ok(state);
        }
        if target == "dialogue"
            && ["combat", "complete", "ended"].contains(&state["phase"].as_str().unwrap_or(""))
        {
            return Err(format!(
                "dialogue target is behind the current {} phase",
                state["phase"].as_str().unwrap_or("unknown")
            ));
        }
        if steps.len() >= maximum {
            return Err("play exhausted --max-steps".to_owned());
        }
        let seats = state["seats"].as_array().ok_or("state has no seats")?;
        if seats.len() < 2 {
            let seat = (1..=2)
                .find(|seat| !seats.iter().any(|present| present["seat"] == *seat))
                .ok_or("seat projection invalid")?;
            let command = json!({"seat":seat.to_string()});
            let result = request(client, "POST", "/dev/join", Some(command.clone())).await?;
            if result["status"] != "joined" {
                return Err(result.to_string());
            }
            steps.push(json!({"command":"join", "input":command, "result":result}));
            continue;
        }
        let mut chosen = None;
        for seat in seats {
            let seat_id = seat["seat"].as_u64().ok_or("seat number absent")?;
            let hero = seat["hero"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| seat_id.to_string());
            let screen = request(
                client,
                "GET",
                &format!("/dev/view?as={}", url_escape(&hero)),
                None,
            )
            .await?;
            let view = &screen["view"];
            if target == "ended" && view["journey"]["adventure_ended"] == true {
                return Ok(state);
            }
            if view["journey"]["phase"] == target {
                return Ok(state);
            }
            if let Some((path, mut command)) = next_command(view, &hero, target) {
                if path == "/dev/create" {
                    command["seat"] = json!(seat_id.to_string());
                    command["name"] = json!(if seat_id == 1 { "Brakka" } else { "Sigrun" });
                    command["choices"] = default_creation_choices(view, seat_id as usize)?;
                }
                chosen = Some((path, command));
                break;
            }
        }
        if let Some((path, command)) = chosen {
            let result = request(client, "POST", path, Some(command.clone())).await?;
            if result["status"] != "accepted" {
                return Err(result.to_string());
            }
            steps.push(json!({"command":path,"input":command,"result":result}));
        }
        // Ordinary presentation gates still govern play; no artificial acknowledgements.
        tokio::time::sleep(Duration::from_millis(750)).await;
    }
}

fn default_creation_choices(view: &Value, seat: usize) -> Result<Value, String> {
    let groups = view["journey"]["creation"]["groups"]
        .as_array()
        .ok_or("creation groups are absent from the live view")?;
    if groups.is_empty() {
        return Err("creation groups are empty in the live view".to_owned());
    }
    let mut choices = serde_json::Map::new();
    let mut selected = Vec::<(String, String)>::with_capacity(groups.len());
    for group in groups {
        let group_id = group["group_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("creation group has no id")?;
        if choices.contains_key(group_id) {
            return Err(format!(
                "creation group {group_id:?} appears more than once"
            ));
        }
        if group_id == "portrait" {
            continue;
        }
        let options = group["options"]
            .as_array()
            .ok_or_else(|| format!("creation group {group_id:?} has no options"))?;
        let eligible = options
            .iter()
            .filter(|option| creation_dependency_is_satisfied(option, &selected))
            .filter(|option| {
                option["option_id"]
                    .as_str()
                    .is_some_and(|id| !duplicate_creation_option(group_id, id, &selected))
            })
            .collect::<Vec<_>>();
        let index = creation_option_index(group_id, seat, eligible.len());
        let option = eligible
            .get(index)
            .or_else(|| eligible.first())
            .copied()
            .ok_or_else(|| format!("creation group {group_id:?} has no valid default"))?;
        let option_id = option["option_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| format!("creation group {group_id:?} has an option without an id"))?;
        choices.insert(group_id.to_owned(), json!(option_id));
        selected.push((group_id.to_owned(), option_id.to_owned()));
    }
    Ok(Value::Object(choices))
}

fn creation_dependency_is_satisfied(option: &Value, selected: &[(String, String)]) -> bool {
    let Some(required_group) = option["requires_group"]
        .as_str()
        .filter(|group| !group.is_empty())
    else {
        return true;
    };
    let Some((_, selected_option)) = selected.iter().find(|(group, _)| group == required_group)
    else {
        return false;
    };
    option["requires_options"]
        .as_array()
        .is_none_or(|required_options| {
            required_options.is_empty()
                || required_options
                    .iter()
                    .any(|required| required.as_str() == Some(selected_option))
        })
}

fn duplicate_creation_option(group: &str, option: &str, selected: &[(String, String)]) -> bool {
    let category = match group {
        "language-1" | "language-2" => "language",
        "skill-1" | "skill-2" => "skill",
        _ => return false,
    };
    selected.iter().any(|(selected_group, selected_option)| {
        selected_option == option
            && match category {
                "language" => matches!(selected_group.as_str(), "language-1" | "language-2"),
                "skill" => matches!(selected_group.as_str(), "skill-1" | "skill-2"),
                _ => false,
            }
    })
}

fn creation_option_index(group: &str, seat: usize, option_count: usize) -> usize {
    if option_count <= 1 {
        return 0;
    }
    let seat_offset = seat.saturating_sub(1);
    if group == "build-package" {
        return seat_offset.min(option_count - 1);
    }
    if !matches!(group, "language-1" | "language-2" | "skill-1" | "skill-2") {
        return 0;
    }
    (seat_offset.saturating_mul(2)).min(option_count - 1)
}

fn next_command(view: &Value, hero: &str, target: &str) -> Option<(&'static str, Value)> {
    let offers = view["offers"].as_array()?;
    if offers.iter().any(|offer| offer["action"] == "create") {
        return Some(("/dev/create", json!({})));
    }
    let sheet = &view["journey"]["own_character"];
    let injured = sheet["hit_points"].as_u64()? < sheet["maximum_hit_points"].as_u64()?;
    let choose = |offer: &Value| {
        let mut choices = serde_json::Map::new();
        if let Some(groups) = offer["input_groups"].as_array() {
            for group in groups {
                let options = group["options"].as_array()?;
                let preferred = if group["group_id"] == "tactical-choice" {
                    tactical_option(view, offer, options)
                } else {
                    options
                        .iter()
                        .find(|option| {
                            if group["group_id"]
                                .as_str()
                                .is_some_and(|id| id.starts_with("hit-point-die"))
                            {
                                option["option_id"] == if injured { "spend" } else { "keep" }
                            } else {
                                option["option_id"] == "yes"
                            }
                        })
                        .or_else(|| options.first())
                }?;
                choices.insert(
                    group["group_id"].as_str()?.into(),
                    json!(preferred["option_id"].as_str()?),
                );
            }
        }
        Some((
            "/dev/act",
            json!({"hero":hero,"action":"choose","option":offer["offer_id"],"choices":choices}),
        ))
    };
    if target == "dialogue" {
        for offer in offers.iter().filter(|offer| offer["action"] == "ask") {
            if let Some(command) = choose(offer) {
                return Some(command);
            }
        }
    }
    for action in [
        "opportunity-attack",
        "decline-reaction",
        "second-wind",
        "stand",
        "pick-up",
        "attack",
        "move",
        "dash",
        "end-turn",
    ] {
        if target == "dialogue" && is_combat_action(action) {
            continue;
        }
        if action == "second-wind" && !injured {
            continue;
        }
        for offer in offers.iter().filter(|offer| offer["action"] == action) {
            if let Some(command) = choose(offer) {
                return Some(command);
            }
        }
    }
    for offer in offers {
        if target == "dialogue" && is_combat_action(offer["action"].as_str().unwrap_or("")) {
            continue;
        }
        if ["search", "new-adventure"]
            .iter()
            .any(|action| offer["action"] == *action)
        {
            continue;
        }
        if offer["action"] == "short-rest" && !injured {
            // A pending rest still needs its keep decision, even at full health.
            if !offer["input_groups"].as_array().is_some_and(|groups| {
                groups.iter().any(|group| {
                    group["group_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("hit-point-die"))
                })
            }) {
                continue;
            }
        }
        if let Some(command) = choose(offer) {
            return Some(command);
        }
    }
    None
}

fn is_combat_action(action: &str) -> bool {
    matches!(
        action,
        "defend"
            | "opportunity-attack"
            | "decline-reaction"
            | "second-wind"
            | "stand"
            | "pick-up"
            | "attack"
            | "move"
            | "dash"
            | "end-turn"
    )
}

fn cell(text: &str) -> Option<(i32, i32)> {
    let (col, row) = text.strip_prefix('c')?.split_once('r')?;
    Some((col.parse().ok()?, row.parse().ok()?))
}

fn tactical_option<'a>(view: &Value, offer: &Value, options: &'a [Value]) -> Option<&'a Value> {
    if offer["action"] != "move" {
        return options.first();
    }
    let combat = &view["journey"]["combat"];
    let enemies = combat["participants"]
        .as_array()?
        .iter()
        .filter(|actor| actor["side"] == 2 && actor["hit_points"].as_u64().unwrap_or(0) > 0)
        .filter_map(|actor| cell(actor["cell"].as_str()?))
        .collect::<Vec<_>>();
    let own = combat["participants"]
        .as_array()?
        .iter()
        .find(|actor| actor["name"] == combat["active_actor"])?;
    let here = cell(own["cell"].as_str()?)?;
    let distance = |(col, row): (i32, i32)| {
        enemies
            .iter()
            .map(|(x, y)| (x - col).abs().max((y - row).abs()))
            .min()
    };
    let current = distance(here)?;
    options
        .iter()
        .filter_map(|option| {
            let (col, row) = option["option_id"].as_str()?.split_once('-')?;
            let destination = (col.parse::<i32>().ok()? + 1, row.parse::<i32>().ok()? + 1);
            let remaining = distance(destination)?;
            (remaining < current).then_some((remaining, option))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, option)| option)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn play_uses_offered_reaction_and_never_a_future_story_action() {
        let view = json!({"offers":[{"action":"decline-reaction","offer_id":"reaction-1","input_groups":[]}],
            "journey":{"own_character":{"hit_points":13,"maximum_hit_points":13}}});
        let (path, input) = next_command(&view, "Sigrun", "ended").unwrap();
        assert_eq!(path, "/dev/act");
        assert_eq!(input["hero"], "Sigrun");
        assert_eq!(input["option"], "reaction-1");
    }
    #[test]
    fn play_spends_a_pending_hit_point_die_when_hurt() {
        let view = json!({"offers":[{"action":"short-rest","offer_id":"rest-1","input_groups":[
            {"group_id":"hit-point-die-brynn","options":[{"option_id":"keep"},{"option_id":"spend"}]}]}],
            "journey":{"own_character":{"hit_points":3,"maximum_hit_points":13}}});
        let (_, input) = next_command(&view, "Brynn", "ended").unwrap();
        assert_eq!(input["choices"]["hit-point-die-brynn"], "spend");
    }

    #[test]
    fn creation_defaults_are_complete_distinct_and_dependency_aware() {
        let options = |ids: &[&str]| {
            ids.iter()
                .map(|id| json!({"option_id":id,"requires_group":"","requires_options":[]}))
                .collect::<Vec<_>>()
        };
        let view = json!({"journey":{"creation":{"groups":[
            {"group_id":"species","options":options(&["dwarf"])},
            {"group_id":"language-1","options":options(&["common-sign","draconic","dwarvish","elvish"])},
            {"group_id":"language-2","options":options(&["common-sign","draconic","dwarvish","elvish"])},
            {"group_id":"skill-1","options":options(&["acrobatics","history","insight","survival"])},
            {"group_id":"skill-2","options":options(&["acrobatics","history","insight","survival"])},
            {"group_id":"build-package","options":options(&["stalwart","vanguard"])},
            {"group_id":"portrait","options":[
                {"option_id":"portrait-hero-1","requires_group":"build-package","requires_options":["stalwart"]},
                {"option_id":"portrait-hero-2","requires_group":"build-package","requires_options":["vanguard"]}
            ]}
        ]}}});

        let first = default_creation_choices(&view, 1).unwrap();
        let second = default_creation_choices(&view, 2).unwrap();
        assert_eq!(first.as_object().unwrap().len(), 6);
        assert_eq!(second.as_object().unwrap().len(), 6);
        assert_eq!(first["build-package"], "stalwart");
        assert!(first.get("portrait").is_none());
        assert_eq!(second["build-package"], "vanguard");
        assert!(second.get("portrait").is_none());
        for (choices, first_group, second_group) in [
            (&first, "language-1", "language-2"),
            (&first, "skill-1", "skill-2"),
            (&second, "language-1", "language-2"),
            (&second, "skill-1", "skill-2"),
        ] {
            assert_ne!(choices[first_group], choices[second_group]);
        }
        assert_ne!(first["language-1"], second["language-1"]);
        assert_ne!(first["skill-1"], second["skill-1"]);
    }

    #[test]
    fn non_tactical_creation_defaults_omit_stale_portrait_groups() {
        let view = json!({"journey":{"creation":{"groups":[
            {"group_id":"species","options":[{"option_id":"dwarf"}]},
            {"group_id":"portrait","options":[
                {"option_id":"portrait-hero-1","requires_group":"","requires_options":[]},
                {"option_id":"portrait-hero-2","requires_group":"","requires_options":[]}
            ]}
        ]}}});

        let first = default_creation_choices(&view, 1).unwrap();
        let second = default_creation_choices(&view, 2).unwrap();

        assert!(first.get("portrait").is_none());
        assert!(second.get("portrait").is_none());
    }

    #[test]
    fn dialogue_target_chooses_ask_even_when_the_combat_offer_comes_first() {
        let view = json!({"offers":[
            {"action":"defend","offer_id":"defend-courier","input_groups":[]},
            {"action":"ask","offer_id":"ask-courier","input_groups":[]}
        ], "journey":{"own_character":{"hit_points":13,"maximum_hit_points":13}}});

        let (path, input) = next_command(&view, "Brakka", "dialogue").unwrap();

        assert_eq!(path, "/dev/act");
        assert_eq!(input["option"], "ask-courier");
    }

    #[test]
    fn dialogue_target_waits_instead_of_submitting_a_combat_offer() {
        let view = json!({"offers":[
            {"action":"defend","offer_id":"defend-courier","input_groups":[]},
            {"action":"attack","offer_id":"attack-bandit","input_groups":[]}
        ], "journey":{"own_character":{"hit_points":13,"maximum_hit_points":13}}});

        assert!(next_command(&view, "Brakka", "dialogue").is_none());
    }

    #[tokio::test]
    async fn play_stops_after_the_configured_number_of_admitted_steps() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                loop {
                    let count = stream.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap();
                let path = request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let document = if path == "/dev/join" {
                    json!({"status":"joined"})
                } else {
                    json!({"phase":"room","seats":[]})
                };
                let body = document.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = Client {
            host: "127.0.0.1".to_owned(),
            port: address.port(),
            token: "test-token".to_owned(),
            pretty: false,
            timeout: Duration::from_secs(2),
        };
        let mut steps = Vec::new();

        let result = drive(&client, "dialogue", 1, &mut steps).await;

        assert_eq!(result.unwrap_err(), "play exhausted --max-steps");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0]["command"], "join");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn play_resume_at_dialogue_returns_without_submitting_an_action() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 2048];
            loop {
                let count = stream.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let body = json!({"phase":"dialogue","seats":[
                {"seat":1,"hero":"Brakka"},{"seat":2,"hero":"Sigrun"}
            ]})
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let client = Client {
            host: "127.0.0.1".to_owned(),
            port: address.port(),
            token: "test-token".to_owned(),
            pretty: false,
            timeout: Duration::from_secs(2),
        };
        let mut steps = Vec::new();

        let state = drive(&client, "dialogue", 1, &mut steps).await.unwrap();

        assert_eq!(state["phase"], "dialogue");
        assert!(steps.is_empty());
        server.await.unwrap();
    }
}
