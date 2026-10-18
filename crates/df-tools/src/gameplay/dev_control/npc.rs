//! `dev npc`: an NPC speaks one line, takes its fight turn on the CLI's order, or holds its turn
//! for the CLI. A line goes into the public table log the way a generated NPC line does, so the
//! TV, the phones and story memory see it as that NPC. A turn is resolved by the same rules the
//! enemy policy uses and committed through the owner's development path, which marks the run
//! edited. Every write is audited with one line.

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, PoisonError},
};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use df_protocol::common as rpc;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Outcome, Shared, authorized, bad_request, failed};
use crate::gameplay::{
    actor, beat_script_runtime, journey, pack::Scene, table_log::LINE_CHARACTERS,
};

/// The NPCs whose turns wait for the CLI, by lowercase display name. Process-wide and dev-only:
/// a restarted server starts with no holds.
static HOLDS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn hold_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// Whether the CLI holds this NPC's turns: the policy must not act for it.
pub(crate) fn held(name: &str) -> bool {
    HOLDS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .any(|held| held == "all" || *held == hold_key(name))
}

/// Sets or clears the hold on one NPC.
pub(crate) fn set_hold(name: &str, on: bool) {
    let mut holds = HOLDS.lock().unwrap_or_else(PoisonError::into_inner);
    if on {
        holds.insert(hold_key(name));
    } else if hold_key(name) == "all" {
        holds.clear();
    } else {
        holds.remove(&hold_key(name));
    }
}

/// The pure choice an enemy turn makes: a held NPC's turn is left to the CLI and the policy is
/// never consulted for it; otherwise the policy decides. The hold is read before the policy runs.
pub(crate) fn choose_turn<T, E>(
    held: bool,
    policy: impl FnOnce() -> Result<T, E>,
) -> Result<Option<T>, E> {
    if held { Ok(None) } else { policy().map(Some) }
}

/// One fight action the CLI can take for an NPC, including an explicit ordinary policy step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NpcAct {
    Attack {
        target: String,
        weapon: Option<String>,
    },
    Move {
        cell: (u16, u16),
    },
    Dash,
    Dodge,
    Disengage,
    EndTurn,
    Policy,
    Development(super::fight::BattleCommand),
    Opportunity {
        weapon: Option<String>,
    },
    DeclineReaction,
    NimbleDisengage,
    Stand,
    DropProne,
    DoffShield,
    DeathSave,
}

impl NpcAct {
    /// Reads `dev npc act <npc> <action> [--target <id>] [--weapon <name>] [--to <col,row>]`.
    pub(crate) fn parse(
        action: &str,
        target: Option<&str>,
        weapon: Option<&str>,
        to: Option<&str>,
    ) -> Result<Self, String> {
        let refuse_extra = |name: &str, given: bool| {
            if given {
                Err(format!("--{name} does not apply to {action}"))
            } else {
                Ok(())
            }
        };
        match action {
            "attack" => {
                refuse_extra("to", to.is_some())?;
                let target = target
                    .filter(|target| !target.trim().is_empty())
                    .ok_or_else(|| "npc act attack needs --target <foe>".to_owned())?;
                Ok(Self::Attack {
                    target: target.trim().to_owned(),
                    weapon: weapon.map(|weapon| weapon.trim().to_owned()),
                })
            }
            "move" => {
                refuse_extra("target", target.is_some())?;
                refuse_extra("weapon", weapon.is_some())?;
                let text = to.ok_or_else(|| "npc act move needs --to <col,row>".to_owned())?;
                let cell = text
                    .split_once(',')
                    .and_then(|(col, row)| {
                        Some((
                            col.trim().parse::<u16>().ok()?,
                            row.trim().parse::<u16>().ok()?,
                        ))
                    })
                    .ok_or_else(|| "--to must be <col,row> in whole grid cells".to_owned())?;
                Ok(Self::Move { cell })
            }
            "opportunity" | "opportunity-attack" => {
                refuse_extra("target", target.is_some())?;
                refuse_extra("to", to.is_some())?;
                Ok(Self::Opportunity { weapon: weapon.map(str::to_owned) })
            }
            "dash" | "dodge" | "disengage" | "end-turn" | "decline-reaction" | "nimble-disengage" | "stand" | "drop-prone" | "policy" | "doff-shield" | "death-save" => {
                refuse_extra("target", target.is_some())?;
                refuse_extra("weapon", weapon.is_some())?;
                refuse_extra("to", to.is_some())?;
                Ok(match action {
                    "dash" => Self::Dash,
                    "dodge" => Self::Dodge,
                    "disengage" => Self::Disengage,
                    "decline-reaction" => Self::DeclineReaction,
                    "nimble-disengage" => Self::NimbleDisengage,
                    "stand" => Self::Stand,
                    "drop-prone" => Self::DropProne,
                    "policy" => Self::Policy,
                    "doff-shield" => Self::DoffShield,
                    "death-save" => Self::DeathSave,
                    _ => Self::EndTurn,
                })
            }
            _ => Err("npc act takes attack, move, dash, dodge, disengage, nimble-disengage, stand, drop-prone, doff-shield, death-save, opportunity, decline-reaction, end-turn or policy".to_owned()),
        }
    }

    pub(crate) fn label(&self) -> String {
        match self {
            Self::Attack { target, weapon } => match weapon {
                Some(weapon) => format!("attack {target} with {weapon}"),
                None => format!("attack {target}"),
            },
            Self::Move { cell: (col, row) } => format!("move to {col},{row}"),
            Self::Dash => "dash".to_owned(),
            Self::Dodge => "dodge".to_owned(),
            Self::Disengage => "disengage".to_owned(),
            Self::EndTurn => "end-turn".to_owned(),
            Self::Policy => "policy turn".to_owned(),
            Self::Development(command) => format!("development {command:?}"),
            Self::Opportunity { weapon } => format!("opportunity {weapon:?}"),
            Self::DeclineReaction => "decline-reaction".to_owned(),
            Self::NimbleDisengage => "nimble-disengage".to_owned(),
            Self::Stand => "stand".to_owned(),
            Self::DropProne => "drop-prone".to_owned(),
            Self::DoffShield => "doff-shield".to_owned(),
            Self::DeathSave => "death-save".to_owned(),
        }
    }
}

/// The text of one spoken line: whitespace collapsed, non-empty, within the table's line size.
pub(crate) fn say_text(text: &str) -> Result<String, String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return Err("npc say needs the line the NPC speaks".to_owned());
    }
    if text.chars().count() > LINE_CHARACTERS {
        return Err(format!(
            "a spoken line is at most {LINE_CHARACTERS} characters"
        ));
    }
    Ok(text)
}

/// The person a requested id or name names, case-insensitively; an id wins over a name.
pub(crate) fn find_person<'a>(
    people: &'a [(String, String)],
    requested: &str,
) -> Option<&'a (String, String)> {
    people
        .iter()
        .find(|(id, _)| id.eq_ignore_ascii_case(requested))
        .or_else(|| {
            people
                .iter()
                .find(|(_, name)| name.eq_ignore_ascii_case(requested))
        })
}

/// The NPC a `say` names, provided it is on stage; otherwise the reason it cannot speak.
pub(crate) fn speaker_on_stage<'a>(
    people: &'a [(String, String)],
    requested: &str,
) -> Result<&'a (String, String), &'static str> {
    find_person(people, requested).ok_or("the NPC is not present in the current scene")
}

/// The people on stage now as (id, name): the scene cast, then the scene's own people.
fn people_on_stage(scene: &Scene) -> Vec<(String, String)> {
    let mut people: Vec<(String, String)> = scene
        .cast
        .iter()
        .map(|person| (person.id.clone(), person.name.clone()))
        .collect();
    if let Some(model) = scene
        .setting
        .as_ref()
        .and_then(|setting| setting.model.as_ref())
    {
        people.extend(
            model
                .people
                .iter()
                .map(|person| (person.id.clone(), person.name.clone())),
        );
    }
    people
}

/// The display name a requested NPC id or name stands for; the text itself when nobody on stage
/// answers to it (a fight's enemy may be named only by the fight).
fn display_name(scene: &Scene, requested: &str) -> String {
    find_person(&people_on_stage(scene), requested)
        .map_or_else(|| requested.trim().to_owned(), |(_, name)| name.clone())
}

fn current_scene(state: &Shared) -> Result<Scene, (StatusCode, Json<Value>)> {
    let checkpoint = state.service.updates.borrow().clone();
    beat_script_runtime::scene(checkpoint.state()).map_err(|_| failed("current scene unavailable"))
}

fn npc_name(requested: &str) -> Result<String, (StatusCode, Json<Value>)> {
    let text = requested.trim();
    if text.is_empty() || text.chars().count() > 64 || text.chars().any(char::is_control) {
        return Err(bad_request("an NPC is named by 1 to 64 characters"));
    }
    Ok(text.to_owned())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SayBody {
    npc: String,
    text: String,
}

/// `POST /dev/npc/say {"npc", "text"}`: the NPC speaks one line into the public table log. The
/// NPC must be on stage in the current scene; otherwise the line is refused and nothing is said.
pub(super) async fn say(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<SayBody>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    super::require_gameplay_overrides()?;
    let requested = npc_name(&body.npc)?;
    let text = say_text(&body.text).map_err(bad_request)?;
    let scene = current_scene(&state)?;
    let people = people_on_stage(&scene);
    let (id, name) = match speaker_on_stage(&people, &requested) {
        Ok((id, name)) => (id.clone(), name.clone()),
        Err(reason) => {
            actor::record_audit_line(
                "gameplay.dev.npc_say",
                format!("DEV-NPC say status=refused npc={requested:?} reason={reason:?}"),
            );
            return Ok((
                StatusCode::OK,
                Json(json!({
                    "status": "refused",
                    "npc": requested,
                    "scene": scene.title,
                    "reason": reason,
                    "edited": false,
                })),
            ));
        }
    };
    let sequence = state.table.push_actor(
        &id,
        &name,
        rpc::TableRole::Npc,
        0,
        rpc::TableKind::Speech,
        &text,
    );
    actor::record_audit_line(
        "gameplay.dev.npc_say",
        format!(
            "DEV-NPC say status=said npc={name:?} npc_id={id:?} chars={} sequence={}",
            text.chars().count(),
            sequence.unwrap_or(0)
        ),
    );
    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "said",
            "npc": name,
            "npc_id": id,
            "text": text,
            "sequence": sequence,
            "edited": false,
        })),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HoldBody {
    npc: String,
    on: bool,
}

/// `POST /dev/npc/hold {"npc", "on"}`: while held, the enemy policy leaves this NPC's fight turns
/// to the CLI and the battle waits on them. Releasing a hold on the NPC whose turn is waiting lets
/// the policy take that turn now; the result is the committed turn.
pub(super) async fn hold(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<HoldBody>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    super::require_gameplay_overrides()?;
    let name = display_name(&current_scene(&state)?, &npc_name(&body.npc)?);
    set_hold(&name, body.on);
    let waiting = {
        let checkpoint = state.service.updates.borrow().clone();
        journey::tactical::waiting_npc(checkpoint.state())
    };
    actor::record_audit_line(
        "gameplay.dev.npc_hold",
        format!("DEV-NPC hold npc={name:?} on={}", body.on),
    );
    if body.on {
        return Ok((
            StatusCode::OK,
            Json(json!({"status": "held", "npc": name, "edited": false})),
        ));
    }
    if waiting
        .as_ref()
        .is_some_and(|waiting| name == "all" || waiting.eq_ignore_ascii_case(&name))
    {
        let mut answer = commit(&state, &waiting.unwrap_or(name.clone()), NpcAct::Policy).await?;
        answer["hold"] = json!("released");
        return Ok((StatusCode::OK, Json(answer)));
    }
    Ok((
        StatusCode::OK,
        Json(json!({"status": "released", "npc": name, "waiting": false, "edited": false})),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActBody {
    npc: String,
    action: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    weapon: Option<String>,
    #[serde(default)]
    to: Option<String>,
}

/// `POST /dev/npc/act {"npc", "action", ...}`: the NPC takes its turn with this action instead of
/// the policy. Refused unless the battle is waiting on that NPC.
pub(super) async fn act(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<ActBody>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    super::require_gameplay_overrides()?;
    let act = NpcAct::parse(
        &body.action,
        body.target.as_deref(),
        body.weapon.as_deref(),
        body.to.as_deref(),
    )
    .map_err(bad_request)?;
    let name = display_name(&current_scene(&state)?, &npc_name(&body.npc)?);
    commit(&state, &name, act)
        .await
        .map(|answer| (StatusCode::OK, Json(answer)))
}

/// Sends one NPC turn to the owner, records the committed revision as an edit and returns the
/// answer: `acted` with the revision, or the owner's own refusal.
pub(super) async fn commit(
    state: &Shared,
    npc: &str,
    act: NpcAct,
) -> Result<Value, (StatusCode, Json<Value>)> {
    let (current, _) = super::jumps::current_revision(state).await?;
    let mut notices = super::jumps::take_stale_notices(state, current);
    let credential = {
        let seats = state.seats.lock().await;
        super::jumps::table_credential(&seats)?
    };
    let label = act.label();
    let job = super::commands::DevJob::NpcTurn {
        npc: npc.to_owned(),
        act: act.clone(),
    };
    let pre = super::jumps::pre_jump_moment(state);
    let (revision, before, after) = super::jumps::owner_jump(state, credential, job).await?;
    state.overrides.lock().await.push(json!({
        "kind": "npc-turn",
        "target": npc,
        "action": label,
        "before": before,
        "after": after,
        "revision": {"epoch": revision.0, "sequence": revision.1},
    }));
    if let Ok(mut guarded) = state.history.lock() {
        guarded.set_redo(pre, revision);
    }
    notices.push("run marked edited".to_owned());
    Ok(json!({
        "status": "acted",
        "npc": npc,
        "action": label,
        "revision": {"epoch": revision.0, "sequence": revision.1},
        "before": before,
        "after": after,
        "notices": notices,
        "edited": true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn act_arguments_parse_each_action_and_refuse_flags_that_do_not_apply() {
        assert_eq!(
            NpcAct::parse("attack", Some("Brynn"), Some("scimitar"), None),
            Ok(NpcAct::Attack {
                target: "Brynn".to_owned(),
                weapon: Some("scimitar".to_owned()),
            })
        );
        assert_eq!(
            NpcAct::parse("move", None, None, Some("5,2")),
            Ok(NpcAct::Move { cell: (5, 2) })
        );
        assert_eq!(NpcAct::parse("dash", None, None, None), Ok(NpcAct::Dash));
        assert_eq!(
            NpcAct::parse("end-turn", None, None, None),
            Ok(NpcAct::EndTurn)
        );
        assert!(NpcAct::parse("attack", None, None, None).is_err());
        assert!(NpcAct::parse("move", None, None, Some("five,two")).is_err());
        assert!(NpcAct::parse("move", None, None, None).is_err());
        assert!(NpcAct::parse("dodge", Some("Brynn"), None, None).is_err());
        assert!(NpcAct::parse("attack", Some("Brynn"), None, Some("1,1")).is_err());
        assert_eq!(
            NpcAct::parse("policy", None, None, None),
            Ok(NpcAct::Policy)
        );
    }

    #[test]
    fn every_policy_action_and_reaction_parses_with_legal_flags() {
        for action in [
            "dodge",
            "disengage",
            "decline-reaction",
            "nimble-disengage",
            "stand",
            "drop-prone",
            "doff-shield",
            "death-save",
            "policy",
        ] {
            assert!(NpcAct::parse(action, None, None, None).is_ok(), "{action}");
            assert!(
                NpcAct::parse(action, Some("Brynn"), None, None).is_err(),
                "{action}"
            );
        }
        assert_eq!(
            NpcAct::parse("opportunity", None, Some("unarmed-strike"), None).unwrap(),
            NpcAct::Opportunity {
                weapon: Some("unarmed-strike".to_owned())
            }
        );
        set_hold("one-dev-enemy", true);
        set_hold("all", true);
        assert!(held("a-future-enemy"));
        set_hold("all", false);
        assert!(!held("one-dev-enemy"));
        assert!(!held("a-future-enemy"));
    }
    #[test]
    fn say_collapses_whitespace_and_refuses_empty_or_overlong_lines() {
        assert_eq!(
            say_text("  Keep  the  lamp   lit. ").as_deref(),
            Ok("Keep the lamp lit.")
        );
        assert!(say_text("   ").is_err());
        assert!(say_text(&"x".repeat(LINE_CHARACTERS + 1)).is_err());
    }

    #[test]
    fn a_held_npc_turn_never_consults_the_policy_and_an_open_one_does() {
        let mut consulted = false;
        let held_turn: Result<Option<u8>, ()> = choose_turn(true, || {
            consulted = true;
            Ok(1)
        });
        assert_eq!(held_turn, Ok(None));
        assert!(!consulted, "the hold is read before the policy runs");

        let open_turn: Result<Option<u8>, ()> = choose_turn(false, || {
            consulted = true;
            Ok(2)
        });
        assert_eq!(open_turn, Ok(Some(2)));
        assert!(consulted);
    }

    #[test]
    fn holds_are_per_npc_and_case_insensitive() {
        let name = "dev-test-hold-npc";
        assert!(!held(name));
        set_hold(&name.to_ascii_uppercase(), true);
        assert!(held(name));
        set_hold(name, false);
        assert!(!held(name));
    }

    #[test]
    fn a_person_is_found_by_id_first_then_by_name() {
        let people = vec![
            ("merchant".to_owned(), "Marek".to_owned()),
            ("guard".to_owned(), "Jessa".to_owned()),
        ];
        assert_eq!(
            find_person(&people, "GUARD").map(|p| p.1.as_str()),
            Some("Jessa")
        );
        assert_eq!(
            find_person(&people, "Marek").map(|p| p.0.as_str()),
            Some("merchant")
        );
    }

    #[test]
    fn say_refuses_an_npc_who_is_not_on_stage_and_names_the_reason() {
        let people = vec![("merchant".to_owned(), "Marek".to_owned())];
        let (id, name) = speaker_on_stage(&people, "marek").expect("Marek is on stage");
        assert_eq!((id.as_str(), name.as_str()), ("merchant", "Marek"));
        assert_eq!(
            speaker_on_stage(&people, "Brynn"),
            Err("the NPC is not present in the current scene")
        );
    }
}
