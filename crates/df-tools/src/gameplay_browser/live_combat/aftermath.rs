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

fn short_rest_benefit() -> &'static str {
    "Regain one use of Second Wind"
}

fn own_hit_point_die_roll(journey: &rpc::JourneyView) -> Option<&str> {
    journey
        .own_character
        .as_ref()?
        .facts
        .iter()
        .find(|fact| fact.label == "Hit Point Die roll")
        .map(|fact| fact.value.as_str())
        .filter(|value| !value.trim().is_empty())
}

fn short_rest_button_label(label: &str) -> String {
    format!("{label}\n{}", short_rest_benefit())
}

fn roll_notice(journey: &rpc::JourneyView) -> Option<String> {
    if let Some(roll) = own_hit_point_die_roll(journey) {
        return Some(format!("Hit Point Die: {roll}"));
    }
    let combat = journey.combat.as_ref()?;
    let mut outcomes = combat
        .death_save_outcomes
        .iter()
        .filter(|outcome| outcome.sequence <= combat.released_sequence)
        .collect::<Vec<_>>();
    outcomes.sort_by_key(|outcome| outcome.sequence);
    let lines = outcomes.into_iter()
        .map(|outcome| {
            let (result, explanation, _) = super::combat::death_save_result(outcome);
            format!("{} rolls a Death Saving Throw: d20 {} · {result} — {explanation} · {} successes, {} failures", outcome.actor_name, outcome.attack_die, outcome.death_successes, outcome.death_failures)
        }).collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn hit_point_detail(member: &rpc::PartyMemberView, combat: Option<&rpc::CombatView>) -> String {
    let mut detail = format!("HP {} / {}", member.hit_points, member.maximum_hit_points);
    if let Some(save) =
        combat.and_then(|combat| super::combat::death_save_for(member, &combat.death_saves))
    {
        detail.push_str(" · ");
        detail.push_str(&super::combat::death_save_summary(save));
    }
    detail
}

fn preserve_rest_label_layout(surface: &CampfireSurface) -> Result<(), JsValue> {
    if let Some(button) = surface
        .root()
        .query_selector(":scope > .camp-actions > button:first-child")?
    {
        button.set_attribute("style", "white-space: pre-line")?;
    }
    Ok(())
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
    let portraits = view
        .journey
        .party
        .iter()
        .map(|member| super::sheet::player_portrait_token(&member.name))
        .collect::<Vec<_>>();
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
            portrait: Some(&portraits[index]),
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

fn is_extra_control(offer: &rpc::GameplayActionOffer) -> bool {
    offer.action_kind == rpc::GameplayActionKind::EndTurn as i32 || !offer.input_groups.is_empty()
}

fn aftermath_action(offer: &rpc::GameplayActionOffer) -> rpc::GameplayActionOffer {
    let mut action = offer.clone();
    if action.action_kind == rpc::GameplayActionKind::EndTurn as i32
        && action.label.to_lowercase().contains("death saves")
    {
        action.label = "Roll Death Saving Throw".to_owned();
    }
    action
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
        .map(|member| hit_point_detail(member, view.journey.combat.as_ref()))
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
    let rest = offer_of(view.offers, rpc::GameplayActionKind::ShortRest)
        .filter(|offer| offer.input_groups.is_empty());
    let rest_choice = offer_of(view.offers, rpc::GameplayActionKind::ShortRest)
        .filter(|offer| !offer.input_groups.is_empty());
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
    let rest_label = rest.map(|offer| short_rest_button_label(&offer.label));
    let roll_notice = roll_notice(view.journey);
    let choice_notice = result_line.as_ref().map_or_else(
        || "Choose what your hero does with the Hit Point Die.".to_owned(),
        |line| format!("{line} Choose what your hero does with the Hit Point Die."),
    );
    let campfire = CampfireView {
        owner_generation: revision.epoch().get(),
        revision: revision.sequence(),
        title: &view.scene.title,
        location,
        narration: view.narration,
        party_label: "Your party",
        notice: if let Some(roll) = roll_notice.as_deref() {
            roll
        } else if rest_choice.is_some() {
            &choice_notice
        } else {
            match (&result_line, proceed.is_some()) {
                (Some(line), _) => line,
                (None, true) => {
                    "Choose how the party spends its time. The server records the result."
                }
                (None, false) => "Waiting for the next choice.",
            }
        },
        connected,
        members: &members,
        rest: rest
            .zip(rest_label.as_deref())
            .map(|(offer, label)| CampfireOffer {
                id: &offer.offer_id,
                label,
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
    for offer in view.offers.iter().filter(|offer| is_extra_control(offer)) {
        crate::gameplay_browser::action_button(client, &extra, aftermath_action(offer))?;
    }
    let mounted = match &client.borrow().combat.mounted {
        Some(Mounted::Rest(surface)) => Some(Rc::clone(surface)),
        _ => None,
    };
    if let Some(surface) = mounted {
        if surface.update(&campfire).is_ok() {
            preserve_rest_label_layout(&surface)?;
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
    preserve_rest_label_layout(&surface)?;
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
    use super::{
        aftermath_action, condition, continue_label, dialogue_line, hit_point_detail,
        is_extra_control, own_hit_point_die_roll, roll_notice, short_rest_benefit,
        short_rest_button_label,
    };
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

    #[test]
    fn short_rest_button_uses_the_server_count_and_rule_benefit() {
        assert_eq!(
            short_rest_button_label("Short Rest · 1 hour · 1 Hit Point Die to spend"),
            "Short Rest · 1 hour · 1 Hit Point Die to spend\nRegain one use of Second Wind"
        );
        assert_eq!(short_rest_benefit(), "Regain one use of Second Wind");
    }

    #[test]
    fn short_rest_die_choices_are_rendered_as_extra_controls_srd_p187() {
        let offer = rpc::GameplayActionOffer {
            action_kind: rpc::GameplayActionKind::ShortRest as i32,
            input_groups: vec![rpc::JourneyGroup {
                group_id: "hit-point-die".to_owned(),
                label: "Choose for your hero".to_owned(),
                options: vec![rpc::JourneyOption {
                    option_id: "spend".to_owned(),
                    label: "Spend your Hit Point Die (d10 + 2)".to_owned(),
                    ..Default::default()
                }],
            }],
            ..Default::default()
        };

        assert!(is_extra_control(&offer));
    }

    #[test]
    fn short_rest_roll_is_shown_from_the_hero_own_sheet_srd_p187() {
        let journey = rpc::JourneyView {
            own_character: Some(rpc::CharacterSheet {
                facts: vec![rpc::SheetFact {
                    label: "Hit Point Die roll".to_owned(),
                    value: "d10: 8 + 2; healed 10; HP 13".to_owned(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(
            own_hit_point_die_roll(&journey),
            Some("d10: 8 + 2; healed 10; HP 13")
        );
        assert_eq!(own_hit_point_die_roll(&rpc::JourneyView::default()), None);
    }

    #[test]
    fn aftermath_death_save_notice_waits_for_release_and_keeps_rest_roll_priority() {
        let mut journey = rpc::JourneyView {
            combat: Some(rpc::CombatView {
                released_sequence: 3,
                death_save_outcomes: vec![rpc::CombatOutcome {
                    kind: rpc::OutcomeKind::DeathSave as i32,
                    sequence: 4,
                    actor_name: "Sigrun".into(),
                    attack_die: 12,
                    death_save_result: rpc::DeathSaveResult::Success as i32,
                    death_successes: 1,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(roll_notice(&journey), None);
        journey.combat.as_mut().unwrap().released_sequence = 4;
        let notice = roll_notice(&journey).unwrap();
        assert!(notice.contains("Sigrun"));
        assert!(notice.contains("d20 12"));
        assert!(notice.contains("1 successes, 0 failures"));
        journey
            .combat
            .as_mut()
            .unwrap()
            .death_save_outcomes
            .push(rpc::CombatOutcome {
                kind: rpc::OutcomeKind::DeathSave as i32,
                sequence: 3,
                actor_name: "Brakka".into(),
                attack_die: 18,
                death_save_result: rpc::DeathSaveResult::Success as i32,
                death_successes: 2,
                ..Default::default()
            });
        let both = roll_notice(&journey).unwrap();
        assert!(both.find("Brakka").unwrap() < both.find("Sigrun").unwrap());
        assert!(both.contains("d20 18"));
        assert!(both.contains("d20 12"));
        journey.own_character = Some(rpc::CharacterSheet {
            facts: vec![rpc::SheetFact {
                label: "Hit Point Die roll".into(),
                value: "d10 8 + 2 = 10".into(),
            }],
            ..Default::default()
        });
        assert!(roll_notice(&journey).unwrap().starts_with("Hit Point Die:"));
    }

    #[test]
    fn aftermath_hit_point_detail_shows_only_that_heros_death_save_tally() {
        let member = rpc::PartyMemberView {
            participant_id: vec![1],
            hit_points: 0,
            maximum_hit_points: 13,
            ..Default::default()
        };
        let combat = rpc::CombatView {
            death_saves: vec![rpc::DeathSaveState {
                participant_id: vec![1],
                successes: 1,
                failures: 2,
                ..Default::default()
            }],
            ..Default::default()
        };
        let detail = hit_point_detail(&member, Some(&combat));
        assert!(detail.contains("HP 0 / 13"));
        assert!(detail.contains(&super::super::combat::death_save_summary(
            &combat.death_saves[0]
        )));
        assert_eq!(hit_point_detail(&member, None), "HP 0 / 13");
    }

    #[test]
    fn aftermath_death_save_control_names_the_roll_and_preserves_the_server_offer() {
        let offer = rpc::GameplayActionOffer {
            action_kind: rpc::GameplayActionKind::EndTurn as i32,
            offer_id: "current-offer".into(),
            label: "Advance death saves".into(),
            ..Default::default()
        };
        let action = aftermath_action(&offer);
        assert_eq!(action.label, "Roll Death Saving Throw");
        assert_eq!(action.offer_id, offer.offer_id);
        assert_eq!(action.action_kind, offer.action_kind);
        let rest = rpc::GameplayActionOffer {
            action_kind: rpc::GameplayActionKind::ShortRest as i32,
            label: "Rest here".into(),
            ..Default::default()
        };
        assert_eq!(aftermath_action(&rest), rest);
    }
}
