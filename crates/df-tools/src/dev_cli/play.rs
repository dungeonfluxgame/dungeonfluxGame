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
    let drive = async {
        loop {
            let state = request(client, "GET", "/dev/state", None).await?;
            if state["phase"] == target {
                return Ok(state);
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
                let hero = seat["hero"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| seat["seat"].to_string());
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
                if let Some((path, mut command)) = next_command(view, &hero) {
                    if path == "/dev/create" {
                        let seat_id = seat["seat"].as_u64().ok_or("seat number absent")?;
                        command["seat"] = json!(seat_id.to_string());
                        command["name"] = json!(if seat_id == 1 { "Brakka" } else { "Sigrun" });
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
    };
    match tokio::time::timeout(client.timeout, drive).await {
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

fn next_command(view: &Value, hero: &str) -> Option<(&'static str, Value)> {
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
    #[test]
    fn play_uses_offered_reaction_and_never_a_future_story_action() {
        let view = json!({"offers":[{"action":"decline-reaction","offer_id":"reaction-1","input_groups":[]}],
            "journey":{"own_character":{"hit_points":13,"maximum_hit_points":13}}});
        let (path, input) = next_command(&view, "Sigrun").unwrap();
        assert_eq!(path, "/dev/act");
        assert_eq!(input["hero"], "Sigrun");
        assert_eq!(input["option"], "reaction-1");
    }
    #[test]
    fn play_spends_a_pending_hit_point_die_when_hurt() {
        let view = json!({"offers":[{"action":"short-rest","offer_id":"rest-1","input_groups":[
            {"group_id":"hit-point-die-brynn","options":[{"option_id":"keep"},{"option_id":"spend"}]}]}],
            "journey":{"own_character":{"hit_points":3,"maximum_hit_points":13}}});
        let (_, input) = next_command(&view, "Brynn").unwrap();
        assert_eq!(input["choices"]["hit-point-die-brynn"], "spend");
    }
}
