use super::{
    ForcedEntries, GenerateOptions, GeneratedAdventure, Tables, TablesError,
    dossier::{
        AntagonistFact, AuthoredLine, Binding, CastMember, Dossier, DossierError, DossierFacts,
        Gender, Group, NpcRole, StakeFact, Topic, mentions_term, roll_dossiers, sentence_count,
    },
    dossier_render::render_dossier,
    dossier_tables::{DossierTables, FILES},
    generate,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

fn table_directory() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/adventure-tables")
}
fn chapter_tables() -> Tables {
    Tables::load(&table_directory()).unwrap()
}
fn dossier_tables() -> DossierTables {
    DossierTables::load(&table_directory()).unwrap()
}
fn seed(index: u64) -> u64 {
    index.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x5851_f42d_4c95_7f2d
}
fn adventure(tables: &Tables, seed: u64) -> GeneratedAdventure {
    generate(
        tables,
        seed,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap()
}
fn gender(facts: &BTreeMap<String, String>, prefix: &str) -> Gender {
    if facts[&format!("{prefix}.sub")] == "she" {
        Gender::She
    } else {
        Gender::He
    }
}
fn group_of(klass: &str) -> Group {
    match klass {
        "person" => Group::Person,
        "goblinoid" => Group::Goblinoid,
        "undead" => Group::Undead,
        _ => Group::Beast,
    }
}

/// Test-only adapter: what the coordinator's real adapter must pass for a generated adventure.
fn facts_from(tables: &Tables, adventure: &GeneratedAdventure) -> DossierFacts {
    let f = &adventure.facts;
    let mut cast = Vec::new();
    for (prefix, role) in [("patron", NpcRole::Patron), ("contact", NpcRole::Contact)] {
        cast.push(CastMember {
            id: format!("{prefix}:{}", f[&format!("{prefix}.id")]),
            name: f[&format!("{prefix}.first")].clone(),
            role,
            role_label: f[&format!("{prefix}.role")].clone(),
            archetype: f[&format!("{prefix}.id")].clone(),
            gender: gender(f, prefix),
            group: Group::Person,
            mannerism: Some(f[&format!("{prefix}.mannerism")].clone()),
            delivery: Some(f[&format!("{prefix}.delivery")].clone()),
            voice: Some(f[&format!("{prefix}.voice")].clone()),
        });
    }
    let creature = tables
        .antagonists
        .entries
        .iter()
        .find(|entry| entry.id == f["ant.id"])
        .unwrap();
    let group = group_of(&creature.klass);
    if creature.speaks {
        cast.push(CastMember {
            id: format!("antagonist:{}", creature.id),
            // "Snag the Goblin" is called Snag; "Wystan Oakes" keeps both names.
            name: f["ant.short"]
                .split(" the ")
                .next()
                .unwrap_or_default()
                .to_owned(),
            role: NpcRole::Antagonist,
            role_label: f["ant.the"].trim_start_matches("the ").to_owned(),
            archetype: creature.id.clone(),
            gender: gender(f, "ant"),
            group,
            mannerism: None,
            delivery: f.get("ant.delivery").cloned(),
            voice: Some(f["ant.voice"].clone()),
        });
    }
    DossierFacts {
        cast,
        stake: StakeFact {
            short: f["stake.short"].clone(),
            full: f["stake.full"].clone(),
        },
        location_id: f["loc.id"].clone(),
        location: f["loc.name"].clone(),
        destination: f["dest.name"].clone(),
        antagonist: AntagonistFact {
            short: f["ant.short"].clone(),
            the: f["ant.the"].clone(),
            group,
        },
        villain: None,
        faction: None,
        secret_id: f["secret.id"].clone(),
        secret_terms: vec![f["secret.fact"].clone(), f["secret.confirm"].clone()],
        authored: super::dossier_cast::authored_lines(&adventure.pack.scenes),
    }
}
/// The same adventure with the campaign layer present: a villain, a faction and a lieutenant.
fn campaign_facts(tables: &Tables, adventure: &GeneratedAdventure) -> DossierFacts {
    let mut facts = facts_from(tables, adventure);
    facts.villain = Some("the Winter Regent".into());
    facts.faction = Some("the Gray Mantle".into());
    facts.cast.push(CastMember {
        id: "villain:ritualist".into(),
        name: "Aldous".into(),
        role: NpcRole::Villain,
        role_label: "scholar-ritualist".into(),
        archetype: "ritualist".into(),
        gender: Gender::He,
        group: Group::Person,
        mannerism: None,
        delivery: Some("cold and precise".into()),
        voice: Some("precise, cold woman".into()),
    });
    facts.cast.push(CastMember {
        id: "lieutenant:raider-chief".into(),
        name: "Grik".into(),
        role: NpcRole::Lieutenant,
        role_label: "raider chief".into(),
        archetype: "raider-chief".into(),
        gender: Gender::She,
        group: Group::Goblinoid,
        mannerism: None,
        delivery: Some("loud and swaggering".into()),
        voice: Some("sharp, squeaky voice".into()),
    });
    facts
}
fn all_texts(dossier: &Dossier) -> Vec<(String, String)> {
    let mut texts = Vec::new();
    for item in &dossier.knowledge.says_freely {
        texts.push(("says_freely".to_owned(), item.text.clone()));
    }
    for (topic, text) in &dossier.prepared_answers {
        texts.push((format!("prepared.{}", topic.label()), text.clone()));
    }
    texts
}
fn files_with(file: &str, change: impl FnOnce(&mut Value)) -> Result<DossierTables, TablesError> {
    let mut files = BTreeMap::new();
    for name in FILES {
        files.insert(
            name.to_owned(),
            std::fs::read(table_directory().join(name)).unwrap(),
        );
    }
    let mut document: Value = serde_json::from_slice(&files[file]).unwrap();
    change(&mut document);
    files.insert(file.to_owned(), serde_json::to_vec(&document).unwrap());
    DossierTables::from_files(&files)
}
fn rejection_reason(result: Result<DossierTables, TablesError>) -> String {
    match result.unwrap_err() {
        TablesError::Invalid { reason, .. } => reason,
        other => panic!("expected a validation error, got {other}"),
    }
}

#[test]
fn shipped_tables_load_with_the_documented_dice() {
    let tables = dossier_tables();
    assert_eq!(tables.background.origins.die, 20);
    assert_eq!(tables.ties.patron.entries.len(), 20);
    assert_eq!(tables.ties.villain.die, 12);
    assert_eq!(tables.social.styles.entries.len(), 20);
    assert_eq!(tables.revision.len(), 64);
}
#[test]
fn every_secret_and_location_has_dossier_data() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    for secret in &chapter.secrets.entries {
        assert!(
            tables.knowledge.secret_keywords.contains_key(&secret.id),
            "secret {} has no keywords",
            secret.id
        );
    }
    for location in &chapter.locations.entries {
        assert!(
            tables.knowledge.place_specific.contains_key(&location.id),
            "location {} has no place lines",
            location.id
        );
    }
}
#[test]
fn same_seed_gives_identical_dossiers() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let generated = adventure(&chapter, seed(3));
    let facts = facts_from(&chapter, &generated);
    let first = roll_dossiers(&tables, &facts, generated.seed).unwrap();
    let second = roll_dossiers(&tables, &facts, generated.seed).unwrap();
    assert_eq!(first, second);
    let other = roll_dossiers(&tables, &facts, generated.seed ^ 1).unwrap();
    assert_ne!(first, other);
}

#[test]
fn generated_dossiers_keep_archetype_provenance_and_narrative_flavor() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let generated = adventure(&chapter, seed(31));
    let facts = facts_from(&chapter, &generated);
    let dossiers = roll_dossiers(&tables, &facts, generated.seed).unwrap();
    let dossier = dossiers
        .iter()
        .find(|dossier| dossier.role == NpcRole::Patron)
        .unwrap();
    let profile = dossier
        .archetype_profile
        .as_ref()
        .expect("patron role is covered by the curated library");

    assert_eq!(profile.library_version, 1);
    assert!(!profile.source_id.is_empty());
    assert_eq!(
        dossier
            .rolls
            .iter()
            .filter(|roll| roll.table.starts_with("dossier.archetype."))
            .count(),
        5
    );
    let profile_value = serde_json::to_value(profile).unwrap();
    for mechanical_key in ["bonus", "skill", "ability", "progression", "stat"] {
        assert!(profile_value.get(mechanical_key).is_none());
    }
    let rendered = render_dossier(dossier);
    assert!(rendered.contains("ROLE ARCHETYPE (narrative flavor only; no mechanics)"));
    assert!(rendered.contains(&profile.source_id));
    assert!(rendered.contains(&profile.experience.label));
    assert!(rendered.contains(&profile.competence.label));
}
#[test]
fn different_npc_ids_roll_on_independent_streams() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let generated = adventure(&chapter, seed(4));
    let mut facts = facts_from(&chapter, &generated);
    let mut twin = facts.cast[0].clone();
    twin.id = "patron:twin".into();
    facts.cast.push(twin);
    let dossiers = roll_dossiers(&tables, &facts, generated.seed).unwrap();
    let first = dossiers.first().unwrap();
    let twin = dossiers.last().unwrap();
    let naturals = |dossier: &Dossier| -> Vec<u8> {
        dossier
            .rolls
            .iter()
            .take(14)
            .map(|roll| roll.natural)
            .collect()
    };
    assert_ne!(naturals(first), naturals(twin));
}
#[test]
fn an_npc_is_unchanged_by_who_else_is_in_the_cast() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let generated = adventure(&chapter, seed(5));
    let full = facts_from(&chapter, &generated);
    let mut alone = full.clone();
    alone.cast.truncate(1);
    let with_company = roll_dossiers(&tables, &full, generated.seed).unwrap();
    let on_own = roll_dossiers(&tables, &alone, generated.seed).unwrap();
    let before_tie = |dossier: &Dossier| -> Vec<_> {
        dossier
            .rolls
            .iter()
            .take_while(|roll| roll.table != "dossier.story_tie")
            .cloned()
            .collect()
    };
    assert_eq!(before_tie(&with_company[0]), before_tie(&on_own[0]));
    assert!(before_tie(&on_own[0]).len() > 10);
}
#[test]
fn dossiers_leave_the_chapter_generator_untouched() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let before = adventure(&chapter, seed(6));
    let facts = facts_from(&chapter, &before);
    roll_dossiers(&tables, &facts, before.seed).unwrap();
    let after = adventure(&chapter, seed(6));
    assert_eq!(
        serde_json::to_vec(&before.pack).unwrap(),
        serde_json::to_vec(&after.pack).unwrap()
    );
    assert_eq!(before.rolls, after.rolls);
}
#[test]
fn every_role_gets_a_complete_dossier_for_real_adventures() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let mut roles = BTreeSet::new();
    for index in 0..30 {
        let generated = adventure(&chapter, seed(index));
        for facts in [
            facts_from(&chapter, &generated),
            campaign_facts(&chapter, &generated),
        ] {
            let dossiers = roll_dossiers(&tables, &facts, generated.seed).unwrap();
            assert_eq!(dossiers.len(), facts.cast.len());
            for dossier in &dossiers {
                roles.insert(dossier.role);
                assert!(dossier.rolls.len() > 20);
                assert!(
                    dossier
                        .rolls
                        .iter()
                        .enumerate()
                        .all(|(i, roll)| roll.n == i + 1 && roll.table.starts_with("dossier."))
                );
                for item in [
                    &dossier.origin,
                    &dossier.past_trade,
                    &dossier.formative_event,
                    &dossier.ideal,
                    &dossier.bond,
                    &dossier.flaw,
                    &dossier.want,
                    &dossier.fear,
                    &dossier.talent,
                ] {
                    assert!(!item.text.is_empty() && !item.text.contains(['{', '}']));
                }
                assert!(!dossier.story_tie.text.is_empty());
                assert!(!dossier.knowledge.says_freely.is_empty());
                assert!(!dossier.knowledge.says_when_trusted.is_empty());
                // The shame and the adventure's secret, then any authored line withheld for a
                // secret term.
                let never = &dossier.knowledge.never_says;
                assert_eq!(never[0].source, "flaw_shame");
                assert_eq!(never[1].source, "adventure_secret");
                assert!(
                    never[2..]
                        .iter()
                        .all(|item| item.source.ends_with("_secret"))
                );
                assert_eq!(dossier.prepared_answers.len(), Topic::ALL.len());
                for (topic, answer) in &dossier.prepared_answers {
                    let sentences = sentence_count(answer);
                    assert!(
                        (1..=3).contains(&sentences),
                        "{} {}: {answer}",
                        dossier.npc_id,
                        topic.label()
                    );
                    assert!(!answer.contains(['{', '}']));
                }
                assert_eq!(dossier.motive.is_some(), dossier.role.opposes_party());
                assert_eq!(dossier.stand_down.is_some(), dossier.role.opposes_party());
                assert!(!render_dossier(dossier).is_empty());
            }
        }
    }
    assert_eq!(roles.len(), 5, "roles seen: {roles:?}");
}
#[test]
fn speaking_antagonists_always_have_a_motive_and_a_stand_down() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let mut seen = 0;
    for index in 0..60 {
        let generated = adventure(&chapter, seed(index));
        let facts = facts_from(&chapter, &generated);
        for dossier in roll_dossiers(&tables, &facts, generated.seed).unwrap() {
            if dossier.role == NpcRole::Antagonist {
                seen += 1;
                assert!(dossier.motive.as_ref().is_some_and(|m| !m.text.is_empty()));
                assert!(
                    dossier
                        .stand_down
                        .as_ref()
                        .is_some_and(|s| !s.text.is_empty())
                );
                assert!(
                    dossier
                        .knowledge
                        .says_when_trusted
                        .iter()
                        .any(|item| item.source == "stand_down_line")
                );
            }
        }
    }
    assert!(seen >= 10, "only {seen} speaking antagonists in 60 seeds");
}
#[test]
fn every_story_tie_binds_to_a_fact_of_that_adventure() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let mut bindings = BTreeSet::new();
    for index in 0..60 {
        let generated = adventure(&chapter, seed(index));
        let facts = facts_from(&chapter, &generated);
        let campaign = campaign_facts(&chapter, &generated);
        for facts in [&facts, &campaign] {
            let ant_short = &facts.antagonist.short;
            let ant_the = &facts.antagonist.the;
            for dossier in roll_dossiers(&tables, facts, generated.seed).unwrap() {
                let tie = &dossier.story_tie;
                bindings.insert(format!("{:?}", tie.binds));
                let known: Vec<&str> = match tie.binds {
                    Binding::Stake => vec![&facts.stake.short, &facts.stake.full],
                    Binding::Antagonist => vec![ant_short, ant_the],
                    Binding::Location => vec![&facts.location],
                    Binding::Destination => vec![&facts.destination],
                    Binding::Villain => vec![facts.villain.as_deref().unwrap()],
                    Binding::Faction => vec![facts.faction.as_deref().unwrap()],
                    Binding::Cast => {
                        let other = facts
                            .cast
                            .iter()
                            .find(|member| Some(&member.id) == tie.bound_cast.as_ref())
                            .unwrap();
                        assert_ne!(other.id, dossier.npc_id);
                        vec![&other.name]
                    }
                };
                assert!(known.contains(&tie.bound_fact.as_str()), "{tie:?}");
                assert!(tie.text.contains(&tie.bound_fact), "{tie:?}");
                assert!(
                    tie.speak
                        .to_lowercase()
                        .contains(&tie.bound_fact.to_lowercase())
                        || tie.binds == Binding::Cast
                );
            }
        }
    }
    assert!(bindings.len() >= 6, "bindings exercised: {bindings:?}");
}
#[test]
fn ties_are_distinct_within_an_adventure_and_rarely_repeat_across_seeds() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let mut patron_triples = Vec::new();
    for index in 0..50 {
        let generated = adventure(&chapter, seed(index));
        let facts = campaign_facts(&chapter, &generated);
        let dossiers = roll_dossiers(&tables, &facts, generated.seed).unwrap();
        let ties: BTreeSet<&str> = dossiers.iter().map(|d| d.story_tie.id.as_str()).collect();
        assert_eq!(ties.len(), dossiers.len(), "seed index {index}");
        let patron = dossiers.iter().find(|d| d.role == NpcRole::Patron).unwrap();
        patron_triples.push((
            patron.origin.id.clone(),
            patron.want.id.clone(),
            patron.story_tie.id.clone(),
        ));
    }
    let repeated = patron_triples
        .iter()
        .filter(|triple| {
            patron_triples
                .iter()
                .filter(|other| other == triple)
                .count()
                > 1
        })
        .count();
    assert!(repeated <= 2, "{repeated} of 50 patrons share a triple");
}
#[test]
fn public_tiers_never_carry_secret_or_shame_text() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let mut checked = 0;
    for index in 0..200 {
        let generated = adventure(&chapter, seed(index));
        for facts in [
            facts_from(&chapter, &generated),
            campaign_facts(&chapter, &generated),
        ] {
            let mut terms: Vec<String> = facts.secret_terms.clone();
            terms.extend(tables.knowledge.secret_keywords[&facts.secret_id].clone());
            for dossier in roll_dossiers(&tables, &facts, generated.seed).unwrap() {
                assert_eq!(dossier.leak(), None);
                let shame = &dossier.knowledge.never_says[0].terms;
                for (field, text) in all_texts(&dossier) {
                    checked += 1;
                    for term in terms.iter().chain(shame) {
                        assert!(
                            !mentions_term(&text, term),
                            "{} {field} mentions {term}: {text}",
                            dossier.npc_id
                        );
                    }
                }
                if dossier.story_tie.public {
                    for term in terms.iter().chain(shame) {
                        assert!(!mentions_term(&dossier.story_tie.speak, term));
                        assert!(!mentions_term(&dossier.story_tie.text, term));
                    }
                }
                for item in &dossier.knowledge.says_when_trusted {
                    for answer in dossier.prepared_answers.values() {
                        assert!(!answer.contains(&item.text), "{answer}");
                    }
                    for term in shame {
                        assert!(!mentions_term(&item.text, term), "{}", item.text);
                    }
                }
            }
        }
    }
    assert!(checked > 5000);
}
#[test]
fn a_secret_term_in_a_public_line_is_reported_as_a_leak() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let generated = adventure(&chapter, seed(7));
    let facts = facts_from(&chapter, &generated);
    let mut dossier = roll_dossiers(&tables, &facts, generated.seed)
        .unwrap()
        .remove(0);
    let term = dossier.knowledge.never_says[1].terms[0].clone();
    dossier.knowledge.says_freely[0].text = format!("Between us, {term}.");
    assert!(dossier.leak().is_some());
    let shame = dossier.knowledge.never_says[0].terms[0].clone();
    let mut other = roll_dossiers(&tables, &facts, generated.seed)
        .unwrap()
        .remove(0);
    other.knowledge.says_when_trusted[0].text = format!("I will tell you: {shame}.");
    assert!(other.leak().is_some());
}
#[test]
fn style_and_temperament_never_clash_with_the_given_traits() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let clash = |left: &BTreeSet<String>, right: &BTreeSet<String>| {
        tables.social.incompatible.iter().any(|[a, b]| {
            (left.contains(a) && right.contains(b)) || (left.contains(b) && right.contains(a))
        })
    };
    let mut hurried_with_slow = 0;
    for index in 0..100 {
        let generated = adventure(&chapter, seed(index));
        let facts = campaign_facts(&chapter, &generated);
        for dossier in roll_dossiers(&tables, &facts, generated.seed).unwrap() {
            let given: BTreeSet<String> = dossier.given.tags.iter().cloned().collect();
            let mut chosen: BTreeSet<String> = dossier.speech_style.tags.iter().cloned().collect();
            assert!(!clash(&given, &chosen), "{} style", dossier.npc_id);
            for reaction in [
                &dossier.temperament.flattery,
                &dossier.temperament.flirting,
                &dossier.temperament.threats,
                &dossier.temperament.lies,
                &dossier.temperament.bribes,
            ] {
                let tags: BTreeSet<String> = reaction.tags.iter().cloned().collect();
                assert!(!clash(&given, &tags), "{} {}", dossier.npc_id, reaction.id);
                assert!(!clash(&chosen, &tags), "{} {}", dossier.npc_id, reaction.id);
                chosen.extend(tags);
            }
            if given.contains("slow") && chosen.contains("hurried") {
                hurried_with_slow += 1;
            }
        }
    }
    assert_eq!(hurried_with_slow, 0);
}
#[test]
fn given_traits_that_already_disagree_are_flagged_for_the_dm() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let generated = adventure(&chapter, seed(8));
    let mut facts = facts_from(&chapter, &generated);
    facts.cast.truncate(1);
    facts.cast[0].voice = Some("brisk, no-nonsense man".into());
    facts.cast[0].mannerism = Some("tugging at his sleeve".into());
    facts.cast[0].delivery = Some("formal and precise".into());
    let dossier = roll_dossiers(&tables, &facts, generated.seed)
        .unwrap()
        .remove(0);
    assert!(dossier.given.tags.contains(&"brisk".to_owned()));
    assert!(dossier.given.tags.contains(&"nervous".to_owned()));
    assert!(dossier.given.note.is_some());
    assert!(dossier.speech_style.tags.iter().all(|tag| tag != "slow"));
}
#[test]
fn starting_attitude_follows_the_role_weights() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let mut patron = BTreeMap::new();
    let mut antagonist = BTreeMap::new();
    for index in 0..300 {
        let generated = adventure(&chapter, seed(index));
        let facts = facts_from(&chapter, &generated);
        for dossier in roll_dossiers(&tables, &facts, generated.seed).unwrap() {
            let counts = match dossier.role {
                NpcRole::Patron => &mut patron,
                NpcRole::Antagonist => &mut antagonist,
                _ => continue,
            };
            *counts
                .entry(dossier.starting_attitude.label())
                .or_insert(0u32) += 1;
        }
    }
    let share = |counts: &BTreeMap<&str, u32>, key: &str| -> f64 {
        let total: u32 = counts.values().sum();
        f64::from(counts.get(key).copied().unwrap_or(0)) / f64::from(total)
    };
    assert!(share(&patron, "friendly") > 0.5, "{patron:?}");
    assert!(share(&antagonist, "hostile") > 0.5, "{antagonist:?}");
}
#[test]
fn bad_facts_and_tables_are_rejected_with_typed_errors() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let generated = adventure(&chapter, seed(9));
    let facts = facts_from(&chapter, &generated);
    let roll = |facts: &DossierFacts| roll_dossiers(&tables, facts, generated.seed);

    let mut empty = facts.clone();
    empty.cast.clear();
    assert_eq!(roll(&empty).unwrap_err(), DossierError::EmptyCast);

    let mut duplicate = facts.clone();
    duplicate.cast.push(duplicate.cast[0].clone());
    assert!(matches!(
        roll(&duplicate),
        Err(DossierError::DuplicateNpc(_))
    ));

    let mut silent = facts.clone();
    silent.cast[0].group = Group::Beast;
    assert!(matches!(roll(&silent), Err(DossierError::SilentSpeaker(_))));

    let mut secret = facts.clone();
    secret.secret_id = "not-a-secret".into();
    assert!(matches!(roll(&secret), Err(DossierError::UnknownSecret(_))));

    let mut blank = facts;
    blank.destination.clear();
    assert!(matches!(roll(&blank), Err(DossierError::EmptyFact(_))));
}
#[test]
fn table_validation_rejects_a_gap_in_a_die_range() {
    let reason = rejection_reason(files_with(FILES[0], |document| {
        document["tables"]["origins"]["entries"][1]["faces"] = json!([3, 3]);
    }));
    assert!(reason.contains("gap"), "{reason}");
}
#[test]
fn table_validation_rejects_a_range_that_stops_short_of_the_die() {
    let reason = rejection_reason(files_with(FILES[0], |document| {
        document["tables"]["talents"]["entries"]
            .as_array_mut()
            .unwrap()
            .pop();
    }));
    assert!(reason.contains("cover"), "{reason}");
}
#[test]
fn table_validation_rejects_an_unknown_slot() {
    let reason = rejection_reason(files_with(FILES[1], |document| {
        document["tables"]["bonds"]["entries"][0]["speak"] =
            json!("I carry {nonsense} everywhere.");
    }));
    assert!(reason.contains("unknown slot"), "{reason}");
}
#[test]
fn table_validation_rejects_an_oversized_entry() {
    let reason = rejection_reason(files_with(FILES[1], |document| {
        document["tables"]["ideals"]["entries"][0]["text"] = json!("x".repeat(400));
    }));
    assert!(reason.contains("characters"), "{reason}");
}
#[test]
fn table_validation_rejects_a_tie_that_does_not_name_its_fact() {
    let reason = rejection_reason(files_with(FILES[2], |document| {
        document["tables"]["patron"]["entries"][0]["binds"] = json!("destination");
    }));
    assert!(reason.contains("fact it binds"), "{reason}");
}
#[test]
fn table_validation_rejects_an_unknown_field_and_a_wrong_schema() {
    let error = files_with(FILES[3], |document| {
        document["tables"]["hints"]["entries"][0]["surprise"] = json!(1);
    })
    .unwrap_err();
    assert!(matches!(error, TablesError::Invalid { .. }));
    let reason = rejection_reason(files_with(FILES[3], |document| {
        document["schema"] = json!("df-npc-tables/9");
    }));
    assert!(reason.contains("schema"), "{reason}");
}
#[test]
fn table_validation_rejects_a_reaction_table_without_an_untagged_fallback() {
    let reason = rejection_reason(files_with(FILES[4], |document| {
        let entries = document["tables"]["threats"]["entries"]
            .as_array_mut()
            .unwrap();
        for entry in entries {
            entry["tags"] = json!(["gruff"]);
        }
    }));
    assert!(reason.contains("untagged"), "{reason}");
}
#[test]
fn whole_word_matching_ignores_case_and_word_fragments() {
    assert!(mentions_term("Someone is a LEAK, I fear.", "leak"));
    assert!(!mentions_term("The bleak road, and leaking roofs.", "leak"));
    assert!(mentions_term(
        "Say the Lamps are low tonight.",
        "lamps are low"
    ));
    assert!(!mentions_term("The lamps are very low.", "lamps are low"));
    assert!(!mentions_term("anything", ""));
    assert_eq!(sentence_count("One. Two! Three?"), 3);
    assert_eq!(sentence_count("Wait... what now?"), 2);
    assert_eq!(sentence_count("No full stop"), 1);
}
#[test]
fn dossiers_round_trip_through_the_adventure_record_format() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let generated = adventure(&chapter, seed(10));
    let facts = campaign_facts(&chapter, &generated);
    let dossiers = roll_dossiers(&tables, &facts, generated.seed).unwrap();
    let text = serde_json::to_string(&dossiers).unwrap();
    let back: Vec<Dossier> = serde_json::from_str(&text).unwrap();
    assert_eq!(dossiers, back);
    assert!(text.contains("\"says_when_trusted\""));
}
#[test]
fn no_speaker_is_ignorant_of_a_line_the_scenes_authored_for_them_across_200_seeds() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let mut checked = 0;
    for index in 0..200 {
        let generated = adventure(&chapter, seed(index));
        for facts in [
            facts_from(&chapter, &generated),
            campaign_facts(&chapter, &generated),
        ] {
            let dossiers = roll_dossiers(&tables, &facts, generated.seed).unwrap();
            for dossier in &dossiers {
                for line in facts
                    .authored
                    .iter()
                    .filter(|line| line.speaker == dossier.name)
                {
                    checked += 1;
                    assert!(!line.text.ends_with('?'), "{}", line.text);
                    let known = dossier
                        .knowledge
                        .says_freely
                        .iter()
                        .chain(&dossier.knowledge.says_when_trusted)
                        .map(|item| item.text.as_str())
                        .chain(
                            dossier
                                .knowledge
                                .never_says
                                .iter()
                                .map(|item| item.text.as_str()),
                        )
                        .any(|text| text == line.text);
                    assert!(known, "{} is ignorant of: {}", dossier.name, line.text);
                }
            }
        }
    }
    assert!(checked > 200, "only {checked} authored lines were checked");
}

/// The fixed-seed Kestrel adventure ties its urgent warning to the named contact.
#[test]
fn kestrel_of_the_fixed_seed_adventure_knows_the_contact_and_why_the_warning_is_urgent() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let generated = adventure(&chapter, 0xf4ec_5005_1a50_89bf);
    let facts = facts_from(&chapter, &generated);
    let dossiers = roll_dossiers(&tables, &facts, generated.seed).unwrap();
    let kestrel = dossiers
        .iter()
        .find(|dossier| dossier.name == "Kestrel")
        .expect("the saved run's patron is Kestrel");
    let freely: Vec<&str> = kestrel
        .knowledge
        .says_freely
        .iter()
        .map(|item| item.text.as_str())
        .collect();
    assert!(
        freely.iter().any(|text| {
            let text = text.to_lowercase();
            text.contains("warn jorun") && text.contains("cannot reach them in time")
        }),
        "{freely:?}"
    );
    assert!(
        freely.iter().any(|text| text.starts_with("Jorun is ")),
        "{freely:?}"
    );
    // The reply-time guard never withholds the names the scenes speak, and the destination
    // is spoken without its article.
    let spoken = crate::gameplay::npc_reply::spoken_names(&generated.pack);
    assert!(spoken.iter().any(|name| name == "Jorun"), "{spoken:?}");
    assert!(spoken.iter().any(|name| name == "Net Loft"), "{spoken:?}");
}

#[test]
fn the_patron_knows_her_errand_from_her_authored_lines_and_the_contact_identity() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let generated = adventure(&chapter, 0x3c6e_f378_dd81_de97);
    let facts = facts_from(&chapter, &generated);
    let dossiers = roll_dossiers(&tables, &facts, generated.seed).unwrap();
    let patron = dossiers
        .iter()
        .find(|dossier| matches!(dossier.role, NpcRole::Patron))
        .unwrap();
    let contact = facts
        .cast
        .iter()
        .find(|member| matches!(member.role, NpcRole::Contact))
        .unwrap();
    let freely = &patron.knowledge.says_freely;
    let identity = freely
        .iter()
        .find(|item| item.source == "identity_line")
        .expect("the patron names who the contact is");
    assert!(identity.text.contains(&contact.name), "{}", identity.text);
    assert!(
        identity.text.contains(&facts.destination),
        "{}",
        identity.text
    );
    // Every statement the patron makes in an authored scene is something she says freely here.
    let spoken: Vec<&AuthoredLine> = facts
        .authored
        .iter()
        .filter(|line| line.speaker == patron.name)
        .collect();
    assert!(!spoken.is_empty(), "the scenes give the patron lines");
    for line in spoken {
        assert!(
            freely
                .iter()
                .any(|item| item.source == "authored_line" && item.text == line.text),
            "{}",
            line.text
        );
    }
    // The adventure's own secret never reaches a public tier, however the errand is told.
    for item in freely {
        for forbidden in &patron.knowledge.never_says {
            for term in &forbidden.terms {
                assert!(!mentions_term(&item.text, term), "{}: {term}", item.text);
            }
        }
    }
}

/// Prints the sheets the handoff quotes: `cargo test ... print_review_sheets -- --ignored --nocapture`.
#[test]
#[ignore = "prints review sheets for people; asserts nothing"]
fn print_review_sheets() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    for seed in [
        0x3c6ef378dd81de97u64,
        0x5816609d5d59f515,
        0xf4ec50051a5089bf,
    ] {
        let generated = adventure(&chapter, seed);
        let facts = facts_from(&chapter, &generated);
        println!(
            "######## seed {seed:#x}: {} / {} / stake {} / antagonist {}",
            facts.location, facts.destination, facts.stake.short, facts.antagonist.short
        );
        for dossier in roll_dossiers(&tables, &facts, seed).unwrap() {
            println!("{}", render_dossier(&dossier));
        }
    }
    let generated = adventure(&chapter, seed(21));
    let facts = campaign_facts(&chapter, &generated);
    println!("######## campaign layer, seed index 21");
    for dossier in roll_dossiers(&tables, &facts, generated.seed).unwrap() {
        println!("{}", render_dossier(&dossier));
    }
}

/// The personal sources with how many lines each dossier holds of them.
const PERSONAL_SOURCES: [(&str, usize); 7] = [
    ("family_line", 1),
    ("children_line", 1),
    ("home_line", 1),
    ("service_line", 1),
    ("opinion_line", 2),
    ("courtship_line", 1),
    ("lore_line", 2),
];

#[test]
fn every_dossier_holds_a_personal_life_with_stable_ids() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let mut checked = 0;
    for index in 0..30 {
        let generated = adventure(&chapter, seed(index));
        let facts = facts_from(&chapter, &generated);
        for dossier in roll_dossiers(&tables, &facts, generated.seed).unwrap() {
            let freely = &dossier.knowledge.says_freely;
            for (source, count) in PERSONAL_SOURCES {
                let lines: Vec<_> = freely.iter().filter(|item| item.source == source).collect();
                assert_eq!(
                    lines.len(),
                    count,
                    "{} holds {source} {} time(s)",
                    dossier.npc_id,
                    lines.len()
                );
                for line in lines {
                    assert!(!line.text.contains(['{', '}']), "{}", line.text);
                    assert!(!line.text.contains([';', ':']), "{}", line.text);
                    assert!(
                        line.id.starts_with("personal:"),
                        "{} has no stable id",
                        line.text
                    );
                }
            }
            // Stable ids are unique within the dossier.
            let mut ids: Vec<&str> = freely
                .iter()
                .filter(|item| !item.id.is_empty())
                .map(|item| item.id.as_str())
                .collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(
                ids.len(),
                freely.iter().filter(|item| !item.id.is_empty()).count()
            );
            // One knowing line per other cast member, pointing at them.
            let knowing: Vec<_> = freely
                .iter()
                .filter(|item| item.source == "knowing_line")
                .collect();
            assert_eq!(knowing.len(), facts.cast.len() - 1, "{}", dossier.npc_id);
            for line in knowing {
                let pointed = line.about.as_deref().expect("a knowing line points");
                assert!(
                    facts.cast.iter().any(|member| member.id == pointed),
                    "{pointed} is nobody"
                );
                assert_eq!(line.id, format!("personal:knowing:{pointed}"));
                assert!(!line.text.contains(['{', '}']), "{}", line.text);
            }
            // The courtship answer matches the partner: tied with a living partner, free
            // without one. The model never invents a partner because the dossier decides.
            let partner = freely
                .iter()
                .find(|item| item.source == "family_line")
                .unwrap();
            let courtship = freely
                .iter()
                .find(|item| item.source == "courtship_line")
                .unwrap();
            let tied = ["wed-lodger", "sweetheart", "promised"]
                .iter()
                .any(|id| partner.id.ends_with(id));
            assert_eq!(
                courtship.id.starts_with("personal:courtship:tied-"),
                tied,
                "{} is {partner:?} but answers {courtship:?}",
                dossier.npc_id,
                partner = partner.text,
                courtship = courtship.text,
            );
            checked += 1;
        }
    }
    assert!(checked > 50, "{checked} dossiers");
}

#[test]
fn a_fighting_label_gets_a_service_history_and_a_trade_gets_a_trade() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let generated = adventure(&chapter, seed(7));
    let martial = ["retired soldier", "veteran of the wars", "watch sergeant"];
    for label in martial {
        let mut facts = facts_from(&chapter, &generated);
        facts.cast[0].role_label = label.into();
        let dossiers = roll_dossiers(&tables, &facts, generated.seed).unwrap();
        let service = dossiers[0]
            .knowledge
            .says_freely
            .iter()
            .find(|item| item.source == "service_line")
            .unwrap();
        assert!(service.id.contains("martial-"), "{label}: {}", service.text);
        assert!(
            !service.text.starts_with("I never wore a uniform"),
            "{label}: {}",
            service.text
        );
    }
    for label in ["bell ringer", "miller", "old mapmaker"] {
        let mut facts = facts_from(&chapter, &generated);
        facts.cast[0].role_label = label.into();
        let dossiers = roll_dossiers(&tables, &facts, generated.seed).unwrap();
        let service = dossiers[0]
            .knowledge
            .says_freely
            .iter()
            .find(|item| item.source == "service_line")
            .unwrap();
        assert!(service.id.contains("trade-"), "{label}: {}", service.text);
    }
}

#[test]
fn knowledge_without_ids_still_loads_for_older_saves() {
    let item: super::dossier::Knowledge = serde_json::from_value(json!({
        "topic": "your_past",
        "source": "event_line",
        "text": "I once cooked a feast for a hundred people from nearly nothing."
    }))
    .unwrap();
    assert!(item.id.is_empty());
    assert!(item.about.is_none());
}

#[test]
fn every_speaker_knows_their_own_side_of_the_errand_across_200_seeds() {
    let chapter = chapter_tables();
    let tables = dossier_tables();
    let mut checked = 0;
    for index in 0..200 {
        let generated = adventure(&chapter, seed(index));
        let facts = facts_from(&chapter, &generated);
        for dossier in roll_dossiers(&tables, &facts, generated.seed).unwrap() {
            let says_identity = dossier
                .knowledge
                .says_freely
                .iter()
                .any(|item| item.source == "identity_line");
            assert!(says_identity, "{} has no identity line", dossier.name);
            checked += 1;
        }
    }
    assert!(checked >= 400, "only {checked} speakers were checked");
}
