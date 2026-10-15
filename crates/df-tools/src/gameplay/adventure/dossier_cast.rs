//! Adapter from a generated adventure to the NPC dossiers of its speaking cast; governing design:
//! adventure-generator §19.11. Pure: no I/O and no draw from the adventure's own dice stream
//! (dossiers roll on their own per-NPC streams), so attaching dossiers never changes what a seed
//! generates.
use super::{
    GenerateError, GeneratedAdventure, RollStatus, Tables,
    dossier::{
        AnswerSubject, AntagonistFact, AuthoredLine, CastMember, Dossier, DossierError,
        DossierFacts, Gender, Group, NpcRole, StakeFact, SubjectAnswer, mentions_term,
        roll_dossiers,
    },
};
use std::collections::BTreeMap;

/// What the party may already hear, and any secret the generator's own facts do not carry.
#[derive(Clone, Debug, Default)]
pub(super) struct PublicStory {
    /// Name or alias by which the party may already hear the villain spoken of.
    pub(super) villain: Option<String>,
    pub(super) faction: Option<String>,
    /// Extra phrases no public line may contain (a campaign's private lead, the villain's name).
    pub(super) extra_secret_terms: Vec<String>,
}

fn fact<'a>(facts: &'a BTreeMap<String, String>, key: &str) -> Result<&'a String, GenerateError> {
    facts
        .get(key)
        .ok_or_else(|| GenerateError::Pattern(format!("dossier fact {key}")))
}

fn gender_of(facts: &BTreeMap<String, String>, prefix: &str) -> Result<Gender, GenerateError> {
    Ok(if fact(facts, &format!("{prefix}.sub"))? == "she" {
        Gender::She
    } else {
        Gender::He
    })
}

fn group_of(class: &str) -> Group {
    match class {
        "person" => Group::Person,
        "goblinoid" => Group::Goblinoid,
        "undead" => Group::Undead,
        _ => Group::Beast,
    }
}

/// The statements each named speaker makes in the authored scenes, in scene order. A question is
/// a request the scene already made, not something the speaker knows, so it is left out.
pub(super) fn authored_lines(scenes: &[super::super::pack::Scene]) -> Vec<AuthoredLine> {
    scenes
        .iter()
        .filter(|scene| !scene.speaker.trim().is_empty())
        .flat_map(|scene| {
            statements(&scene.dialogue)
                .into_iter()
                .map(move |text| AuthoredLine {
                    speaker: scene.speaker.clone(),
                    text,
                })
        })
        .collect()
}

/// Sentences of `text` that end in `.` or `!`, each trimmed. A sentence ends at a terminal mark
/// followed by a space or the end of the text.
fn statements(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut start = 0;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        let ends_here = matches!(character, '.' | '!' | '?')
            && characters
                .peek()
                .is_none_or(|(_, next)| next.is_whitespace());
        if !ends_here {
            continue;
        }
        let end = index + character.len_utf8();
        let sentence = text[start..end].trim();
        if !sentence.is_empty() && !sentence.ends_with('?') {
            found.push(sentence.to_owned());
        }
        start = end;
    }
    let rest = text[start..].trim();
    if !rest.is_empty() && !rest.ends_with('?') {
        found.push(rest.to_owned());
    }
    found
}

/// The speaking cast and shared facts of an adventure: the patron and contact, plus the
/// antagonist only when it speaks. Ids are `"<role>:<archetype id>"`.
fn dossier_facts(
    tables: &Tables,
    adventure: &GeneratedAdventure,
    story: &PublicStory,
) -> Result<DossierFacts, GenerateError> {
    let facts = &adventure.facts;
    let mut cast = Vec::new();
    for (prefix, role) in [("patron", NpcRole::Patron), ("contact", NpcRole::Contact)] {
        let archetype = fact(facts, &format!("{prefix}.id"))?;
        cast.push(CastMember {
            id: format!("{}:{archetype}", role.label()),
            name: fact(facts, &format!("{prefix}.first"))?.clone(),
            role,
            role_label: fact(facts, &format!("{prefix}.role"))?.clone(),
            archetype: archetype.clone(),
            gender: gender_of(facts, prefix)?,
            group: Group::Person,
            mannerism: Some(fact(facts, &format!("{prefix}.mannerism"))?.clone()),
            delivery: Some(fact(facts, &format!("{prefix}.delivery"))?.clone()),
            voice: Some(fact(facts, &format!("{prefix}.voice"))?.clone()),
        });
    }
    let antagonist_id = fact(facts, "ant.id")?;
    let creature = tables
        .antagonists
        .entries
        .iter()
        .find(|entry| &entry.id == antagonist_id)
        .ok_or_else(|| GenerateError::Pattern("dossier antagonist".into()))?;
    let group = group_of(&creature.klass);
    if creature.speaks {
        let short = fact(facts, "ant.short")?;
        cast.push(CastMember {
            id: format!("{}:{}", NpcRole::Antagonist.label(), creature.id),
            // "Snag the Goblin" is called Snag; "Wystan Oakes" keeps both names.
            name: short.split(" the ").next().unwrap_or(short).to_owned(),
            role: NpcRole::Antagonist,
            role_label: fact(facts, "ant.the")?
                .trim_start_matches("the ")
                .to_owned(),
            archetype: creature.id.clone(),
            gender: gender_of(facts, "ant")?,
            group,
            mannerism: None,
            delivery: facts.get("ant.delivery").cloned(),
            voice: Some(fact(facts, "ant.voice")?.clone()),
        });
    }
    // The secret's id is a recorded roll, so it survives a campaign overlay that replaces the
    // rendered secret facts.
    let secret_id = adventure
        .rolls
        .iter()
        .find(|roll| roll.table == "secret" && roll.status != RollStatus::Rejected)
        .map(|roll| roll.entry_id.clone())
        .ok_or_else(|| GenerateError::Pattern("dossier secret".into()))?;
    let secret_terms = ["secret.fact", "secret.confirm"]
        .into_iter()
        .filter_map(|key| facts.get(key))
        .chain(&story.extra_secret_terms)
        .filter(|term| !term.trim().is_empty())
        .cloned()
        .collect();
    Ok(DossierFacts {
        authored: authored_lines(&adventure.pack.scenes),
        cast,
        stake: StakeFact {
            short: fact(facts, "stake.short")?.clone(),
            full: fact(facts, "stake.full")?.clone(),
        },
        location_id: fact(facts, "loc.id")?.clone(),
        location: fact(facts, "loc.name")?.clone(),
        destination: fact(facts, "dest.name")?.clone(),
        antagonist: AntagonistFact {
            short: fact(facts, "ant.short")?.clone(),
            the: fact(facts, "ant.the")?.clone(),
            group,
        },
        villain: story.villain.clone(),
        faction: story.faction.clone(),
        secret_id,
        secret_terms,
    })
}

/// Rolls the dossiers of an adventure's speaking cast from the adventure seed.
pub(super) fn roll_for(
    tables: &Tables,
    adventure: &GeneratedAdventure,
    story: &PublicStory,
) -> Result<Vec<Dossier>, GenerateError> {
    let facts = dossier_facts(tables, adventure, story)?;
    let mut dossiers = roll_dossiers(
        &tables.for_rating(adventure.rating).dossiers,
        &facts,
        adventure.seed,
    )
    .map_err(GenerateError::Dossier)?;
    for dossier in &mut dossiers {
        dossier.subject_answers = subject_answers(adventure, dossier)?;
        store_profile(dossier);
        if let Some(leak) = dossier.leak() {
            return Err(GenerateError::Dossier(DossierError::Leak {
                npc_id: dossier.npc_id.clone(),
                field: leak.field,
                term: leak.term,
            }));
        }
    }
    // Bystanders use independent per-person streams, so they never change the named cast's
    // rolls. Their room knowledge is their own; the patron's errand answers are not inherited.
    for setting in [
        &adventure.settings.meeting,
        &adventure.settings.confrontation,
        &adventure.settings.destination,
    ] {
        if let Some(model) = &setting.model {
            for person in model.people.iter().filter(|person| person.is_background) {
                let mut local = facts.clone();
                local.cast = vec![CastMember {
                    id: format!("bystander:{}", person.id),
                    name: person.name.clone(),
                    role: NpcRole::Contact,
                    role_label: person.role.clone(),
                    archetype: person.role.clone(),
                    gender: if person.role.contains("woman") {
                        Gender::She
                    } else {
                        Gender::He
                    },
                    group: Group::Person,
                    mannerism: Some(person.description.clone()),
                    delivery: None,
                    voice: None,
                }];
                local.authored.clear();
                for mut dossier in roll_dossiers(
                    &tables.for_rating(adventure.rating).dossiers,
                    &local,
                    adventure.seed,
                )
                .map_err(GenerateError::Dossier)?
                {
                    dossier.starting_attitude = match person.attitude {
                        super::scene_model::StartingAttitude::Friendly => {
                            super::dossier::Attitude::Friendly
                        }
                        super::scene_model::StartingAttitude::Indifferent => {
                            super::dossier::Attitude::Indifferent
                        }
                        super::scene_model::StartingAttitude::Hostile => {
                            super::dossier::Attitude::Hostile
                        }
                    };
                    // Preserve this person's generated biography, but not generic errand
                    // claims they were never given. Room facts still pass the existing secret
                    // guard; a lexical collision must not prevent the whole adventure loading.
                    dossier.knowledge.says_freely.retain(|fact| {
                        !matches!(
                            fact.source.as_str(),
                            "identity_line"
                                | "authored_line"
                                | "errand_line"
                                | "lore_line"
                                | "rumour_line"
                                | "hint_line"
                                | "stand_down_line"
                                | "world_line"
                                | "place_line"
                                | "place_line2"
                        )
                    });
                    let room_knowledge = person
                        .knowledge
                        .iter()
                        .filter(|fact| {
                            !dossier.knowledge.never_says.iter().any(|secret| {
                                secret
                                    .terms
                                    .iter()
                                    .any(|term| mentions_term(&fact.statement, term))
                            })
                        })
                        .map(|fact| super::dossier::Knowledge {
                            topic: super::dossier::Topic::ThePlace,
                            source: "place_line".to_owned(),
                            text: fact.statement.clone(),
                            id: fact.fact_id.clone(),
                            about: Some(person.id.clone()),
                        })
                        .collect::<Vec<_>>();
                    dossier.knowledge.says_freely.extend(room_knowledge);
                    dossier.knowledge.says_when_trusted.clear();
                    dossier.prepared_answers.insert(
                        super::dossier::Topic::WhoAreYou,
                        format!("I'm {}, the {}.", person.name, person.role),
                    );
                    dossier.prepared_answers.insert(
                        super::dossier::Topic::WhatAreYouHiding,
                        "I don't know about that.".to_owned(),
                    );
                    store_profile(&mut dossier);
                    if let Some(leak) = dossier.leak() {
                        return Err(GenerateError::Dossier(DossierError::Leak {
                            npc_id: dossier.npc_id,
                            field: leak.field,
                            term: leak.term,
                        }));
                    }
                    dossiers.push(dossier);
                }
            }
        }
    }
    Ok(dossiers)
}

fn store_profile(dossier: &mut Dossier) {
    use super::dossier::{Effect, FeelingKind, Forgiveness, GroundedFeeling, Length, Patience};
    let mut profile = dossier.emotional_profile();
    for (kind, because, text) in [
        (FeelingKind::Delight, "want", &dossier.want.text),
        (FeelingKind::Aversion, "flaw", &dossier.flaw.text),
    ] {
        let evidence: String = text.chars().take(120).collect();
        profile.feelings.push(GroundedFeeling {
            kind,
            text: evidence.clone(),
            because: because.to_owned(),
            evidence,
        });
    }
    profile.forgiveness = if dossier.temperament.threats.effect == Effect::Cool
        && dossier.temperament.lies.effect == Effect::Cool
    {
        Forgiveness::Unforgiving
    } else {
        Forgiveness::Forgiving
    };
    profile.patience = match dossier.speech_style.length {
        Length::Short => Patience::Low,
        Length::Medium => Patience::Middling,
        Length::Long => Patience::High,
    };
    dossier.emotional_profile = Some(profile);
}

fn subject_answers(
    adventure: &GeneratedAdventure,
    dossier: &Dossier,
) -> Result<BTreeMap<AnswerSubject, SubjectAnswer>, GenerateError> {
    let fact = |key: &str| {
        adventure
            .facts
            .get(key)
            .ok_or_else(|| GenerateError::Pattern(format!("dossier answer {key}")))
    };
    let scene_line = |key| {
        adventure
            .pack
            .scenes
            .iter()
            .find(|scene| scene.key == key)
            .map(|scene| scene.dialogue.as_str())
            .ok_or_else(|| GenerateError::Pattern(format!("dossier answer {key:?}")))
    };
    let (stake, full_stake, patron, contact, patron_role, contact_role, destination, location) = (
        fact("stake.short")?,
        fact("stake.full")?,
        fact("patron.first")?,
        fact("contact.first")?,
        fact("patron.role")?,
        fact("contact.role")?,
        fact("dest.name")?,
        fact("loc.name")?,
    );
    let opening = scene_line(super::super::pack::SceneKey::Opening)?;
    let why = scene_line(super::super::pack::SceneKey::DialogueSeal)?;
    let hook_kind = fact("hook.kind")?;
    let task = match hook_kind.as_str() {
        "deliver" => format!("take {stake} to {contact} at {destination}"),
        "escort" => format!("get me to {contact} at {destination}"),
        "warn" => format!("warn {contact} about {stake} at {destination}"),
        "retrieve" => format!("recover {full_stake} for {contact} at {destination}"),
        _ => return Err(GenerateError::Pattern("dossier answer hook.kind".into())),
    };
    let deadline = deadline_from(opening, why);
    let reward_roll = adventure
        .rolls
        .iter()
        .find(|roll| roll.table == "reward" && roll.status != RollStatus::Rejected)
        .ok_or_else(|| GenerateError::Pattern("dossier answer reward".into()))?;
    // The existing d12 reward row supplies a bounded 5–16 GP errand payment without another roll.
    let reward_face = if reward_roll.natural > 0 {
        reward_roll.natural
    } else {
        reward_roll
            .entry_id
            .strip_prefix("variant ")
            .and_then(|face| face.parse::<u8>().ok())
            .unwrap_or(1)
    };
    let reward_gp = u16::from(reward_face.saturating_sub(1)) + 5;
    let enemy_name = fact("ant.short")?;
    let enemy_kind = fact("ant.the")?;
    let enemy_appearance = adventure
        .facts
        .get("arrival")
        .ok_or_else(|| GenerateError::Pattern("dossier answer arrival".into()))?;
    let enemy_known = !dossier
        .knowledge
        .never_says
        .iter()
        .flat_map(|item| &item.terms)
        .any(|term| super::dossier::mentions_term(enemy_name, term));
    let enemy = if enemy_known {
        let mut facts = vec![
            format!("{enemy_name} is known as {enemy_kind}."),
            enemy_appearance.clone(),
        ];
        facts.extend(
            dossier
                .knowledge
                .says_freely
                .iter()
                .filter(|knowledge| knowledge.about.as_deref() == Some("antagonist"))
                .map(|knowledge| knowledge.text.clone()),
        );
        if let Some(rumour) = dossier
            .knowledge
            .says_freely
            .iter()
            .find(|knowledge| knowledge.source == "rumour_line")
        {
            facts.push(rumour.text.clone());
        }
        facts.join(" ")
    } else {
        "I do not know who is behind the trouble.".to_owned()
    };
    let people = if dossier.role == NpcRole::Contact {
        format!(
            "I'm {}, the {contact_role} at {destination}. {patron}, the {patron_role}, asked me to help.",
            dossier.name
        )
    } else if dossier.role == NpcRole::Patron {
        format!(
            "I'm {}, the {patron_role} at {location}. I asked {contact}, the {contact_role}, to meet you at {destination}.",
            dossier.name
        )
    } else {
        format!(
            "I'm {}, a {} at {location}. {patron}, the {patron_role}, asked {contact}, the {contact_role}, to meet you at {destination}.",
            dossier.name, dossier.role_label
        )
    };
    let deadline_answer = if deadline.contains("no exact time is stated") {
        format!("{contact} needs {stake} as soon as possible; no exact deadline is stated.")
    } else {
        format!("{contact} needs {stake} {deadline}; that is the deadline.")
    };
    let answers = BTreeMap::from([
        (
            AnswerSubject::Errand,
            SubjectAnswer {
                text: format!("{task}."),
                restatement: format!("That is the errand."),
            },
        ),
        (
            AnswerSubject::People,
            SubjectAnswer {
                text: people,
                restatement: format!("{contact} waits at {destination}."),
            },
        ),
        (
            AnswerSubject::Stakes,
            SubjectAnswer {
                text: format!("{stake} matters to {contact}; failure leaves them without it."),
                restatement: format!("{contact} would go without it."),
            },
        ),
        (
            AnswerSubject::Deadline,
            SubjectAnswer {
                text: deadline_answer,
                restatement: format!("Finish {deadline}."),
            },
        ),
        (
            AnswerSubject::Where,
            SubjectAnswer {
                text: format!("I'm at {location}; {contact} waits at {destination}."),
                restatement: format!("Go to {destination}."),
            },
        ),
        (
            AnswerSubject::Reward,
            SubjectAnswer {
                text: format!("I'll pay {reward_gp} GP."),
                restatement: format!("The pay is {reward_gp} GP."),
            },
        ),
        (
            AnswerSubject::Enemy,
            SubjectAnswer {
                text: enemy,
                restatement: if enemy_known {
                    format!("It is {enemy_kind}.")
                } else {
                    "I still do not know who is behind the trouble.".to_owned()
                },
            },
        ),
        (
            AnswerSubject::ErrandObject,
            SubjectAnswer {
                text: format!(
                    "{} is {full_stake}.",
                    stake.strip_prefix("the ").unwrap_or(stake)
                ),
                restatement: format!("It is {full_stake}."),
            },
        ),
    ]);
    Ok(answers
        .into_iter()
        .map(|(subject, answer)| {
            let leaks = |text: &str| {
                dossier
                    .knowledge
                    .never_says
                    .iter()
                    .flat_map(|item| &item.terms)
                    .any(|term| mentions_term(text, term))
            };
            if leaks(&answer.text) || leaks(&answer.restatement) {
                (subject, private_answer(subject))
            } else {
                (subject, answer)
            }
        })
        .collect())
}

fn private_answer(subject: AnswerSubject) -> SubjectAnswer {
    let (text, restatement) = match subject {
        AnswerSubject::Errand
        | AnswerSubject::ErrandObject
        | AnswerSubject::People
        | AnswerSubject::Stakes
        | AnswerSubject::Deadline
        | AnswerSubject::Where
        | AnswerSubject::Reward => ("I can't share that here.", "I can't say more about it."),
        AnswerSubject::Enemy => ("I do not know who is behind it.", "I still don't know."),
        AnswerSubject::SealedLetter => (
            "I do not know what it says; I have not opened it.",
            "I still do not know what it says.",
        ),
        AnswerSubject::Other => ("I can't share more here.", "I can't say more here."),
    };
    SubjectAnswer {
        text: text.to_owned(),
        restatement: restatement.to_owned(),
    }
}

fn deadline_from(opening: &str, why: &str) -> String {
    let words = format!("{opening} {why}").to_lowercase();
    [
        "before dawn",
        "before morning",
        "tonight",
        "before the wards fail completely",
    ]
    .into_iter()
    .find(|deadline| words.contains(deadline))
    .map(str::to_owned)
    .unwrap_or_else(|| "as soon as possible; no exact time is stated".to_owned())
}

#[cfg(test)]
mod tests {
    use super::super::dossier::{AnswerSubject, NpcRole};
    use std::collections::BTreeSet;

    #[test]
    fn every_errand_kind_answers_about_its_own_object() {
        let mut kinds = BTreeSet::new();
        for seed in 1..=40 {
            let adventure = crate::gameplay::npc_dossier_tests::adventure(seed);
            kinds.insert(adventure.facts["hook.kind"].clone());
            let object = &adventure.facts["stake.full"];
            let expected = format!(
                "{} is {object}.",
                adventure.facts["stake.short"]
                    .strip_prefix("the ")
                    .unwrap_or(&adventure.facts["stake.short"])
            );
            for dossier in adventure.dossiers {
                if dossier.npc_id.starts_with("bystander:") {
                    assert!(
                        !dossier
                            .subject_answers
                            .contains_key(&AnswerSubject::ErrandObject)
                    );
                } else {
                    assert_eq!(
                        dossier.subject_answers[&AnswerSubject::ErrandObject].text,
                        expected
                    );
                }
            }
        }
        assert_eq!(
            kinds,
            BTreeSet::from([
                "deliver".to_owned(),
                "escort".to_owned(),
                "retrieve".to_owned(),
                "warn".to_owned(),
            ])
        );
    }

    #[test]
    fn enemy_answer_uses_this_adventures_appearance_knowledge_and_rumour() {
        for seed in 1..=40 {
            let adventure = crate::gameplay::npc_dossier_tests::adventure(seed);
            let Some(dossier) = adventure
                .dossiers
                .iter()
                .find(|dossier| dossier.role == NpcRole::Patron)
            else {
                continue;
            };
            let answer = &dossier.subject_answers[&AnswerSubject::Enemy].text;
            if !answer.contains(" is known as ") {
                continue;
            }
            assert!(answer.contains(&adventure.facts["arrival"]));
            for fact in dossier.knowledge.says_freely.iter().filter(|fact| {
                fact.about.as_deref() == Some("antagonist") || fact.source == "rumour_line"
            }) {
                assert!(answer.contains(&fact.text), "{}", fact.source);
            }
            return;
        }
        panic!("no generated patron has public facts about the current enemy");
    }
}
