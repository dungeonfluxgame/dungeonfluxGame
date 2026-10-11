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
    portrait: &'static str,
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
}
struct Text {
    keyart: Option<&'static str>,
    title: String,
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
        let portrait = match index.checked_sub(view.journey.party.len()) {
            Some(_) => "bandit",
            None if index == 0 => "hero-1",
            None => "hero-2",
        };
        let mut resources = Vec::new();
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
            status: if !member.character_ready {
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
                label: format!("{} attacks {}", outcome.actor_name, outcome.target_name),
                value: outcome.attack_die.to_string(),
                explanation: format!(
                    "d20 {} {:+} = {} vs AC {}",
                    outcome.attack_die,
                    outcome.attack_modifier,
                    outcome.attack_total,
                    outcome.target_armor_class
                ),
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
            }
        })
        .collect()
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
    let blocked = state.busy || (state.last_request.is_some() && !state.last_confirmed);
    let keyart = active
        .as_deref()
        .and_then(|key| actors.iter().find(|actor| actor.key == key))
        .map(|actor| actor.portrait);
    Ok(Text {
        keyart,
        title: view.scene.title.clone(),
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
            (true, _) | (_, None) => format!("Round {} · Encounter resolved", combat.round),
            (false, Some(_)) if own_turn => format!("Round {} · Your turn", combat.round),
            (false, Some(name)) => format!("Round {} · {name}'s turn", combat.round),
        },
        active,
        action_empty: match (player, active_name) {
            (false, _) => "Actions appear on each player's device.".to_owned(),
            (true, Some(name)) => format!("Waiting for {name}…"),
            (true, None) => "The encounter is resolved.".to_owned(),
        },
        busy: state.busy,
        actors,
        offers: offers(state, view, blocked)?,
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
            portrait: Some(actor.portrait),
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
        location: "The Lantern Wharf · Greyhaven harbor",
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
        publish_keyart(&document, surface.root(), text.keyart)?;
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
    publish_keyart(&document, surface.root(), text.keyart)?;
    host.insert_before(surface.root(), host.first_child().as_ref())?;
    client.borrow_mut().combat.mounted = Some(Mounted::Combat(Rc::new(surface)));
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
    let (savage, graze) = {
        let state = client.borrow();
        (
            state.combat.choice(SAVAGE_ATTACKER),
            state.combat.choice(GRAZE),
        )
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
