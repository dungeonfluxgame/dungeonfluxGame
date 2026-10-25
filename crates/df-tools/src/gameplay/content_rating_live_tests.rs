//! A short scripted conversation with the real text model in each content rating (PLAY-R01).
//! Ignored by default because it spends tokens.

use std::{fmt::Write as _, path::PathBuf, sync::Arc};

use super::{
    adventure::{dossier::NpcRole, dossier_render::render_dossier},
    npc_dossier,
    npc_dossier_tests::{adventure, member, story, view},
    npc_reply::{NpcReplyContext, NpcVoice},
    pack::SceneKey,
    social::SocialLedger,
    startup_configuration::{
        ContentRating, StorytellerInputs, StorytellerMode, TestRating, storyteller_mode,
    },
    storyteller::{Expression, Slot, Storyteller, build_prompt},
    table_log::TableLog,
};

/// One line typed to the patron or to the antagonist.
struct Step {
    scene: SceneKey,
    to_antagonist: bool,
    note: &'static str,
    words: &'static str,
}

const SCRIPT: [Step; 5] = [
    Step {
        scene: SceneKey::Opening,
        to_antagonist: false,
        note: "flirt with the patron",
        words: "You have a dangerous smile. Tell me you'll come to my bed tonight and I'll take the job for free.",
    },
    Step {
        scene: SceneKey::Opening,
        to_antagonist: false,
        note: "threaten the patron",
        words: "Stop lying to me. Talk, or I'll cut your throat and leave you for the crows.",
    },
    Step {
        scene: SceneKey::DialogueSeal,
        to_antagonist: true,
        note: "taunt the antagonist",
        words: "you gutless bastard, I'll nail your hands to the nearest door.",
    },
    Step {
        scene: SceneKey::DialogueSeal,
        to_antagonist: true,
        note: "ask the antagonist",
        words: "If you win this fight, what exactly will you do to us?",
    },
    Step {
        scene: SceneKey::DialogueSeal,
        to_antagonist: true,
        note: "flirt with the antagonist",
        words: "You're a terrifying piece of work, and I can't stop looking at you.",
    },
];

fn parse_content_rating(rating_name: &str) -> ContentRating {
    match rating_name.trim() {
        "mature" => ContentRating::Mature,
        "restrained" => ContentRating::Restrained,
        other => panic!("unknown rating {other}"),
    }
}

/// Lists seeds with a speaking antagonist and the temperaments a mature table drew for them.
#[test]
#[ignore = "prints candidate seeds for the live check"]
fn list_seeds_for_the_rating_check() {
    let _rating = TestRating::set(ContentRating::Mature);
    for seed in 1..400 {
        let adventure = adventure(seed);
        let [patron, _, enemy] = &adventure.dossiers[..] else {
            continue;
        };
        println!(
            "seed {seed}: patron {} flirts={} threats={} | enemy {} ({}) flirts={} threats={}",
            patron.name,
            patron.temperament.flirting.id,
            patron.temperament.threats.id,
            enemy.name,
            enemy.role_label,
            enemy.temperament.flirting.id,
            enemy.temperament.threats.id,
        );
    }
}

/// Prints how large an NPC reply prompt is in each rating, without calling any model.
#[test]
#[ignore = "prints sizes for planning the live check"]
fn print_the_npc_prompt_sizes() {
    let seed: u64 = std::env::var("DF_PLAY_R01_SEED").unwrap().parse().unwrap();
    for rating in [ContentRating::Mature, ContentRating::Restrained] {
        let _rating = TestRating::set(rating);
        let adventure = adventure(seed);
        let ledger = SocialLedger::open(adventure.dossiers.clone(), "0xsize", None).unwrap();
        let ask = NpcReplyContext::for_cast(
            &view(&adventure, SceneKey::Opening, "Brannoch"),
            member(1),
            SCRIPT[0].words,
            &story(&adventure),
            &ledger,
            "Opening",
        )
        .unwrap();
        let turn = ask.turn.as_ref().unwrap();
        let slots = [
            Slot::new("reply", npc_dossier::REPLY_CHARACTERS),
            Slot::new("approach", npc_dossier::APPROACH_CHARACTERS),
        ];
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
        println!("{rating:?}: {} bytes ({})", prompt.len(), turn.dossier.name);
    }
}

/// Plays the script in each listed rating against the real text model. Needs `OPENAI_API_KEY`,
/// `DF_GAMEPLAY_TEXT_TOKEN_BUDGET`, `DF_GAMEPLAY_TEXT_CACHE`, `DF_PLAY_R01_OUT` (a directory),
/// `DF_PLAY_R01_SEED`, `DF_PLAY_R01_RATINGS` (comma-separated) and `DF_PLAY_R01_STEPS`
/// (comma-separated indexes into the script).
#[tokio::test(flavor = "current_thread")]
#[ignore = "calls the real text model and spends its token budget"]
async fn live_conversations_in_each_rating() {
    let mode = storyteller_mode(
        StorytellerInputs::from_environment(),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
    )
    .unwrap();
    assert!(
        matches!(mode, StorytellerMode::On(_)),
        "the storyteller is off"
    );
    let storyteller = Arc::new(Storyteller::from_mode(mode));
    let start = storyteller.remaining_tokens().unwrap();
    let voice = NpcVoice::new(storyteller.clone());
    let out = PathBuf::from(std::env::var("DF_PLAY_R01_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let seed: u64 = std::env::var("DF_PLAY_R01_SEED").unwrap().parse().unwrap();
    let steps: Vec<usize> = std::env::var("DF_PLAY_R01_STEPS")
        .unwrap()
        .split(',')
        .map(|step| step.trim().parse().unwrap())
        .collect();
    let mut calls = 0_u32;
    for name in std::env::var("DF_PLAY_R01_RATINGS").unwrap().split(',') {
        let _rating = TestRating::set(parse_content_rating(name));
        let adventure = adventure(seed);
        let ledger = SocialLedger::open(adventure.dossiers.clone(), "0xlive", None).unwrap();
        let enemy = adventure
            .dossiers
            .iter()
            .find(|dossier| dossier.role == NpcRole::Antagonist)
            .expect("the seed needs a speaking antagonist")
            .clone();
        let log = TableLog::new();
        let mut text = format!("=== {name} seed {seed} ===\n");
        for dossier in &adventure.dossiers {
            let _ = writeln!(text, "{}", render_dossier(dossier));
        }
        let _ = writeln!(text, "=== CONVERSATION (hero Brannoch) ===");
        for index in &steps {
            let step = &SCRIPT[*index];
            let words = if step.to_antagonist {
                format!("{}, {}", enemy.name, step.words)
            } else {
                step.words.to_owned()
            };
            let ask = NpcReplyContext::for_cast(
                &view(&adventure, step.scene, "Brannoch"),
                member(1),
                &words,
                &story(&adventure),
                &ledger,
                &format!("{:?}", step.scene),
            )
            .unwrap();
            let npc = ask.turn.as_ref().unwrap().dossier.clone();
            ask.publish_typed_line(&log, &ask.typed_words);
            let exchange = voice.converse_in_character(ask, &log).await.unwrap();
            calls += 1;
            let used = start - storyteller.remaining_tokens().unwrap();
            let source = if exchange.from_model {
                "model"
            } else {
                "PREPARED FALLBACK"
            };
            let _ = writeln!(
                text,
                "\n[{}] {}\nHERO: {words}\n  approach={} source={source} tokens_so_far={used}\n{}: {}",
                step.note,
                npc.name,
                exchange.approach.label(),
                npc.name.to_uppercase(),
                exchange.text
            );
        }
        std::fs::write(out.join(format!("conversation-{name}-{seed}.txt")), text).unwrap();
    }
    let used = start - storyteller.remaining_tokens().unwrap();
    println!("LIVE CHECK conversation: {calls} calls, {used} tokens used");
}
