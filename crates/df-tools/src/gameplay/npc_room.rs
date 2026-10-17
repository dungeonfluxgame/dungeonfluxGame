//! Listener-safe room knowledge and prepared answers from the committed scene model.

use serde_json::{Value, json};

use super::{NpcReplyContext, npc_dossier::Withheld};
use crate::gameplay::adventure::{
    dossier::{AnswerSubject, Dossier},
    scene_model::{SceneNow, ScenePerson, SceneTargetKind},
};
use crate::gameplay::npc_facts::{AskReading, RoomReading};

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

/// Mentioning a person in a question does not address that person. A vocative or approaching
/// them does; target aliases are resolved against the scene model rather than invented names.
pub(super) fn addresses(words: &str, name: &str) -> bool {
    let line = words.trim().to_lowercase();
    let name = name.to_lowercase();
    let role_vocative = [
        "merchant,",
        "merchant:",
        "soldier,",
        "soldier:",
        "adventurer,",
        "adventurer:",
    ];
    line.starts_with(&format!("{name},"))
        || line.starts_with(&format!("{name}:"))
        || role_vocative.iter().any(|prefix| line.starts_with(prefix))
        || line.starts_with("i walk over to ")
        || line.starts_with("i talk to ")
        || line.starts_with("i ask ")
        || line.starts_with("excuse me, ")
        || line.starts_with("serving woman,")
        || line.starts_with("gentlemen")
        || line.starts_with("carters,")
}

pub(super) fn question_body(context: &NpcReplyContext) -> String {
    let lowered = context.typed_words.to_lowercase();
    let mut names = vec![context.speaker.to_lowercase()];
    if let Some(turn) = &context.turn {
        names.push(turn.dossier.name.to_lowercase());
        if let Some(person) = context.room.as_ref().and_then(|room| {
            room.model
                .people
                .iter()
                .find(|person| is_speaker(person, &turn.dossier))
        }) {
            names.extend(person.aliases.iter().map(|alias| alias.to_lowercase()));
        }
    }
    names.sort_by_key(|name| std::cmp::Reverse(name.len()));
    for name in names {
        for prefix in [
            format!("{name},"),
            format!("{name}:"),
            format!("i ask {name} "),
            format!("i talk to {name} "),
            format!("i walk over to {name} "),
        ] {
            if let Some(question) = lowered.strip_prefix(&prefix) {
                return question.trim().to_owned();
            }
        }
    }
    lowered
}

/// The room answer for a question about the room's people, things and exits. The question kind
/// is Jev's when Jev read the line; the keyword kind on the question body decides otherwise.
/// Exact names of people, things and exits stay a data match against the scene model.
pub(super) fn answer(
    context: &NpcReplyContext,
    reading: &AskReading,
    subject: AnswerSubject,
) -> Option<String> {
    let room = context.room.as_ref()?;
    let model = &room.model;
    let turn = context.turn.as_ref()?;
    let body = question_body(context);
    let words = body.as_str();
    let question = if reading.focus.is_some() {
        reading.room
    } else {
        RoomReading::keywords(words)
    };
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
                            if words.contains("work") && words.contains("you") {
                                let verb = if person.relation_to_speaker.starts_with("works ") {
                                    ""
                                } else {
                                    "is "
                                };
                                format!("{} {verb}{}.", person.name, person.relation_to_speaker)
                            } else {
                                if person.role == "speaker" {
                                    format!("{} is {}.", person.name, person.position)
                                } else {
                                    format!(
                                        "{} is the {} {}.",
                                        person.name, person.role, person.position
                                    )
                                }
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
        let subject = if reading.focus.is_some() {
            subject
        } else {
            super::super::social::classify_keywords_reading_for_dossier(words, &turn.dossier)
                .subject
        };
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
