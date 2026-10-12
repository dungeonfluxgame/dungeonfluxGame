//! Scripted five-exchange conversations with the real text model, through the server's dialogue
//! path (PLAY-N06). One command plays every scenario in each listed content rating and writes
//! each transcript, the exact facts and prompt sent for every reply, and the tokens and cost, so
//! the replies can be read by a person and checked by `npc_talk_rules`. It is ignored by
//! default because it spends tokens.
//!
//! Environment: `OPENAI_API_KEY`, `DF_GAMEPLAY_TEXT_TOKEN_BUDGET`, `DF_GAMEPLAY_TEXT_CACHE` (a
//! fresh directory, so no reply is served from a cache), `DF_N06_OUT` (the run directory),
//! `DF_N06_SEEDS` (three adventure seeds, decimal or `0x` hex, the first two with a patron and
//! contact, the third and first also with a speaking antagonist), and optionally
//! `DF_N06_RATINGS` (default `mature,restrained`) and `DF_N06_ONLY` (scenario names to play).

use std::{fmt::Write as _, path::PathBuf, sync::Arc, time::Instant};

use df_protocol::common as rpc;
use serde_json::{Value, json};

use super::{
    adventure::dossier::{Dossier, ForbiddenScope, NpcRole, mentions_term},
    npc_dossier,
    npc_dossier_tests::{adventure, member, story, view},
    npc_reply::{NpcReplyContext, NpcVoice, reply_slots},
    pack::SceneKey,
    social::SocialLedger,
    startup_configuration::{
        ContentRating, StorytellerInputs, StorytellerMode, TestRating, storyteller_mode,
    },
    storyteller::{Expression, Storyteller, build_prompt},
    table_log::TableLog,
};

/// How the scripted hero treats the speaker.
#[derive(Clone, Copy)]
enum Tone {
    Earnest,
    Rude,
    Kind,
}

impl Tone {
    fn label(self) -> &'static str {
        match self {
            Self::Earnest => "earnest",
            Self::Rude => "rude",
            Self::Kind => "kind",
        }
    }
}

/// Whether the hero was privately told the scene's clue.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Clue {
    NotInvolved,
    Holder,
    Outsider,
}

struct Scenario {
    name: &'static str,
    /// Index into the run's three seeds.
    seed: usize,
    scene: SceneKey,
    speaker: NpcRole,
    tone: Tone,
    clue: Clue,
    hero: &'static str,
    lines: [&'static str; 5],
}

const SCENARIOS: [Scenario; 7] = [
    Scenario {
        name: "patron-earnest",
        seed: 0,
        scene: SceneKey::Opening,
        speaker: NpcRole::Patron,
        tone: Tone::Earnest,
        clue: Clue::NotInvolved,
        hero: "Brakka",
        lines: [
            "What is the trouble, {npc}? Why did you send for us?",
            "Tell me about {contact_full}. Can we trust them?",
            "Who or what might stand in our way on the road to {destination}?",
            "How long do we have, and what should we watch for?",
            "Anything else before we set out?",
        ],
    },
    Scenario {
        name: "contact-earnest",
        seed: 1,
        scene: SceneKey::Inn,
        speaker: NpcRole::Contact,
        tone: Tone::Earnest,
        clue: Clue::NotInvolved,
        hero: "Brakka",
        lines: [
            "{npc}? {patron} sent us. What have you got for us?",
            "What do you know about {stake}?",
            "Who is making trouble out here?",
            "What would you do in our place?",
            "Where do we go from here?",
        ],
    },
    Scenario {
        name: "witness-holder",
        seed: 0,
        scene: SceneKey::Opening,
        speaker: NpcRole::Patron,
        tone: Tone::Earnest,
        clue: Clue::Holder,
        hero: "Brakka",
        lines: [
            "{npc}, about what you told me quietly. Say more about it.",
            "Tell me about {contact_full} and {destination}.",
            "And the other way you hinted at. How do I find it?",
            "Why keep it between us? Who would stop us?",
            "What should I do first?",
        ],
    },
    Scenario {
        name: "witness-outsider",
        seed: 0,
        scene: SceneKey::Opening,
        speaker: NpcRole::Patron,
        tone: Tone::Earnest,
        clue: Clue::Outsider,
        hero: "Tamsin",
        lines: [
            "{npc}, what did you whisper to Brakka just now? I want to hear it too.",
            "Where does {stake} go, and who meets us there?",
            "Is there a quicker way to get there?",
            "Why keep secrets from us? Who would stop us?",
            "What should I do first?",
        ],
    },
    Scenario {
        name: "antagonist",
        seed: 2,
        scene: SceneKey::DialogueSeal,
        speaker: NpcRole::Antagonist,
        tone: Tone::Earnest,
        clue: Clue::NotInvolved,
        hero: "Brakka",
        lines: [
            "{npc}, who are you, and why do you block the road?",
            "What do you want from us?",
            "What will you do if we refuse?",
            "Is there any way to end this without a fight?",
            "Who sent you?",
        ],
    },
    Scenario {
        name: "patron-rude",
        seed: 1,
        scene: SceneKey::Opening,
        speaker: NpcRole::Patron,
        tone: Tone::Rude,
        clue: Clue::NotInvolved,
        hero: "Vex",
        lines: [
            "Quit stalling. What do you want from us?",
            "That tells me nothing. Who is {contact_full}, and why should I care?",
            "You are wasting my time. What is out on the road?",
            "Pathetic. Is that really all you know?",
            "Fine. Where do we go?",
        ],
    },
    Scenario {
        name: "contact-kind",
        seed: 2,
        scene: SceneKey::Inn,
        speaker: NpcRole::Contact,
        tone: Tone::Kind,
        clue: Clue::NotInvolved,
        hero: "Sunniva",
        lines: [
            "Thank you for seeing us, {npc}. {patron} spoke well of you.",
            "That sounds hard. What do you need from us most?",
            "I am sorry you carry this. Who is behind the trouble?",
            "We will do our best. What should we look out for?",
            "Thank you. What is our next step?",
        ],
    },
];

fn parse_seed(text: &str) -> u64 {
    let text = text.trim();
    match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).unwrap(),
        None => text.parse().unwrap(),
    }
}

fn rating_named(name: &str) -> ContentRating {
    match name.trim() {
        "mature" => ContentRating::Mature,
        "restrained" => ContentRating::Restrained,
        other => panic!("unknown rating {other}"),
    }
}

/// USD per million tokens of the configured text model (planning/pricing-and-costs.md).
const INPUT_PER_MILLION: f64 = 0.10;
const CACHED_INPUT_PER_MILLION: f64 = 0.01;
const OUTPUT_PER_MILLION: f64 = 0.50;

#[derive(Default, Clone, Copy)]
struct Spend {
    input: u128,
    cached: u128,
    output: u128,
    reasoning: u128,
}

impl Spend {
    fn cost(self) -> f64 {
        let fresh = self.input.saturating_sub(self.cached) as f64;
        (fresh * INPUT_PER_MILLION
            + self.cached as f64 * CACHED_INPUT_PER_MILLION
            + self.output as f64 * OUTPUT_PER_MILLION)
            / 1_000_000.0
    }

    fn add(&mut self, other: Spend) {
        self.input += other.input;
        self.cached += other.cached;
        self.output += other.output;
        self.reasoning += other.reasoning;
    }

    fn json(self) -> Value {
        json!({
            "input_tokens": self.input.to_string(),
            "cached_input_tokens": self.cached.to_string(),
            "output_tokens": self.output.to_string(),
            "reasoning_tokens": self.reasoning.to_string(),
            "usd": format!("{:.6}", self.cost()),
        })
    }
}

/// What the telemetry of the model calls since `from` adds up to.
fn spend_since(storyteller: &Storyteller, from: usize) -> Spend {
    let mut total = Spend::default();
    for usage in storyteller.usage_log().into_iter().skip(from) {
        total.add(Spend {
            input: usage.input_tokens,
            cached: usage.cached_input_tokens,
            output: usage.output_tokens,
            reasoning: usage.reasoning_output_tokens,
        });
    }
    total
}

fn fill(template: &str, names: &[(&str, String)]) -> String {
    let mut text = template.to_owned();
    for (key, value) in names {
        text = text.replace(&format!("{{{key}}}"), value);
    }
    text
}

fn dossier_json(dossier: &Dossier) -> Value {
    json!({
        "id": dossier.npc_id, "name": dossier.name, "role": dossier.role.label(),
        "role_label": dossier.role_label,
        "starting_attitude": dossier.starting_attitude.label(),
        "style_phrases": [
            dossier.speech_style.tic, dossier.speech_style.opener, dossier.speech_style.closer,
        ],
        "colour_lines": dossier
            .knowledge
            .says_freely
            .iter()
            .filter(|item| matches!(
                item.source.as_str(),
                "place_line" | "place_line2" | "world_line"
            ))
            .map(|item| item.text.clone())
            .collect::<Vec<_>>(),
        "own_lines": dossier
            .knowledge
            .says_freely
            .iter()
            .map(|item| item.text.clone())
            .collect::<Vec<_>>(),
    })
}

/// Plays every scenario in each rating against the real text model.
#[tokio::test(flavor = "current_thread")]
#[ignore = "calls the real text model and spends its token budget"]
async fn live_npc_talk_transcripts() {
    let mode = storyteller_mode(
        StorytellerInputs::from_environment(),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
    )
    .unwrap();
    assert!(
        matches!(mode, StorytellerMode::On(_)),
        "the storyteller is off: {mode:?}"
    );
    let storyteller = Arc::new(Storyteller::from_mode(mode));
    let voice = NpcVoice::new(storyteller.clone());
    let out = PathBuf::from(std::env::var("DF_N06_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let seeds: Vec<u64> = std::env::var("DF_N06_SEEDS")
        .unwrap()
        .split(',')
        .map(parse_seed)
        .collect();
    assert_eq!(seeds.len(), 3, "three seeds are needed");
    let ratings = std::env::var("DF_N06_RATINGS").unwrap_or_else(|_| "mature,restrained".into());
    let only: Option<Vec<String>> = std::env::var("DF_N06_ONLY").ok().map(|names| {
        names
            .split(',')
            .map(|name| name.trim().to_owned())
            .collect()
    });
    let label = std::env::var("DF_N06_LABEL").unwrap_or_default();

    let mut total = Spend::default();
    let mut calls = 0_u32;
    let mut fallbacks = 0_u32;
    let mut rule_findings = 0_usize;
    let mut conversations = Vec::new();
    let mut member_byte = 0_u8;
    for rating_name in ratings.split(',') {
        let rating = rating_named(rating_name);
        let _rating = TestRating::set(rating);
        for scenario in &SCENARIOS {
            if only
                .as_ref()
                .is_some_and(|only| !only.iter().any(|name| name == scenario.name))
            {
                continue;
            }
            member_byte += 1;
            let hero_member = member(member_byte);
            let seed = seeds[scenario.seed];
            let adventure = adventure(seed);
            let ledger = SocialLedger::open(adventure.dossiers.clone(), "0xlive", None).unwrap();
            let speaker = adventure
                .dossiers
                .iter()
                .find(|dossier| dossier.role == scenario.speaker)
                .unwrap_or_else(|| panic!("seed {seed}: no {:?} dossier", scenario.speaker))
                .clone();
            let fact = |key: &str| adventure.facts.get(key).cloned().unwrap_or_default();
            let names = [
                ("npc", speaker.name.clone()),
                ("patron", fact("patron.first")),
                ("contact_full", fact("contact.full")),
                (
                    "destination",
                    adventure
                        .pack
                        .destination_label
                        .clone()
                        .unwrap_or_default()
                        .trim_start_matches("the ")
                        .to_owned(),
                ),
                ("stake", fact("stake.short")),
            ];
            let mut base = view(&adventure, scenario.scene, scenario.hero);
            // The choices the story offers a hero in this scene, as the player screen shows them.
            let labels = &adventure.pack.offer_labels;
            let offered: Vec<&str> = match scenario.scene {
                SceneKey::Inn => vec![labels.short_rest.as_str()],
                SceneKey::DialogueEscort => vec![labels.choose_harbor_scene.as_str()],
                _ => vec![
                    labels.escort_courier.as_str(),
                    labels.choose_harbor_scene.as_str(),
                ],
            };
            if let Some(rpc::view_message::Audience::Player(player)) = base.audience.as_mut() {
                player.offers = offered
                    .into_iter()
                    .map(|label| rpc::GameplayActionOffer {
                        label: label.to_owned(),
                        ..Default::default()
                    })
                    .collect();
                if scenario.clue == Clue::Holder {
                    player.private_clue = adventure.pack.courier_clue.clone();
                }
            }
            let log = TableLog::new();
            let name = format!("{}-{}", scenario.name, rating_name);
            let directory = out.join(&name);
            std::fs::create_dir_all(&directory).unwrap();
            let mut text = format!(
                "=== {name}: seed {seed:#x}, {rating_name}, {} hero {}, scene {:?} ===\n",
                scenario.tone.label(),
                scenario.hero,
                scenario.scene
            );
            let _ = writeln!(
                text,
                "speaker {} ({} / {}), starts {}\nprivate clue held by hero: {}\n",
                speaker.name,
                speaker.role.label(),
                speaker.role_label,
                speaker.starting_attitude.label(),
                scenario.clue == Clue::Holder
            );
            if scenario.clue != Clue::NotInvolved {
                let _ = writeln!(text, "clue: {}\n", adventure.pack.courier_clue);
            }
            let mut exchanges = Vec::new();
            let mut conversation_spend = Spend::default();
            for (index, template) in scenario.lines.iter().enumerate() {
                let mut words = fill(template, &names);
                if scenario.speaker == NpcRole::Antagonist && !words.starts_with(&speaker.name) {
                    words = format!("{}, {words}", speaker.name);
                }
                let ask = NpcReplyContext::for_cast(
                    &base,
                    hero_member,
                    &words,
                    &story(&adventure),
                    &ledger,
                    &format!("{:?}", scenario.scene),
                )
                .unwrap_or_else(|| panic!("{name}: no reply context for exchange {}", index + 1));
                let turn = ask.turn.as_ref().expect("a dossier turn");
                let before = turn.standing;
                let slots = reply_slots(turn);
                let prompt = build_prompt(
                    &Expression {
                        purpose: "npc_reply",
                        facts: &ask.facts,
                        slots: &slots,
                        style: npc_dossier::STYLE,
                        profile: npc_dossier::profile(rating),
                    },
                    None,
                );
                let facts = ask.facts.clone();
                ask.publish_typed_line(&log, &ask.typed_words);
                let usage_before = storyteller.usage_log().len();
                let started = Instant::now();
                let exchange = voice.converse_in_character(ask, &log).await;
                let seconds = started.elapsed().as_secs_f64();
                calls += 1;
                let spend = spend_since(&storyteller, usage_before);
                total.add(spend);
                conversation_spend.add(spend);
                let after = ledger.standing(hero_member, &speaker);
                let (reply, from_model, approach, shift) = match &exchange {
                    Some(exchange) => (
                        exchange.text.clone(),
                        exchange.from_model,
                        exchange.approach.label(),
                        format!("{:?}", exchange.shift),
                    ),
                    None => (String::new(), false, "none", "None".into()),
                };
                if !from_model {
                    fallbacks += 1;
                }
                std::fs::write(directory.join(format!("prompt-{}.txt", index + 1)), &prompt)
                    .unwrap();
                let _ = writeln!(
                    text,
                    "[{}] HERO ({}): {words}\n    {} ({} -> {}, trust {}){}:\n    {reply}\n",
                    index + 1,
                    approach,
                    speaker.name.to_uppercase(),
                    before.attitude.label(),
                    after.attitude.label(),
                    after.trust,
                    if from_model {
                        ""
                    } else {
                        " [FALLBACK, not the model]"
                    },
                );
                exchanges.push(json!({
                    "n": index + 1,
                    "hero_words": words,
                    "reply": reply,
                    "from_model": from_model,
                    "approach": approach,
                    "shift": shift,
                    "attitude_before": before.attitude.label(),
                    "attitude_after": after.attitude.label(),
                    "trust_after": after.trust,
                    "facts": facts,
                    "prompt_bytes": prompt.len(),
                    "seconds": format!("{seconds:.1}"),
                    "spend": spend.json(),
                }));
            }
            let _ = writeln!(text, "spend: {}", conversation_spend.json());
            // What the listener may never hear from this speaker: the shame and secret terms
            // the server withheld for this hero, as the prompt builder computed them.
            let withheld_terms = {
                let ask = NpcReplyContext::for_cast(
                    &base,
                    hero_member,
                    "Hello.",
                    &story(&adventure),
                    &ledger,
                    &format!("{:?}", scenario.scene),
                )
                .unwrap();
                ask.turn.as_ref().unwrap().withheld.all().to_vec()
            };
            // The terms only the clue's holder may hear: secret terms the clue contains that the
            // scenes never say aloud.
            let spoken = super::npc_reply::spoken_names(&adventure.pack);
            let clue_only_terms: Vec<String> = adventure
                .pack
                .secret_terms
                .iter()
                .filter(|term| mentions_term(&adventure.pack.courier_clue, term))
                .filter(|term| !spoken.iter().any(|name| name.eq_ignore_ascii_case(term)))
                .cloned()
                .collect();
            let never_terms: Vec<String> = speaker
                .knowledge
                .never_says
                .iter()
                .filter(|item| item.scope == ForbiddenScope::Everywhere)
                .flat_map(|item| item.terms.clone())
                .collect();
            let record = json!({
                "run": label,
                "conversation": name,
                "seed": format!("{seed:#x}"),
                "rating": rating_name,
                "scene": format!("{:?}", scenario.scene),
                "tone": scenario.tone.label(),
                "hero": scenario.hero,
                "holds_private_clue": scenario.clue == Clue::Holder,
                "clue_in_play": scenario.clue != Clue::NotInvolved,
                "clue_text": if scenario.clue == Clue::NotInvolved {
                    String::new()
                } else {
                    adventure.pack.courier_clue.clone()
                },
                "deflects": scenario.clue == Clue::Outsider,
                "secret_terms": adventure.pack.secret_terms,
                "clue_only_terms": clue_only_terms,
                "never_terms": never_terms,
                "withheld_terms": withheld_terms,
                "speaker": dossier_json(&speaker),
                "all_speakers": adventure.dossiers.iter().map(dossier_json).collect::<Vec<_>>(),
                "exchanges": exchanges,
                "spend": conversation_spend.json(),
            });
            std::fs::write(
                directory.join("conversation.json"),
                serde_json::to_vec_pretty(&record).unwrap(),
            )
            .unwrap();
            std::fs::write(directory.join("transcript.txt"), &text).unwrap();
            // The model-free rules read the same record. Everything they find is written down
            // for a person to judge; a leak of the private clue or of a never-say term fails the run.
            let findings = super::npc_talk_rules::Transcript::from_json(&record)
                .map(|conversation| super::npc_talk_rules::check(&conversation))
                .expect("a recorded conversation");
            let report: String = findings
                .iter()
                .map(|finding| {
                    format!(
                        "{} (exchange {}): {}\n",
                        finding.check, finding.exchange, finding.detail
                    )
                })
                .collect();
            std::fs::write(directory.join("rules.txt"), &report).unwrap();
            rule_findings += findings.len();
            let hard: Vec<_> = findings
                .iter()
                .filter(|finding| {
                    matches!(
                        finding.check,
                        "leaked_the_private_clue" | "said_a_never_term"
                    )
                })
                .collect();
            assert!(hard.is_empty(), "{name}: {hard:?}");
            conversations.push(name);
        }
    }
    let summary = json!({
        "run": label,
        "model": storyteller.model(),
        "calls": calls,
        "fallbacks": fallbacks,
        "rule_findings": rule_findings,
        "conversations": conversations,
        "total_spend": total.json(),
    });
    std::fs::write(
        out.join(format!("summary-{}.json", ratings.replace(',', "-"))),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    println!("LIVE NPC TALK: {summary}");
}

/// Prints the generated facts of the seeds in `DF_N06_SEEDS`, for deciding what a speaker knows.
#[test]
#[ignore = "prints facts for planning the NPC knowledge"]
fn print_the_adventure_facts() {
    for seed in std::env::var("DF_N06_SEEDS").unwrap().split(',') {
        let adventure = adventure(parse_seed(seed));
        println!("== seed {seed}");
        for (key, value) in &adventure.facts {
            println!("{key} = {value}");
        }
    }
}
