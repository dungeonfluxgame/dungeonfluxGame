//! Maps the server's combat projection onto the designed `CombatPhaseSurface` for both roles.
//!
//! Every number and outcome shown is a field of the received `CombatView`/`CombatOutcome`;
//! this module only formats them. Inputs go through the existing `prepare`/`send` path.

use super::{LiveView, Mounted, presentation_revision};
use crate::gameplay_browser::{Client, prepare, redraw_current, send, status};
use df_protocol::common as rpc;
use df_types::{ClientBindingId, RevisionLabel};
use df_ui::{
    CombatActor, CombatArt, CombatFeedback, CombatHitPoints, CombatIntent, CombatLimits,
    CombatOffer, CombatOfferKind, CombatPhaseSurface, CombatPhaseView, CombatResource, CombatRoll,
    CombatTone, CombatVerdict,
};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::JsValue;
use web_sys::Element;

const LIMITS: CombatLimits = CombatLimits {
    max_actors: 8,
    max_offers: 12,
    max_rolls: 8,
    max_resources_per_actor: 8,
    max_text_bytes: 2048,
    max_total_text_bytes: 32_768,
};
const SAVAGE_ATTACKER: &str = "savage-attacker";
const GRAZE: &str = "graze";
const UNSUPPORTED_INPUT: &str = "The server offered an input this screen cannot present.";

struct ActorText {
    key: String,
    name: String,
    badge: String,
    status: &'static str,
    hit_points: Option<CombatHitPoints>,
    portrait: String,
    resources: Vec<(&'static str, String)>,
}
struct OfferText {
    key: String,
    offer: RevisionLabel,
    option: Option<RevisionLabel>,
    kind: CombatOfferKind,
    label: String,
    explanation: String,
    enabled: bool,
}
struct RollText {
    key: String,
    label: String,
    value: String,
    explanation: String,
    source: String,
    verdict: (&'static str, CombatTone),
    /// The server's kept d20 face for the phone's small die; `None` marks no natural.
    die_face: Option<u8>,
    /// The resting-frame art behind the small die's number, once the dice manifest has
    /// loaded; the badge reads as before until then.
    die_rest: Option<String>,
}
struct Text {
    keyart: Option<String>,
    title: String,
    location: String,
    narration: String,
    connection: &'static str,
    notice: &'static str,
    turn_label: String,
    active: Option<String>,
    action_empty: String,
    busy: bool,
    actors: Vec<ActorText>,
    offers: Vec<OfferText>,
    rolls: Vec<RollText>,
}

fn invalid(reason: &str) -> JsValue {
    JsValue::from_str(reason)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn label(value: &str) -> Result<RevisionLabel, JsValue> {
    RevisionLabel::new(Some(value)).map_err(|_| invalid("offer identity invalid"))
}

fn death_save_summary(save: &rpc::DeathSaveState) -> String {
    let state = if save.dead {
        "Dead · "
    } else if save.stable {
        "Stable at 0 HP · "
    } else {
        ""
    };
    let success = if save.successes == 1 {
        "success"
    } else {
        "successes"
    };
    let failure = if save.failures == 1 {
        "failure"
    } else {
        "failures"
    };
    format!(
        "{state}{} {success} · {} {failure}",
        save.successes, save.failures,
    )
}

fn death_save_for<'a>(
    participant: &rpc::PartyMemberView,
    saves: &'a [rpc::DeathSaveState],
) -> Option<&'a rpc::DeathSaveState> {
    saves.iter().find(|save| {
        if participant.participant_id.is_empty() || save.participant_id.is_empty() {
            save.name == participant.name
        } else {
            save.participant_id == participant.participant_id
        }
    })
}

fn conditions_resource(member: &rpc::PartyMemberView) -> Option<String> {
    if member
        .conditions
        .iter()
        .any(|condition| condition == "dead")
    {
        Some("dead".to_owned())
    } else if member.conditions.is_empty() {
        None
    } else {
        Some(member.conditions.join(", "))
    }
}

fn tactical_option_label(label: &str, target: &rpc::LegalCombatTarget) -> String {
    let cover = if target.cover == "none" {
        "no cover".to_owned()
    } else {
        format!("{} cover", target.cover)
    };
    let mut label = format!("{label} · {cover} · {}", target.range_reason);
    if target.disadvantage {
        let reasons = target
            .reasons
            .iter()
            .filter_map(|reason| reason.strip_prefix("disadvantage: "))
            .collect::<Vec<_>>()
            .join(", ");
        if reasons.is_empty() {
            label.push_str(" · disadvantage");
        } else {
            label.push_str(&format!(" · disadvantage: {reasons}"));
        }
    }
    label
}

fn action_empty(
    player: bool,
    own_turn: bool,
    active_name: Option<&str>,
    waiting_for_view: bool,
    resolving: bool,
) -> String {
    match (player, active_name) {
        (true, _) if waiting_for_view => "Waiting for the current fight view.".to_owned(),
        (true, _) if resolving => "Watch the TV: the last action is playing out.".to_owned(),
        (false, _) => "Actions appear on each player's device.".to_owned(),
        (true, _) if own_turn => "Your turn.".to_owned(),
        (true, Some(name)) => format!("Waiting for {name}…"),
        (true, None) => "The encounter is resolved.".to_owned(),
    }
}

fn waiting_for_current_view(
    role: &str,
    last_confirmed: bool,
    request: Option<&rpc::SubmitActionRequest>,
    offers: &[rpc::GameplayActionOffer],
) -> bool {
    role == "player"
        && last_confirmed
        && request.is_some_and(|request| {
            request.action_kind != rpc::GameplayActionKind::SetTableSetting as i32
                && !request.offer_id.is_empty()
                && offers
                    .iter()
                    .any(|offer| offer.offer_id == request.offer_id)
        })
}

fn actors(view: &LiveView<'_>, combat: &rpc::CombatView) -> (Vec<ActorText>, Option<String>) {
    let own = view.journey.own_character.as_ref();
    let own_matches = own.map_or(0, |own| {
        combat
            .participants
            .iter()
            .filter(|member| member.name == own.name)
            .count()
    });
    let mut active = None;
    let mut result = Vec::new();
    for (index, member) in combat.participants.iter().enumerate() {
        let key = if member.participant_id.is_empty() {
            format!("participant-{index}")
        } else {
            hex(&member.participant_id)
        };
        if member.active_turn && !combat.finished {
            active = Some(key.clone());
        }
        // Participants are the party in join order followed by the opposing creature.
        let portrait = if !member.portrait.is_empty() {
            member
                .portrait
                .strip_prefix("portrait-")
                .unwrap_or(&member.portrait)
                .to_owned()
        } else {
            match index.checked_sub(view.journey.party.len()) {
                Some(_) => "bandit",
                None if index == 0 => "hero-1",
                None => "hero-2",
            }
            .to_owned()
        };
        let mut resources = Vec::new();
        if let Some(cell) = &member.cell {
            resources.push((
                "Cell",
                format!("Column {}, row {}", cell.col + 1, cell.row + 1),
            ));
        }
        if let Some(conditions) = conditions_resource(member) {
            resources.push(("Conditions", conditions));
        }
        if let Some(save) = death_save_for(member, &combat.death_saves) {
            resources.push(("Death saves", death_save_summary(save)));
        }
        if let Some(own) = own
            && own_matches == 1
            && own.name == member.name
        {
            resources.push(("Armor Class", own.armor_class.to_string()));
            if let Some(uses) = own
                .facts
                .iter()
                .find(|fact| fact.label == "Second Wind uses")
            {
                resources.push(("Second Wind", format!("{} remaining", uses.value)));
            }
        }
        result.push(ActorText {
            key,
            name: member.name.clone(),
            badge: member.name.chars().next().map_or_else(
                || "?".to_owned(),
                |initial| initial.to_uppercase().collect(),
            ),
            status: if member
                .conditions
                .iter()
                .any(|condition| condition == "dead")
            {
                "Dead"
            } else if member.hit_points == 0
                && member
                    .conditions
                    .iter()
                    .any(|condition| condition == "stable")
            {
                "Stable at 0 HP"
            } else if member.hit_points == 0 && member.unconscious {
                "Dying"
            } else if !member.character_ready {
                "Creating a hero"
            } else if member.unconscious {
                "Unconscious"
            } else if member.active_turn && !combat.finished {
                "Acting now"
            } else {
                "Ready"
            },
            hit_points: (member.maximum_hit_points > 0).then_some(CombatHitPoints {
                current: member.hit_points,
                maximum: member.maximum_hit_points,
            }),
            portrait,
            resources,
        });
    }
    (result, active)
}

fn rolls(combat: &rpc::CombatView) -> Vec<RollText> {
    // The server lists outcomes oldest first; the newest is shown first and largest.
    combat
        .outcomes
        .iter()
        .rev()
        .enumerate()
        .map(|(index, outcome)| {
            let damage = if outcome.damage == 0 {
                "No damage".to_owned()
            } else if outcome.damage_dice.is_empty() {
                format!("{} damage", outcome.damage)
            } else {
                let dice: Vec<String> = outcome.damage_dice.iter().map(u32::to_string).collect();
                format!("{} damage ({})", outcome.damage, dice.join(" + "))
            };
            let knockout = if outcome.knocked_out {
                " · Nonlethal knockout: unconscious at 1 HP"
            } else {
                ""
            };
            RollText {
                key: format!("roll-{index}"),
                // The card says what kind of attack it was: a shot or a melee strike, and with what.
                label: if outcome.weapon.is_empty() {
                    format!("{} attacks {}", outcome.actor_name, outcome.target_name)
                } else {
                    format!(
                        "{} {} {} \u{b7} {}",
                        outcome.actor_name,
                        if outcome.attack_style == 2 {
                            "shoots"
                        } else {
                            "strikes"
                        },
                        outcome.target_name,
                        df_render::battle::float_text::attack_name(&outcome.weapon)
                    )
                },
                value: outcome.attack_die.to_string(),
                explanation: if outcome.target_armor_class == 0 {
                    format!(
                        "d20 {} {:+} = {}",
                        outcome.attack_die, outcome.attack_modifier, outcome.attack_total
                    )
                } else {
                    format!(
                        "d20 {} {:+} = {} vs AC {}",
                        outcome.attack_die,
                        outcome.attack_modifier,
                        outcome.attack_total,
                        outcome.target_armor_class
                    )
                },
                source: format!(
                    "{damage} · {} has {} HP left{knockout}",
                    outcome.target_name, outcome.target_hit_points
                ),
                verdict: if outcome.hit && outcome.critical {
                    ("Critical hit", CombatTone::Success)
                } else if outcome.hit {
                    ("Hit", CombatTone::Success)
                } else if outcome.grazed {
                    ("Graze", CombatTone::Neutral)
                } else {
                    ("Miss", CombatTone::Failure)
                },
                die_face: u8::try_from(outcome.attack_die)
                    .ok()
                    .filter(|face| (1..=20).contains(face)),
                die_rest: crate::gameplay_browser::live_dice::rest_url_for(
                    outcome.attack_die,
                    &outcome.faces,
                    outcome.roll_mode,
                ),
            }
        })
        .collect()
}

/// How one roll reads on the display, for the final-blow overlay that shows a roll the
/// combat screen never got to draw.
pub(in crate::gameplay_browser) struct RollDescription {
    pub(in crate::gameplay_browser) label: String,
    pub(in crate::gameplay_browser) value: String,
    pub(in crate::gameplay_browser) explanation: String,
    pub(in crate::gameplay_browser) source: String,
    pub(in crate::gameplay_browser) verdict: &'static str,
    pub(in crate::gameplay_browser) tone: &'static str,
}

pub(in crate::gameplay_browser) fn describe(
    outcome: &rpc::CombatOutcome,
) -> Option<RollDescription> {
    let single = rpc::CombatView {
        outcomes: vec![outcome.clone()],
        ..Default::default()
    };
    let roll = rolls(&single).into_iter().next()?;
    Some(RollDescription {
        label: roll.label,
        value: roll.value,
        explanation: roll.explanation,
        source: roll.source,
        verdict: roll.verdict.0,
        tone: match roll.verdict.1 {
            CombatTone::Success => "success",
            CombatTone::Failure => "failure",
            CombatTone::Neutral => "neutral",
        },
    })
}

fn offered_input(group: &rpc::JourneyGroup) -> Option<&str> {
    let known = matches!(group.group_id.as_str(), SAVAGE_ATTACKER | GRAZE);
    let shape = group.options.len() == 2
        && group.options.iter().any(|option| option.option_id == "no")
        && group.options.iter().any(|option| option.option_id == "yes");
    (known && shape).then_some(group.group_id.as_str())
}

fn offers(state: &Client, view: &LiveView<'_>, blocked: bool) -> Result<Vec<OfferText>, JsValue> {
    let mut result = Vec::new();
    if view
        .journey
        .combat
        .as_ref()
        .is_some_and(|combat| !combat.layout_id.is_empty())
    {
        return Ok(result);
    }
    for (index, offer) in view.offers.iter().enumerate() {
        let kind = rpc::GameplayActionKind::try_from(offer.action_kind).ok();
        let (key, explanation) = match kind {
            Some(rpc::GameplayActionKind::GreatswordAttack) => (
                "strike".to_owned(),
                "The server rolls the attack and the damage.",
            ),
            Some(rpc::GameplayActionKind::SecondWind) => (
                "second-wind".to_owned(),
                "A bonus action. The server resolves the healing.",
            ),
            Some(rpc::GameplayActionKind::EndTurn) => (
                "end-turn".to_owned(),
                "Pass the turn to the next combatant.",
            ),
            _ => (
                format!("offer-{index}"),
                "Offered by the server for this moment.",
            ),
        };
        let supported = offer.destinations.is_empty()
            && offer.input_groups.len() <= 2
            && offer
                .input_groups
                .iter()
                .all(|group| offered_input(group).is_some());
        let identity = label(&offer.offer_id)?;
        result.push(OfferText {
            key,
            offer: identity.clone(),
            option: None,
            kind: CombatOfferKind::Action,
            label: offer.label.clone(),
            explanation: if supported {
                explanation
            } else {
                UNSUPPORTED_INPUT
            }
            .to_owned(),
            enabled: supported && !blocked,
        });
        for group in offer.input_groups.iter().filter(|_| supported) {
            let description = group
                .options
                .iter()
                .find(|option| option.option_id == "yes")
                .map(|option| option.description.trim())
                .filter(|description| !description.is_empty())
                .unwrap_or("Optional choice for this attack.");
            result.push(OfferText {
                key: format!("choice-{}", group.group_id),
                offer: identity.clone(),
                option: Some(label(&group.group_id)?),
                kind: CombatOfferKind::Choice {
                    selected: state.combat.choice(&group.group_id),
                },
                label: group.label.clone(),
                explanation: description.to_owned(),
                enabled: !blocked,
            });
        }
    }
    Ok(result)
}

fn build(state: &Client, view: &LiveView<'_>) -> Result<Text, JsValue> {
    let combat = view
        .journey
        .combat
        .as_ref()
        .ok_or_else(|| invalid("combat view absent"))?;
    let player = state.role == "player";
    let (actors, active) = actors(view, combat);
    let active_name = (!combat.finished && !combat.active_actor.is_empty())
        .then_some(combat.active_actor.as_str());
    let own_turn = view
        .journey
        .own_character
        .as_ref()
        .is_some_and(|own| Some(own.name.as_str()) == active_name);
    let waiting_for_view = waiting_for_current_view(
        state.role,
        state.last_confirmed,
        state.last_request.as_ref(),
        view.offers,
    );
    let blocked = state.busy || (state.last_request.is_some() && !state.last_confirmed);
    // One clock: while the display is still presenting resolved beats the server withholds
    // offers; the phone says so instead of inviting an action the TV has not reached.
    let resolving = combat.released_sequence < combat.latest_sequence;
    let keyart = active
        .as_deref()
        .and_then(|key| actors.iter().find(|actor| actor.key == key))
        .map(|actor| actor.portrait.clone());
    Ok(Text {
        keyart,
        title: view.scene.title.clone(),
        location: if view.scene.place.is_empty() {
            combat.location_id.replace('-', " ")
        } else {
            view.scene.place.clone()
        },
        narration: view.narration.to_owned(),
        connection: if state.connection.is_some() {
            "The server rolls and resolves every action"
        } else {
            "Reconnecting to the game"
        },
        notice: if player {
            "Your private view"
        } else {
            "Shared battle view"
        },
        turn_label: match (combat.finished, active_name) {
            _ if waiting_for_view => format!("Round {} · Waiting for current view…", combat.round),
            _ if resolving && !combat.finished => {
                format!("Round {} · On the TV now…", combat.round)
            }
            (true, _) | (_, None) => format!("Round {} · Encounter resolved", combat.round),
            (false, Some(_)) if own_turn => format!("Round {} · Your turn", combat.round),
            (false, Some(name)) => format!("Round {} · {name}'s turn", combat.round),
        },
        active,
        action_empty: action_empty(player, own_turn, active_name, waiting_for_view, resolving),
        busy: state.busy,
        actors,
        offers: offers(state, view, blocked || waiting_for_view)?,
        rolls: rolls(combat),
    })
}

fn with_view<T>(text: &Text, operation: impl FnOnce(&CombatPhaseView<'_>) -> T) -> T {
    let resources: Vec<Vec<CombatResource<'_>>> = text
        .actors
        .iter()
        .map(|actor| {
            actor
                .resources
                .iter()
                .map(|(label, value)| CombatResource { label, value })
                .collect()
        })
        .collect();
    let actors: Vec<CombatActor<'_>> = text
        .actors
        .iter()
        .zip(&resources)
        .map(|(actor, resources)| CombatActor {
            key: &actor.key,
            name: &actor.name,
            initiative: &actor.badge,
            status: actor.status,
            resources,
            hit_points: actor.hit_points,
            portrait: Some(&actor.portrait),
        })
        .collect();
    let offers: Vec<CombatOffer<'_>> = text
        .offers
        .iter()
        .map(|offer| CombatOffer {
            key: &offer.key,
            offer: &offer.offer,
            option: offer.option.as_ref(),
            kind: offer.kind,
            label: &offer.label,
            explanation: &offer.explanation,
            enabled: offer.enabled,
            pending: text.busy && !matches!(offer.kind, CombatOfferKind::Choice { .. }),
        })
        .collect();
    let rolls: Vec<CombatRoll<'_>> = text
        .rolls
        .iter()
        .map(|roll| CombatRoll {
            key: &roll.key,
            label: &roll.label,
            value: &roll.value,
            explanation: &roll.explanation,
            source: &roll.source,
            die_face: roll.die_face,
            die_rest: roll.die_rest.as_deref(),
            verdict: Some(CombatVerdict {
                label: roll.verdict.0,
                tone: roll.verdict.1,
            }),
        })
        .collect();
    operation(&CombatPhaseView {
        art: CombatArt::Harbor,
        chapter: "Combat",
        title: &text.title,
        location: &text.location,
        narration: &text.narration,
        connection: text.connection,
        notice: text.notice,
        turn_label: &text.turn_label,
        active_actor: text.active.as_deref(),
        actors: &actors,
        action_heading: "Your actions",
        action_empty: &text.action_empty,
        offers: &offers,
        roll_heading: "Latest roll",
        roll_empty: "Waiting for the first strike…",
        rolls: &rolls,
        reaction: None,
        draft: None,
        feedback: if text.busy {
            CombatFeedback::Pending("Waiting for the game server…")
        } else {
            CombatFeedback::None
        },
    })
}

fn binding(state: &Client) -> Result<ClientBindingId, JsValue> {
    state
        .binding
        .as_ref()
        .and_then(|binding| binding.value.as_deref())
        .and_then(|bytes| ClientBindingId::from_bytes(bytes).ok())
        .ok_or_else(|| invalid("combat binding absent"))
}

fn render_error(client: &Rc<RefCell<Client>>, reason: &str) -> JsValue {
    let _ = client
        .borrow()
        .root
        .set_attribute("data-render-error", reason);
    invalid("Combat controls could not be displayed")
}

/// Creates the combat surface once, then updates it in place for every accepted view.
pub(super) fn present(
    client: &Rc<RefCell<Client>>,
    host: &Element,
    view: &LiveView<'_>,
) -> Result<(), JsValue> {
    let (document, role, binding, revision, text) = {
        let state = client.borrow();
        let revision = state
            .views
            .as_ref()
            .and_then(|views| views.current_revision())
            .ok_or_else(|| invalid("combat revision absent"))?;
        (
            state.document.clone(),
            state.role,
            binding(&state)?,
            revision,
            build(&state, view)?,
        )
    };
    let presentation = presentation_revision(client, revision)?;
    let mounted = match &client.borrow().combat.mounted {
        Some(Mounted::Combat(surface)) => Some(Rc::clone(surface)),
        _ => None,
    };
    if let Some(surface) = mounted {
        with_view(&text, |view| surface.update(binding, presentation, view))
            .map_err(|_| render_error(client, "combat-update"))?;
        publish_keyart(&document, surface.root(), text.keyart.as_deref())?;
        publish_initiative(&document, surface.root(), role, view)?;
        publish_tactical_controls(client, surface.root(), view)?;
        return Ok(());
    }
    let surface = with_view(&text, |view| {
        CombatPhaseSurface::create(
            &document,
            &format!("live-combat-{role}"),
            binding,
            presentation,
            view,
            LIMITS,
        )
    })
    .map_err(|_| render_error(client, "combat-create"))?;
    let weak = Rc::downgrade(client);
    surface
        .on_intent(move |intent| {
            if let Some(client) = weak.upgrade() {
                handle_intent(&client, intent);
            }
        })
        .map_err(|_| render_error(client, "combat-intent"))?;
    publish_keyart(&document, surface.root(), text.keyart.as_deref())?;
    publish_initiative(&document, surface.root(), role, view)?;
    publish_tactical_controls(client, surface.root(), view)?;
    host.insert_before(surface.root(), host.first_child().as_ref())?;
    client.borrow_mut().combat.mounted = Some(Mounted::Combat(Rc::new(surface)));
    Ok(())
}

/// The mounted fight uses the same bounded server-offer controls as the other phone phases.
fn publish_tactical_controls(
    client: &Rc<RefCell<Client>>,
    root: &Element,
    view: &LiveView<'_>,
) -> Result<(), JsValue> {
    use crate::gameplay_browser::{action_button, node};
    let existing = root.query_selector(":scope > .tactical-controls")?;
    let Some(combat) = view
        .journey
        .combat
        .as_ref()
        .filter(|combat| !combat.layout_id.is_empty() && client.borrow().role == "player")
    else {
        if let Some(existing) = existing {
            existing.remove();
        }
        return Ok(());
    };
    let document = client.borrow().document.clone();
    let controls = match existing {
        Some(existing) => {
            existing.set_text_content(None);
            existing
        }
        None => node(&document, root, "div", "tactical-controls", "")?,
    };
    if let Some(budget) = &combat.own_budget {
        let strip = node(
            &document,
            &controls,
            "div",
            "turn-budget resource-strip",
            "",
        )?;
        for value in [
            format!("Movement {} ft", budget.movement_ft),
            format!("Action {}", if budget.action { "ready" } else { "spent" }),
            format!(
                "Bonus {}",
                if budget.bonus_action {
                    "ready"
                } else {
                    "spent"
                }
            ),
            format!(
                "Reaction {}",
                if budget.reaction { "ready" } else { "spent" }
            ),
        ] {
            node(&document, &strip, "span", "resource-chip", &value)?;
        }
    }
    if let Some(ammunition) = &combat.own_ammunition {
        let strip = node(&document, &controls, "div", "ammunition resource-strip", "")?;
        for value in [
            format!("Arrows {}", ammunition.arrows),
            format!("Bolts {}", ammunition.bolts),
        ] {
            node(&document, &strip, "span", "resource-chip", &value)?;
        }
    }
    if waiting_for_current_view(
        client.borrow().role,
        client.borrow().last_confirmed,
        client.borrow().last_request.as_ref(),
        view.offers,
    ) {
        node(
            &document,
            &controls,
            "p",
            "turn-waiting",
            "Waiting for the current fight view.",
        )?;
        return Ok(());
    }
    for offer in view.offers {
        let mut offer = offer.clone();
        for group in &mut offer.input_groups {
            for option in &mut group.options {
                if let Some(target) = combat
                    .legal_targets
                    .iter()
                    .find(|target| target.option_id == option.option_id)
                {
                    option.label = tactical_option_label(&option.label, target);
                }
            }
        }
        action_button(client, &controls, offer)?;
    }
    Ok(())
}

/// The player's own initiative roll, large at the top of the phone's combat screen while the
/// fight opens ("Initiative 14 + 1 = 15 \u{2014} you act 2nd"). It is an extra child of the
/// surface root, which the surface never touches; it fades itself out and is removed as soon as
/// anyone has acted. Nothing is shown when the view carries no recorded roll for the player.
fn publish_initiative(
    document: &web_sys::Document,
    root: &Element,
    role: &str,
    view: &LiveView<'_>,
) -> Result<(), JsValue> {
    if role != "player" {
        return Ok(());
    }
    let existing = root.query_selector(":scope > .live-initiative")?;
    let line = view.journey.combat.as_ref().and_then(|combat| {
        let own = view.journey.own_character.as_ref()?;
        let opening = combat.outcomes.is_empty() && combat.round <= 1 && !combat.finished;
        let roll = combat
            .initiative
            .iter()
            .find(|roll| roll.name == own.name)
            .filter(|_| opening)?;
        Some(df_render::battle::initiative_line(
            roll.die,
            roll.modifier,
            roll.total,
            roll.order,
        ))
    });
    match (line, existing) {
        (Some(line), None) => {
            let banner = document.create_element("div")?;
            banner.set_class_name("live-initiative");
            banner.set_attribute("role", "status")?;
            banner.set_text_content(Some(&line));
            root.append_child(&banner)?;
        }
        (None, Some(banner)) => banner.remove(),
        _ => {}
    }
    Ok(())
}

/// The active combatant's large portrait, shown by the shared display's layout. It is an extra
/// child of the surface root, which the surface never touches.
fn publish_keyart(
    document: &web_sys::Document,
    root: &Element,
    portrait: Option<&str>,
) -> Result<(), JsValue> {
    let art = match root.query_selector(":scope > .live-keyart")? {
        Some(art) => art,
        None => {
            let art = document.create_element("div")?;
            art.set_class_name("live-keyart");
            art.set_attribute("aria-hidden", "true")?;
            root.append_child(&art)?;
            art
        }
    };
    match portrait {
        Some(portrait) => {
            art.set_attribute("data-portrait", portrait)?;
            art.remove_attribute("hidden")
        }
        None => art.set_attribute("hidden", ""),
    }
}

/// A choice flips only this screen's own draft; an action submits through the existing path.
fn handle_intent(client: &Rc<RefCell<Client>>, intent: CombatIntent) {
    if let CombatOfferKind::Choice { .. } = intent.kind {
        if let Some(group) = &intent.option {
            client.borrow_mut().combat.toggle_choice(group.as_str());
        }
        if redraw_current(client).is_err() {
            status(&client.borrow(), "Saved view could not be displayed.");
        }
        return;
    }
    let offer = {
        let state = client.borrow();
        state
            .views
            .as_ref()
            .and_then(|views| views.current())
            .and_then(|view| match &view.audience {
                Some(rpc::view_message::Audience::Player(player)) => Some(&player.offers),
                _ => None,
            })
            .and_then(|offers| {
                offers
                    .iter()
                    .find(|offer| offer.offer_id == intent.offer.as_str())
                    .cloned()
            })
    };
    let Some(offer) = offer else {
        crate::gameplay_browser::feedback(
            &client.borrow(),
            "That offer has ended. Choose from the current view.",
        );
        return;
    };
    // A switch travels only with an offer that asks for it: the server refuses it on any other.
    let (savage, graze) = {
        let state = client.borrow();
        let asked = |group: &str| {
            offer
                .input_groups
                .iter()
                .any(|input| input.group_id == group)
                && state.combat.choice(group)
        };
        (asked(SAVAGE_ATTACKER), asked(GRAZE))
    };
    match prepare(
        client,
        &offer,
        None,
        savage,
        graze,
        rpc::HarborDestination::Unspecified as i32,
    ) {
        Ok(body) => send(client.clone(), body, false),
        Err(error) => crate::gameplay_browser::feedback(&client.borrow(), &error),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        action_empty, conditions_resource, death_save_for, death_save_summary,
        tactical_option_label, waiting_for_current_view,
    };
    use df_protocol::common as rpc;

    #[test]
    fn accepted_offer_remains_blocked_until_a_new_view_replaces_it() {
        let request = rpc::SubmitActionRequest {
            action_kind: rpc::GameplayActionKind::EndTurn as i32,
            offer_id: "old-turn".to_owned(),
            ..Default::default()
        };
        let stale_view = [rpc::GameplayActionOffer {
            offer_id: "old-turn".to_owned(),
            ..Default::default()
        }];
        let current_view = [rpc::GameplayActionOffer {
            offer_id: "next-turn".to_owned(),
            ..Default::default()
        }];

        assert!(waiting_for_current_view(
            "player",
            true,
            Some(&request),
            &stale_view
        ));
        assert!(!waiting_for_current_view(
            "player",
            true,
            Some(&request),
            &current_view
        ));
    }

    #[test]
    fn table_setting_receipt_does_not_block_combat_offers() {
        let request = rpc::SubmitActionRequest {
            action_kind: rpc::GameplayActionKind::SetTableSetting as i32,
            offer_id: "table-setting".to_owned(),
            ..Default::default()
        };
        let stale_view = [rpc::GameplayActionOffer {
            offer_id: "table-setting".to_owned(),
            ..Default::default()
        }];

        assert!(!waiting_for_current_view(
            "player",
            true,
            Some(&request),
            &stale_view
        ));
    }

    #[test]
    fn death_save_summary_shows_progress_and_terminal_state() {
        assert_eq!(
            death_save_summary(&rpc::DeathSaveState {
                successes: 1,
                failures: 2,
                ..Default::default()
            }),
            "1 success · 2 failures"
        );
        assert_eq!(
            death_save_summary(&rpc::DeathSaveState {
                failures: 2,
                stable: true,
                ..Default::default()
            }),
            "Stable at 0 HP · 0 successes · 2 failures"
        );
        assert_eq!(
            death_save_summary(&rpc::DeathSaveState {
                dead: true,
                ..Default::default()
            }),
            "Dead · 0 successes · 0 failures"
        );
    }

    #[test]
    fn death_save_is_matched_by_participant_identity() {
        let member = rpc::PartyMemberView {
            participant_id: vec![1, 2],
            name: "Hero".to_owned(),
            ..Default::default()
        };
        let saves = [
            rpc::DeathSaveState {
                participant_id: vec![9],
                name: "Hero".to_owned(),
                failures: 2,
                ..Default::default()
            },
            rpc::DeathSaveState {
                participant_id: vec![1, 2],
                name: "Other label".to_owned(),
                failures: 1,
                ..Default::default()
            },
        ];

        assert_eq!(
            death_save_for(&member, &saves).map(|save| save.failures),
            Some(1)
        );
    }

    #[test]
    fn dead_participant_conditions_show_only_dead() {
        let dead = rpc::PartyMemberView {
            conditions: vec![
                "prone".to_owned(),
                "unconscious".to_owned(),
                "dead".to_owned(),
            ],
            ..Default::default()
        };
        let living = rpc::PartyMemberView {
            conditions: vec!["prone".to_owned()],
            ..Default::default()
        };

        assert_eq!(conditions_resource(&dead).as_deref(), Some("dead"));
        assert_eq!(conditions_resource(&living).as_deref(), Some("prone"));
    }

    #[test]
    fn own_turn_empty_actions_does_not_wait_for_own_name() {
        let message = action_empty(true, true, Some("Brakka"), false, false);

        assert_eq!(message, "Your turn.");
        assert!(!message.contains("Waiting for Brakka"));
    }

    #[test]
    fn other_hero_turn_empty_actions_names_active_hero() {
        assert_eq!(
            action_empty(true, false, Some("Sigrun"), false, false),
            "Waiting for Sigrun…"
        );
    }

    #[test]
    fn tactical_option_label_explains_cover_and_disadvantage() {
        let target = rpc::LegalCombatTarget {
            cover: "none".to_owned(),
            range_reason: "normal range: 30 ft".to_owned(),
            disadvantage: true,
            reasons: vec!["disadvantage: hostile within 5 ft".to_owned()],
            ..Default::default()
        };

        assert_eq!(
            tactical_option_label("Longbow at Weasel", &target),
            "Longbow at Weasel · no cover · normal range: 30 ft · disadvantage: hostile within 5 ft"
        );
        let covered = rpc::LegalCombatTarget {
            cover: "half (+2 AC)".to_owned(),
            range_reason: "normal range: 10 ft".to_owned(),
            disadvantage: true,
            ..Default::default()
        };
        assert_eq!(
            tactical_option_label("Longbow at Weasel", &covered),
            "Longbow at Weasel · half (+2 AC) cover · normal range: 10 ft · disadvantage"
        );
    }
}
