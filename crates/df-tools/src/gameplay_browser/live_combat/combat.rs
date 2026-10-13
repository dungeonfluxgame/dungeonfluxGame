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
use wasm_bindgen::{JsCast, JsValue};
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

/// A turn is spent, and its buttons wait for the next view, only once the server has ACCEPTED
/// the action: after a rejection the same offer is still good and stays usable.
fn waiting_for_current_view(
    role: &str,
    last_accepted: bool,
    request: Option<&rpc::SubmitActionRequest>,
    offers: &[rpc::GameplayActionOffer],
) -> bool {
    role == "player"
        && last_accepted
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

fn rolls(combat: &rpc::CombatView, released_sequence: Option<u64>) -> Vec<RollText> {
    // The server lists outcomes oldest first; the newest is shown first and largest.
    combat
        .outcomes
        .iter()
        .filter(|outcome| released_sequence.is_none_or(|sequence| outcome.sequence <= sequence))
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
    let roll = rolls(&single, None).into_iter().next()?;
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
        state.last_accepted,
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
        rolls: rolls(combat, player.then_some(combat.released_sequence)),
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
    use crate::gameplay_browser::{action_button, action_identity, node};
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
        Some(existing) => existing,
        None => node(&document, root, "div", "tactical-controls", "")?,
    };
    let stale_children = controls.query_selector_all(":scope > :not(.offered-action)")?;
    for index in (0..stale_children.length()).rev() {
        if let Some(child) = stale_children.item(index) {
            child.unchecked_into::<Element>().remove();
        }
    }
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
        client.borrow().last_accepted,
        client.borrow().last_request.as_ref(),
        view.offers,
    ) {
        let actions = controls.query_selector_all(":scope > .offered-action")?;
        for index in (0..actions.length()).rev() {
            if let Some(action) = actions.item(index) {
                action.unchecked_into::<Element>().remove();
            }
        }
        node(
            &document,
            &controls,
            "p",
            "turn-waiting",
            "Waiting for the current fight view.",
        )?;
        return Ok(());
    }
    let mut active_action_ids = Vec::new();
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
        active_action_ids.push(action_identity(&offer));
        action_button(client, &controls, offer)?;
    }
    let offers = controls.query_selector_all(":scope > .offered-action")?;
    for index in (0..offers.length()).rev() {
        if let Some(element) = offers.item(index) {
            let element = element.unchecked_into::<Element>();
            if element
                .get_attribute("data-action-identity")
                .map_or(true, |identity| !active_action_ids.contains(&identity))
            {
                element.remove();
            }
        }
    }
    publish_move_map(client, &controls, combat)
}

/// Largest grid the phone draws as a map; a larger fight keeps the plain list.
const MOVE_MAP_MAX_COLUMNS: u32 = 16;
const MOVE_MAP_MAX_ROWS: u32 = 12;

/// What one square of the phone's map shows.
#[derive(Clone, Debug, Eq, PartialEq)]
enum MoveSquare {
    Empty,
    /// A cell the Move offer lists: its option id.
    Reachable(String),
    /// A combatant: the mark drawn, the side (1 heroes, 2 enemies), down, and the player's own.
    Occupied {
        mark: String,
        side: u32,
        down: bool,
        own: bool,
    },
}

/// The mark for a combatant on the map: the initial, and a trailing number when the name has one
/// ("Giant Weasel 2" is "G2").
fn move_map_mark(name: &str) -> String {
    let initial = name.chars().find(|character| character.is_alphanumeric());
    let number = name
        .rsplit(' ')
        .next()
        .filter(|last| !last.is_empty() && last.chars().all(|digit| digit.is_ascii_digit()));
    match (initial, number) {
        (Some(initial), Some(number)) => format!("{}{number}", initial.to_uppercase()),
        (Some(initial), None) => initial.to_uppercase().to_string(),
        (None, _) => "?".to_owned(),
    }
}

/// The map's squares, row by row, sized to hold every combatant and every offered cell. `None`
/// when there is nothing to offer or the fight is larger than the phone can draw.
fn move_map_squares(
    option_ids: &[String],
    participants: &[rpc::PartyMemberView],
    own_name: Option<&str>,
) -> Option<(u32, Vec<MoveSquare>)> {
    let offered = option_ids
        .iter()
        .filter_map(|id| {
            let (col, row) = id.split_once('-')?;
            Some(((col.parse::<u32>().ok()?, row.parse::<u32>().ok()?), id))
        })
        .collect::<Vec<_>>();
    if offered.is_empty() || offered.len() != option_ids.len() {
        return None;
    }
    let occupied = participants
        .iter()
        .filter_map(|member| {
            let cell = member.cell.as_ref()?;
            Some(((cell.col, cell.row), member))
        })
        .collect::<Vec<_>>();
    let cells = offered
        .iter()
        .map(|(cell, _)| *cell)
        .chain(occupied.iter().map(|(cell, _)| *cell));
    let columns = cells.clone().map(|(col, _)| col).max()? + 1;
    let rows = cells.map(|(_, row)| row).max()? + 1;
    if columns > MOVE_MAP_MAX_COLUMNS || rows > MOVE_MAP_MAX_ROWS {
        return None;
    }
    let mut squares = Vec::new();
    for row in 0..rows {
        for col in 0..columns {
            let standing = occupied
                .iter()
                .filter(|(cell, _)| *cell == (col, row))
                .map(|(_, member)| member)
                .min_by_key(|member| member.unconscious || member.hit_points == 0);
            squares.push(
                if let Some((_, id)) = offered.iter().find(|(cell, _)| *cell == (col, row)) {
                    MoveSquare::Reachable((*id).clone())
                } else if let Some(member) = standing {
                    MoveSquare::Occupied {
                        mark: move_map_mark(&member.name),
                        side: member.side,
                        down: member.unconscious || member.hit_points == 0,
                        own: own_name == Some(member.name.as_str()),
                    }
                } else {
                    MoveSquare::Empty
                },
            );
        }
    }
    Some((columns, squares))
}

/// The owner's chosen way to move (PHN-01): a small map of the fight above the Move button.
/// Lit squares are the cells the server offers; a tap picks that cell in the offer's own list,
/// which stays the input that is submitted, so the map adds no rule of its own.
fn publish_move_map(
    client: &Rc<RefCell<Client>>,
    controls: &Element,
    combat: &rpc::CombatView,
) -> Result<(), JsValue> {
    use crate::gameplay_browser::{button, node};
    let kind = rpc::GameplayActionKind::Move as i32;
    let Some(wrapper) = controls.query_selector(&format!(
        ":scope > .offered-action:has(> button[data-action-kind=\"{kind}\"])"
    ))?
    else {
        return Ok(());
    };
    for stale in [":scope > .move-map", ":scope > .move-chosen"] {
        if let Some(stale) = wrapper.query_selector(stale)? {
            stale.remove();
        }
    }
    let Some(select) = wrapper
        .query_selector(":scope > label[data-group-id=\"tactical-choice\"] > select")?
        .and_then(|select| select.dyn_into::<web_sys::HtmlSelectElement>().ok())
    else {
        return Ok(());
    };
    let options = select.query_selector_all(":scope > option")?;
    let option_ids = (0..options.length())
        .filter_map(|index| options.item(index))
        .filter_map(|option| option.unchecked_into::<Element>().get_attribute("value"))
        .collect::<Vec<_>>();
    let own_name = combat
        .participants
        .iter()
        .find(|member| member.participant_id == combat.active_participant_id)
        .map(|member| member.name.clone());
    let Some((columns, squares)) =
        move_map_squares(&option_ids, &combat.participants, own_name.as_deref())
    else {
        return Ok(());
    };
    let document = client.borrow().document.clone();
    let map = document.create_element("div")?;
    map.set_class_name("move-map");
    map.set_attribute("role", "group")?;
    map.set_attribute(
        "aria-label",
        "Map of the fight. Tap a lit square to move there.",
    )?;
    map.set_attribute("style", &format!("--columns:{columns}"))?;
    wrapper.insert_before(&map, wrapper.first_child().as_ref())?;
    let chosen = document.create_element("p")?;
    chosen.set_class_name("move-chosen");
    map.after_with_node_1(&chosen)?;
    let describe = |select: &web_sys::HtmlSelectElement| {
        let value = select.value();
        let label = select
            .query_selector_all(":scope > option")
            .ok()
            .and_then(|options| {
                (0..options.length())
                    .filter_map(|index| options.item(index))
                    .map(|option| option.unchecked_into::<Element>())
                    .find(|option| option.get_attribute("value").as_deref() == Some(&value))
            })
            .and_then(|option| option.text_content())
            .unwrap_or_default();
        format!("Tap a lit square, then Move. Chosen: {label}.")
    };
    chosen.set_text_content(Some(&describe(&select)));
    let selected = select.value();
    for square in squares {
        match square {
            MoveSquare::Empty => {
                node(&document, &map, "span", "move-cell", "")?;
            }
            MoveSquare::Occupied {
                mark,
                side,
                down,
                own,
            } => {
                let cell = node(&document, &map, "span", "move-cell", &mark)?;
                cell.set_attribute("data-side", &side.to_string())?;
                if down {
                    cell.set_attribute("data-down", "")?;
                }
                if own {
                    cell.set_attribute("data-own", "")?;
                }
            }
            MoveSquare::Reachable(id) => {
                let (picked, list, board, line) =
                    (id.clone(), select.clone(), map.clone(), chosen.clone());
                let cell = button(client, &map, "", move |_| {
                    list.set_value(&picked);
                    if let Ok(cells) = board.query_selector_all("button.move-cell") {
                        for index in 0..cells.length() {
                            if let Some(cell) = cells.item(index) {
                                let cell = cell.unchecked_into::<Element>();
                                let pressed =
                                    cell.get_attribute("data-cell").as_deref() == Some(&picked);
                                let _set = cell.set_attribute("aria-pressed", &pressed.to_string());
                            }
                        }
                    }
                    line.set_text_content(Some(&describe(&list)));
                })?;
                cell.set_class_name("move-cell");
                cell.set_attribute("type", "button")?;
                cell.set_attribute("data-cell", &id)?;
                cell.set_attribute("aria-pressed", &(id == selected).to_string())?;
                if let Some((col, row)) = id.split_once('-')
                    && let (Ok(col), Ok(row)) = (col.parse::<u32>(), row.parse::<u32>())
                {
                    cell.set_attribute(
                        "aria-label",
                        &format!("Column {}, row {}", col + 1, row + 1),
                    )?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod move_map_tests {
    use super::{MoveSquare, move_map_mark, move_map_squares};
    use df_protocol::common as rpc;

    fn member(name: &str, col: u32, row: u32, side: u32, hit_points: u32) -> rpc::PartyMemberView {
        rpc::PartyMemberView {
            name: name.to_owned(),
            hit_points,
            cell: Some(rpc::GridCell { col, row }),
            side,
            ..Default::default()
        }
    }

    #[test]
    fn marks_are_an_initial_and_the_trailing_number() {
        assert_eq!(move_map_mark("Brakka"), "B");
        assert_eq!(move_map_mark("Giant Weasel 2"), "G2");
        assert_eq!(move_map_mark(""), "?");
    }

    #[test]
    fn the_map_holds_every_combatant_and_every_offered_cell_row_by_row() {
        let ids = vec!["1-0".to_owned(), "2-1".to_owned()];
        let party = vec![
            member("Brakka", 0, 0, 1, 13),
            member("Giant Weasel 2", 2, 0, 2, 0),
        ];
        let (columns, squares) = move_map_squares(&ids, &party, Some("Brakka")).unwrap();
        assert_eq!(columns, 3);
        assert_eq!(
            squares,
            vec![
                MoveSquare::Occupied {
                    mark: "B".to_owned(),
                    side: 1,
                    down: false,
                    own: true
                },
                MoveSquare::Reachable("1-0".to_owned()),
                MoveSquare::Occupied {
                    mark: "G2".to_owned(),
                    side: 2,
                    down: true,
                    own: false
                },
                MoveSquare::Empty,
                MoveSquare::Empty,
                MoveSquare::Reachable("2-1".to_owned()),
            ]
        );
    }

    #[test]
    fn an_option_that_is_not_a_cell_or_a_fight_too_large_keeps_the_list() {
        let party = vec![member("Brakka", 0, 0, 1, 13)];
        assert_eq!(move_map_squares(&["stay".to_owned()], &party, None), None);
        assert_eq!(move_map_squares(&[], &party, None), None);
        assert_eq!(move_map_squares(&["16-0".to_owned()], &party, None), None);
    }
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

    #[test]
    fn roll_cards_follow_the_released_sequence() {
        let combat = rpc::CombatView {
            outcomes: (1..=3)
                .map(|sequence| rpc::CombatOutcome {
                    sequence,
                    actor_name: format!("Actor {sequence}"),
                    ..Default::default()
                })
                .collect(),
            latest_sequence: 3,
            ..Default::default()
        };

        // At a fight's start the TV has shown nothing yet: the phone shows nothing either.
        assert!(super::rolls(&combat, Some(0)).is_empty());
        let released = super::rolls(&combat, Some(1));
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].label, "Actor 1 attacks ");

        let fully_released = super::rolls(&combat, Some(3));
        assert_eq!(
            fully_released
                .iter()
                .map(|roll| roll.label.as_str())
                .collect::<Vec<_>>(),
            ["Actor 3 attacks ", "Actor 2 attacks ", "Actor 1 attacks "]
        );
    }
}
