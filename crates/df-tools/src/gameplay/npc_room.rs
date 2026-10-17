//! Listener-safe room knowledge and prepared answers from the committed scene model.

use serde_json::{Value, json};

use super::{NpcReplyContext, npc_dossier::Withheld};
use crate::gameplay::adventure::{
    dossier::{AnswerSubject, Dossier},
    scene_model::{SceneNow, ScenePerson, SceneTargetKind},
};
use crate::gameplay::npc_facts::AskReading;

pub(super) fn is_speaker(person: &ScenePerson, dossier: &Dossier) -> bool {
    dossier.npc_id == format!("bystander:{}", person.id)
        || dossier.npc_id.starts_with(&format!("{}:", person.id))
        || dossier.npc_id.ends_with(&format!(":{}", person.id))
}

pub(super) fn facts(room: &SceneNow, dossier: &Dossier, withheld: &Withheld) -> Value {
    let safe = |text: &str| !withheld.leaks_secret(text);
    let model = &room.model;
    json!({
        "time_of_day": model.time_of_day,
        "weather": model.weather,
        "people": model.people.iter().filter(|person| safe(&person.name) && safe(&person.description)).map(|person| json!({
            "id": person.id, "name": person.name, "role": (person.role != "speaker").then_some(&person.role),
            "look_and_doing": person.description, "position": person.position,
            "relation_to_host": person.relation_to_speaker,
        })).collect::<Vec<_>>(),
        "things": model.things.iter().filter(|thing| safe(&thing.short)).map(|thing| json!({
            "id": thing.id, "name": thing.name, "short": thing.short,
            "position": thing.position, "owner": thing.owner_id,
            "in_scene": thing.in_scene, "holder": thing.holder_id,
            "open": thing.open, "lit": thing.lit, "broken": thing.broken,
        })).collect::<Vec<_>>(),
        "ways_out": model.ways_out.iter().filter(|way| safe(&way.beyond)).map(|way| json!({
            "name": way.name, "position": way.position, "beyond": way.beyond, "open": way.open,
        })).collect::<Vec<_>>(),
        "wares": model.offers.iter().filter(|offer| safe(&offer.description)
            && model.offer_available(&offer.id)).map(|offer| json!({
            "name": offer.name, "description": offer.description, "cost_cp": offer.cost_cp,
            "seller": model.people.iter().find(|person| person.id == offer.seller_id).map(|person| &person.name),
        })).collect::<Vec<_>>(),
        "changes": room.changes.iter().filter(|change| safe(change)).collect::<Vec<_>>(),
        "your_knowledge": model.people.iter().find(|person| is_speaker(person, dossier))
            .map(|person| person.knowledge.iter().filter(|fact| safe(&fact.statement)).map(|fact| &fact.statement).collect::<Vec<_>>()),
    })
}

/// The room answer for a question about the room's people, things and exits. The question kind is
/// Jev's; a neutral reading asks no room question. Exact names of people, things and exits stay a
/// data match against the scene model.
pub(super) fn answer(
    context: &NpcReplyContext,
    reading: &AskReading,
    subject: AnswerSubject,
) -> Option<String> {
    let room = context.room.as_ref()?;
    let model = &room.model;
    let turn = context.turn.as_ref()?;
    let body = context.typed_words.to_lowercase();
    let words = body.as_str();
    let question = reading.room;
    if question.identity {
        return super::answer::identity(context).map(|(text, _)| text);
    }
    // A named object takes precedence over a person alias embedded in its name.
    let named_thing = model
        .things
        .iter()
        .find(|thing| thing.in_scene && words.contains(&thing.name.to_lowercase()));
    let target = named_thing
        .and_then(|thing| model.resolve_target(&format!("look at {}", thing.name)))
        .or_else(|| model.resolve_target(words))
        .or_else(|| {
            // A follow-up about "she" or "that" retains the last explicitly named room subject.
            question
                .follow_up
                .then(|| {
                    context
                        .turn
                        .as_ref()?
                        .ledger
                        .recent(context.member, &context.turn.as_ref()?.dossier.npc_id)
                        .iter()
                        .rev()
                        .filter(|(hero, _)| *hero)
                        .find_map(|(_, line)| model.resolve_target(line))
                })
                .flatten()
        });
    let answer = if let Some(target) = target {
        match target.kind {
            SceneTargetKind::Person => {
                // A person is described only when Jev read a question about people. A name in the
                // line alone never answers, so an errand question about an equal-named bystander
                // draws no room answer.
                let asks_about_person = question.identity
                    || question.self_name
                    || question.unknown_person
                    || question.follow_up
                    || question.who_here;
                if !asks_about_person {
                    return None;
                }
                let people = model
                    .people
                    .iter()
                    .filter(|person| target.ids.contains(&person.id))
                    .collect::<Vec<_>>();
                if people.len() == 1
                    && context
                        .turn
                        .as_ref()
                        .is_some_and(|turn| is_speaker(people[0], &turn.dossier))
                {
                    if question.self_name {
                        return super::answer::identity(context).map(|(text, _)| text);
                    } else {
                        return None;
                    }
                } else {
                    people
                        .iter()
                        .map(|person| {
                            // A follow-up about a named host's work says where they work with you.
                            if question.follow_up
                                && person.relation_to_speaker.starts_with("works ")
                            {
                                format!("{} {}.", person.name, person.relation_to_speaker)
                            } else if person.role == "speaker" {
                                format!("{} is {}.", person.name, person.position)
                            } else {
                                format!(
                                    "{} is the {} {}.",
                                    person.name, person.role, person.position
                                )
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                }
            }
            SceneTargetKind::Thing => {
                let thing = model
                    .things
                    .iter()
                    .find(|thing| target.ids.contains(&thing.id))?;
                if question.owner {
                    if let Some(owner) = thing
                        .owner_id
                        .as_ref()
                        .and_then(|id| model.people.iter().find(|person| &person.id == id))
                    {
                        format!("The {} belongs to {}.", thing.name, owner.name)
                    } else {
                        format!("I don't know who owns the {}.", thing.name)
                    }
                } else if let Some(holder) = &thing.holder_id {
                    let name = model
                        .people
                        .iter()
                        .find(|person| &person.id == holder)
                        .map_or(holder.as_str(), |person| person.name.as_str());
                    format!("{name} has the {} now.", thing.name)
                } else if thing.broken {
                    format!("The {} is broken now.", thing.name)
                } else {
                    format!(
                        "{} {}",
                        thing.short,
                        if thing.open { "It is open." } else { "" }
                    )
                    .trim()
                    .to_owned()
                }
            }
            SceneTargetKind::WayOut => {
                let way = model
                    .ways_out
                    .iter()
                    .find(|way| target.ids.contains(&way.id))?;
                format!(
                    "The {} leads to {}. It is {}.",
                    way.name,
                    way.beyond,
                    if way.open { "open" } else { "closed" }
                )
            }
            SceneTargetKind::Offer => {
                let offer = model
                    .offers
                    .iter()
                    .find(|offer| target.ids.contains(&offer.id))?;
                format!(
                    "{} It costs {} copper pieces.",
                    offer.description, offer.cost_cp
                )
            }
        }
    } else if question.who_here {
        let names = model
            .people
            .iter()
            .filter(|person| person.name != context.speaker)
            .map(|person| person.name.as_str())
            .collect::<Vec<_>>();
        format!(
            "{} are here with us. They may hear us if we speak aloud.",
            names.join(" and ")
        )
    } else if question.ways_out {
        model
            .ways_out
            .iter()
            .map(|way| format!("The {} leads to {}.", way.name, way.beyond))
            .collect::<Vec<_>>()
            .join(" ")
    } else if question.unknown_person {
        if subject != crate::gameplay::adventure::dossier::AnswerSubject::Other
            && turn.dossier.subject_answers.contains_key(&subject)
        {
            // Someone outside this room may still be part of the speaker's own errand.
            return None;
        }
        "I don't know who that is.".to_owned()
    } else {
        return None;
    };
    (!turn.withheld.leaks(&answer)).then_some(answer)
}
