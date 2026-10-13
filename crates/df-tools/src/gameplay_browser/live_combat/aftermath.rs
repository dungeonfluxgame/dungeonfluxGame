//! Encounter aftermath and short rest. The shared display shows the resolution in
//! `EncounterAftermathSurface`; each player gets the Short Rest / Continue choices in
//! `CampfireSurface`. Offers, labels and destinations are exactly what the server offered.

use super::{LiveView, Mounted, presentation_revision};
use crate::gameplay_browser::{Client, feedback, prepare, send};
use df_protocol::common as rpc;
use df_ui::{
    AftermathPartyMember, CampfireAction, CampfireMember, CampfireOffer, CampfireSelection,
    CampfireSurface, CampfireView, CombatHitPoints, EncounterAftermathSurface,
    EncounterAftermathView,
};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::JsValue;
use web_sys::Element;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn member_key(index: usize, member: &rpc::PartyMemberView) -> String {
    if member.participant_id.is_empty() {
        format!("member-{index}")
    } else {
        hex(&member.participant_id)
    }
}

fn condition(member: &rpc::PartyMemberView) -> &'static str {
    if member
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
    } else if member
        .conditions
        .iter()
        .any(|condition| condition == "bloodied")
    {
        "Bloodied"
    } else if member
        .conditions
        .iter()
        .any(|condition| condition == "hurt")
    {
        "Hurt"
    } else {
        ""
    }
}

/// The continue choice names its one destination, unless its own wording already does
/// ("Follow Rowan to the Vestry" needs no "· the Vestry" after it).
fn continue_label(label: &str, destination: &str) -> String {
    let named = destination
        .strip_prefix("the ")
        .or_else(|| destination.strip_prefix("The "))
        .unwrap_or(destination);
    if named.is_empty() || label.to_lowercase().contains(&named.to_lowercase()) {
        label.to_owned()
    } else {
        format!("{label} · {destination}")
    }
}

fn dialogue_line(journey: &rpc::JourneyView) -> Option<String> {
    (!journey.dialogue.is_empty()).then(|| format!("{}: “{}”", journey.speaker, journey.dialogue))
}

/// "Victory" when the server projects the courier's closing line, which only the won encounter
/// has; otherwise "Defeat".
fn outcome_word(view: &LiveView<'_>) -> &'static str {
    if crate::presentation::is_victory(view.journey) {
        "Victory"
    } else {
        "Defeat"
    }
}

fn error(client: &Rc<RefCell<Client>>, reason: &str) -> JsValue {
    let _ = client
        .borrow()
        .root
        .set_attribute("data-render-error", reason);
    JsValue::from_str("Aftermath controls could not be displayed")
}

/// Shared display: the resolution and the party. It offers no input.
pub(super) fn present_display(
    client: &Rc<RefCell<Client>>,
    host: &Element,
    view: &LiveView<'_>,
) -> Result<(), JsValue> {
    let (document, revision) = {
        let state = client.borrow();
        let revision = state
            .views
            .as_ref()
            .and_then(|views| views.current_revision())
            .ok_or_else(|| JsValue::from_str("aftermath revision absent"))?;
        (state.document.clone(), revision)
    };
    let presentation = presentation_revision(client, revision)?;
    let result_line = dialogue_line(view.journey);
    let keys: Vec<String> = view
        .journey
        .party
        .iter()
        .enumerate()
        .map(|(index, member)| member_key(index, member))
        .collect();
    let party: Vec<AftermathPartyMember<'_>> = view
        .journey
        .party
        .iter()
        .zip(&keys)
        .enumerate()
        .map(|(index, (member, key))| AftermathPartyMember {
            key,
            name: &member.name,
            summary: condition(member),
            hit_points: (member.maximum_hit_points > 0).then_some(CombatHitPoints {
                current: member.hit_points,
                maximum: member.maximum_hit_points,
            }),
            portrait: Some(
                member
                    .portrait
                    .strip_prefix("portrait-")
                    .filter(|_| !member.portrait.is_empty())
                    .unwrap_or(if index == 0 { "hero-1" } else { "hero-2" }),
            ),
        })
        .collect();
    host.set_attribute("data-outcome", &outcome_word(view).to_lowercase())?;
    host.set_attribute(
        "data-narrative",
        crate::presentation::narrative_tier(view.narration),
    )?;

    let aftermath = EncounterAftermathView {
        generation: revision.epoch().get(),
        revision: presentation.sequence(),
        chapter: outcome_word(view),
        title: &view.scene.title,
        narrative: view.narration,
        dialogue: result_line.as_deref(),
        party_heading: "The party",
        party: &party,
        next_heading: "What happens next",
        next_context: "Each player chooses for their hero on their own device.",
        next_empty: "The party decides: rest or move on.",
        next_scene: None,
        feedback: None,
    };
    let mounted = match &client.borrow().combat.mounted {
        Some(Mounted::Aftermath(surface)) => Some(Rc::clone(surface)),
        _ => None,
    };
    if let Some(surface) = mounted {
        if surface.update(&aftermath).is_ok() {
            return Ok(());
        }
        // A replaced owner or refused view is rebuilt from the accepted snapshot.
        client.borrow_mut().combat.mounted = None;
        surface
            .dispose()
            .map_err(|_| error(client, "aftermath-cleanup"))?;
    }
    let surface = EncounterAftermathSurface::create(&document, &aftermath, |_| {})
        .map_err(|_| error(client, "aftermath-create"))?;
    host.insert_before(surface.root(), host.first_child().as_ref())?;
    client.borrow_mut().combat.mounted = Some(Mounted::Aftermath(Rc::new(surface)));
    Ok(())
}

fn offer_of(
    offers: &[rpc::GameplayActionOffer],
    kind: rpc::GameplayActionKind,
) -> Option<&rpc::GameplayActionOffer> {
    offers.iter().find(|offer| offer.action_kind == kind as i32)
}

/// A player's rest and continue choices, enabled only while a request can be made.
pub(super) fn present_rest(
    client: &Rc<RefCell<Client>>,
    host: &Element,
    view: &LiveView<'_>,
) -> Result<(), JsValue> {
    let (document, revision, connected, blocked) = {
        let state = client.borrow();
        let revision = state
            .views
            .as_ref()
            .and_then(|views| views.current_revision())
            .ok_or_else(|| JsValue::from_str("rest revision absent"))?;
        (
            state.document.clone(),
            revision,
            state.connection.is_some(),
            state.busy || (state.last_request.is_some() && !state.last_confirmed),
        )
    };
    let busy = client.borrow().busy;
    let keys: Vec<String> = view
        .journey
        .party
        .iter()
        .enumerate()
        .map(|(index, member)| member_key(index, member))
        .collect();
    let conditions: Vec<&str> = view.journey.party.iter().map(condition).collect();
    let details: Vec<String> = view
        .journey
        .party
        .iter()
        .map(|member| format!("HP {} / {}", member.hit_points, member.maximum_hit_points))
        .collect();
    let members: Vec<CampfireMember<'_>> = view
        .journey
        .party
        .iter()
        .enumerate()
        .map(|(index, member)| CampfireMember {
            key: &keys[index],
            name: &member.name,
            condition: conditions[index],
            detail: &details[index],
        })
        .collect();
    let rest = offer_of(view.offers, rpc::GameplayActionKind::ShortRest);
    let proceed = offer_of(view.offers, rpc::GameplayActionKind::ChooseHarborScene);
    // The offered destination is part of the label so the choice is never ambiguous.
    let proceed_label = proceed.map(|offer| match offer.destinations.as_slice() {
        [only] => continue_label(&offer.label, &only.label),
        _ => offer.label.clone(),
    });
    let single_destination = proceed.is_some_and(|offer| offer.destinations.len() == 1);
    // The view carries no location name, so the eyebrow is only the outcome.
    let location = outcome_word(view);
    let result_line = dialogue_line(view.journey);
    let campfire = CampfireView {
        owner_generation: revision.epoch().get(),
        revision: revision.sequence(),
        title: &view.scene.title,
        location,
        narration: view.narration,
        party_label: "Your party",
        notice: match (&result_line, rest.is_some() || proceed.is_some()) {
            (Some(line), _) => line,
            (None, true) => "Choose how the party spends its time. The server records the result.",
            (None, false) => "Waiting for the next choice.",
        },
        connected,
        members: &members,
        rest: rest.map(|offer| CampfireOffer {
            id: &offer.offer_id,
            label: &offer.label,
            enabled: !blocked,
            pending: busy,
        }),
        continue_action: proceed.zip(proceed_label.as_deref()).map(|(offer, label)| {
            CampfireOffer {
                id: &offer.offer_id,
                label,
                enabled: !blocked && single_destination,
                pending: busy,
            }
        }),
    };
    if let Some(existing) = host.query_selector(":scope > .aftermath-tactical-controls")? {
        existing.remove();
    }
    let extra =
        crate::gameplay_browser::node(&document, host, "div", "aftermath-tactical-controls", "")?;
    for offer in view
        .offers
        .iter()
        .filter(|offer| offer.action_kind == rpc::GameplayActionKind::EndTurn as i32)
    {
        crate::gameplay_browser::action_button(client, &extra, offer.clone())?;
    }
    let mounted = match &client.borrow().combat.mounted {
        Some(Mounted::Rest(surface)) => Some(Rc::clone(surface)),
        _ => None,
    };
    if let Some(surface) = mounted {
        if surface.update(&campfire).is_ok() {
            return Ok(());
        }
        // The surface disposes itself when its owner changes; rebuild from the accepted view.
        client.borrow_mut().combat.mounted = None;
    }
    let weak = Rc::downgrade(client);
    let surface = CampfireSurface::create(&document, &campfire, move |selection| {
        if let Some(client) = weak.upgrade() {
            submit(&client, &selection);
        }
    })
    .map_err(|_| error(client, "rest-create"))?;
    host.insert_before(surface.root(), host.first_child().as_ref())?;
    client.borrow_mut().combat.mounted = Some(Mounted::Rest(Rc::new(surface)));
    Ok(())
}

fn submit(client: &Rc<RefCell<Client>>, selection: &CampfireSelection) {
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
                    .find(|offer| offer.offer_id == selection.offer_id)
                    .cloned()
            })
    };
    let Some(offer) = offer else {
        feedback(
            &client.borrow(),
            "That offer has ended. Choose from the current view.",
        );
        return;
    };
    let destination = match selection.action {
        CampfireAction::Rest => rpc::HarborDestination::Unspecified as i32,
        CampfireAction::Continue => match offer.destinations.as_slice() {
            [only] => only.destination,
            _ => {
                feedback(&client.borrow(), "Choose an offered destination.");
                return;
            }
        },
    };
    match prepare(client, &offer, None, false, false, destination) {
        Ok(body) => send(client.clone(), body, false),
        Err(reason) => feedback(&client.borrow(), &reason),
    }
}

#[cfg(test)]
mod tests {
    use super::{condition, continue_label, dialogue_line};
    use df_protocol::common as rpc;

    #[test]
    fn victory_display_projection_omits_healthy_state_and_includes_patron_dialogue() {
        let healthy = rpc::PartyMemberView {
            character_ready: true,
            hit_points: 8,
            maximum_hit_points: 8,
            ..Default::default()
        };
        let bloodied = rpc::PartyMemberView {
            character_ready: true,
            conditions: vec!["bloodied".to_owned()],
            ..Default::default()
        };
        let hurt = rpc::PartyMemberView {
            character_ready: true,
            conditions: vec!["hurt".to_owned()],
            ..Default::default()
        };
        let stable = rpc::PartyMemberView {
            hit_points: 0,
            conditions: vec!["stable".to_owned()],
            ..Default::default()
        };
        let dying = rpc::PartyMemberView {
            hit_points: 0,
            unconscious: true,
            ..Default::default()
        };
        let dead = rpc::PartyMemberView {
            conditions: vec!["dead".to_owned()],
            ..Default::default()
        };
        let journey = rpc::JourneyView {
            speaker: "Perrin".to_owned(),
            dialogue: "Thank you for bringing the ledger home.".to_owned(),
            ..Default::default()
        };

        assert_eq!(condition(&healthy), "");
        assert_eq!(condition(&bloodied), "Bloodied");
        assert_eq!(condition(&hurt), "Hurt");
        assert_eq!(condition(&stable), "Stable at 0 HP");
        assert_eq!(condition(&dying), "Dying");
        assert_eq!(condition(&dead), "Dead");
        assert_eq!(
            dialogue_line(&journey).as_deref(),
            Some("Perrin: “Thank you for bringing the ledger home.”")
        );
        assert_eq!(dialogue_line(&rpc::JourneyView::default()), None);
    }

    #[test]
    fn the_continue_choice_does_not_name_its_destination_twice() {
        assert_eq!(
            continue_label("Follow Rowan to the Vestry", "the Vestry"),
            "Follow Rowan to the Vestry"
        );
        assert_eq!(
            continue_label("Move on", "the Vestry"),
            "Move on · the Vestry"
        );
        assert_eq!(continue_label("Move on", ""), "Move on");
    }
}
