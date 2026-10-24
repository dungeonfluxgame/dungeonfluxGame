use super::*;
use crate::gameplay::startup_configuration::ApproachSettings;
use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply, cache_directory, teller};
use std::time::Duration;

const HS67_BREAD_FACT: &str =
    "Nella's bread has a brown crust, pale open crumb and warm yeasty smell, baked this morning.";
const HS67_IMPRESSION: &str =
    "The bread has a brown crust, pale open crumb and a warm yeasty smell.";

fn hs67_bread() -> (
    Situation,
    FreeformPlan,
    super::super::npc_reply::NpcReplyContext,
) {
    let mut situation = shop();
    situation.hero.name = "Pip Thistledown".into();
    situation.things.push(ThingFact {
        owner_id: Some("nella".into()),
        ..thing_fact(
            "bread",
            "loaf of bread",
            "A loaf of bread on the stall.",
            true,
            false,
        )
    });
    let plan = FreeformPlan {
        sense: Some(Sense::Smell),
        ..plan(
            ActKind::Sense,
            Some(target(&situation, "bread")),
            Difficulty::None,
        )
    };
    let view = rpc::ViewMessage {
        audience: Some(rpc::view_message::Audience::Player(
            rpc::PlayerGameplayView {
                narration: "Basket Court has a bread stall.".into(),
                scene: Some(rpc::GameplayScene {
                    title: "Basket Court".into(),
                    ..Default::default()
                }),
                journey: Some(rpc::JourneyView {
                    speaker: "Nella".into(),
                    dialogue: "I baked these this morning.".into(),
                    own_character: Some(rpc::CharacterSheet {
                        name: "Pip Thistledown".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    let story = pack::with_content(super::super::npc_reply::StoryFacts::from_pack);
    let mut context = super::super::npc_reply::NpcReplyContext::from_view(
        &view,
        MemberId::from_bytes(&[0x67; 16]).unwrap(),
        "I inspect the bread.",
        &story,
    )
    .unwrap();
    context.npc_id = "scene-person:nella".into();
    context.actor_id = "nella".into();
    context.facts["listener_knowledge"] = json!({"claim":"I baked these this morning.", "known_truth":"Nella baked this bread this morning and is honestly describing it."});
    (situation, plan, context)
}

fn hs67_voice(
    endpoint: &FakeEndpoint,
    jev: &FakeEndpoint,
    name: &str,
) -> (
    super::super::npc_reply::NpcVoice,
    std::sync::Arc<super::super::npc_reply::local_lore::LocalLore>,
) {
    let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS67")
        .join(format!("{name}-{}", std::process::id()));
    let lore = std::sync::Arc::new(
        super::super::npc_reply::local_lore::LocalLore::open(name, None).unwrap(),
    );
    let voice = super::super::npc_reply::NpcVoice::new(std::sync::Arc::new(teller(
        endpoint.address,
        20_000,
        Duration::from_secs(5),
        &cache,
    )))
    .with_approach_model(crate::gameplay::approach_model::tests::model(jev))
    .with_local_lore(lore.clone());
    (voice, lore)
}

fn hs67_backfill_replies() -> Vec<Reply> {
    vec![
        Reply::Output(
            json!({"questions":json!(["What are the crust, crumb, smell and freshness of Nella's bread?"]).to_string()}),
        ),
        Reply::Output(
            json!({"fact":json!({"name":"Bread senses", "content":HS67_BREAD_FACT,
            "answer":"My bread has a brown crust, pale open crumb and warm yeasty smell, baked this morning."}).to_string()}),
        ),
    ]
}

#[tokio::test(flavor = "current_thread")]
async fn hs67_freeform_action_requested_bread_check_corrects_skill_rolls_both_and_reuses_canon() {
    let (situation, plan, mut context) = hs67_bread();
    let mut replies = hs67_backfill_replies();
    replies.push(Reply::Output(json!({"impression":HS67_IMPRESSION,"quality":"The loaf is fresh from this morning's baking."})));
    replies.push(Reply::Output(json!({"impression":HS67_IMPRESSION})));
    let endpoint = FakeEndpoint::start(replies);
    let jev = FakeEndpoint::start(vec![
        answer("o7"), // Both checks, sensory skill corrected.
        answer("o2"),
        answer("o1"),
        answer("o1"), // REF-069 backfill, harmless, approve.
        answer("o1"),
        answer("o1"), // Honest Easy reading, approve impression.
        answer("o1"),
        answer("o1"), // Later plain smell: no check, approve reused impression.
    ]);
    let (voice, lore) = hs67_voice(&endpoint, &jev, "bread-compound");
    let words = "I study the bread closely, its crust and crumb, and breathe in its smell. I make an Insight roll to judge how fresh it really is and whether Nella's being straight with me.";
    let mut resolution = run(&situation, &plan, words, 16);
    let additional = refine_sense(
        &voice,
        Some(&mut context),
        &mut resolution,
        words,
        16,
        fair(3),
        || Ok(12),
    )
    .await;
    let primary = performance(&resolution).check.as_ref().unwrap();
    assert_eq!(primary.label, "Wisdom (Perception)");
    assert_eq!(primary.die, 16);
    assert_eq!(additional.len(), 1);
    assert_eq!(additional[0].label, "Wisdom (Insight)");
    assert_eq!(additional[0].die, 12);
    assert!(additional[0].success);
    assert!(primary.row().roll.is_some());
    assert!(additional[0].row().roll.is_some());
    let line = narrate(voice.hero_identity_models().0, &resolution, words).await;
    let log = TableLog::new();
    publish_hero_line(&log, &resolution.hero, 0, words);
    publish_compound_outcome(&log, 0, Some(primary.row()), &additional, &line);
    let lines = log.lines();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0].speaker, "Pip Thistledown");
    assert_eq!(lines[1].roll.as_ref().unwrap().faces, vec![16]);
    assert_eq!(lines[2].roll.as_ref().unwrap().faces, vec![12]);
    assert!(lines[1].roll.as_ref().unwrap().succeeded);
    assert!(lines[2].roll.as_ref().unwrap().succeeded);
    assert_eq!(lines[3].speaker, "Narrator");
    assert!(
        lines
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    assert!(
        line.contains("Wisdom (Perception) for senses and Wisdom (Insight) for honesty."),
        "{line}"
    );
    assert!(
        line.contains("brown crust") && line.contains("warm yeasty smell"),
        "{line}"
    );
    assert!(
        line.contains("fresh from this morning") && line.contains("Nella's claim reads as honest"),
        "{line}"
    );
    assert_eq!(lore.development_decisions().len(), 1);
    println!(
        "HS67 compound: {line}\n{}\n{}",
        primary.line(),
        additional[0].line()
    );
    let mut plain = run(&situation, &plan, "I smell the bread.", 1);
    let (_, _, mut later_context) = hs67_bread();
    let extra = refine_sense(
        &voice,
        Some(&mut later_context),
        &mut plain,
        "I breathe in the loaf's scent.",
        1,
        |_, _, _| panic!("plain smell never rolls"),
        || panic!("plain smell never draws"),
    )
    .await;
    assert!(extra.is_empty());
    assert!(performance(&plain).check.is_none());
    assert!(fallback_line(&plain).contains(HS67_IMPRESSION));
    assert!(!fallback_line(&plain).contains("fresh from"));
    assert_eq!(
        lore.development_decisions().len(),
        1,
        "one backfill across paraphrased senses"
    );
    let bodies = jev.bodies();
    assert!(bodies[4].to_string().contains("honestly describing it"));
    assert_eq!(
        endpoint.bodies().len(),
        4,
        "second question reuses accepted REF-069 fact"
    );
    println!("HS67 plain: {}", fallback_line(&plain));
}

#[tokio::test(flavor = "current_thread")]
async fn hs67_freeform_action_failed_bread_checks_keep_impression_without_quality_or_false_suspicion()
 {
    let (situation, plan, mut context) = hs67_bread();
    let mut replies = hs67_backfill_replies();
    replies.push(Reply::Output(json!({"impression":HS67_IMPRESSION})));
    let endpoint = FakeEndpoint::start(replies);
    let jev = FakeEndpoint::start(vec![
        answer("o7"),
        answer("o2"),
        answer("o1"),
        answer("o1"),
        answer("o1"),
        answer("o1"),
    ]);
    let (voice, _) = hs67_voice(&endpoint, &jev, "bread-failure");
    let mut resolution = run(
        &situation,
        &plan,
        "I roll Insight to judge the loaf and Nella's honesty.",
        1,
    );
    let additional = refine_sense(
        &voice,
        Some(&mut context),
        &mut resolution,
        "I roll Insight to judge the loaf and Nella's honesty.",
        1,
        fair(0),
        || Ok(2),
    )
    .await;
    let line = fallback_line(&resolution);
    assert!(!performance(&resolution).succeeded);
    assert_eq!(narration_facts(&resolution, "")["outcome"], "failed");
    assert!(!additional[0].success);
    assert!(line.contains(HS67_IMPRESSION), "{line}");
    assert!(
        line.contains("cannot tell whether Nella's claim is honest"),
        "{line}"
    );
    assert!(
        !line.contains("morning") && !line.contains("nothing new") && !line.contains("evasion"),
        "{line}"
    );
    let sent = endpoint.bodies();
    assert!(
        !sent
            .last()
            .unwrap()
            .to_string()
            .contains("\"quality\":{\"type\"")
    );
    println!("HS67 failed: {line}");
}

#[tokio::test(flavor = "current_thread")]
async fn hs67_freeform_action_person_honesty_uses_only_insight_and_never_sensory_backfill() {
    let (mut situation, _, mut context) = hs67_bread();
    situation.people.push(PersonFact {
        id: "nella".into(),
        name: "Nella".into(),
        role: rpc::SceneRole::Contact,
        passive_perception: 10,
    });
    let plan = FreeformPlan {
        sense: Some(Sense::Sight),
        ..plan(
            ActKind::Sense,
            Some(target(&situation, "nella")),
            Difficulty::Easy,
        )
    };
    let endpoint = FakeEndpoint::start(vec![]);
    let jev = FakeEndpoint::start(vec![answer("o3"), answer("o1")]);
    let (voice, lore) = hs67_voice(&endpoint, &jev, "person-insight");
    let mut resolution = run(&situation, &plan, "I roll Insight on Nella's claim.", 18);
    let additional = refine_sense(
        &voice,
        Some(&mut context),
        &mut resolution,
        "I roll Insight on Nella's claim.",
        18,
        fair(0),
        || panic!("one part draws one die"),
    )
    .await;
    assert!(additional.is_empty());
    assert_eq!(
        performance(&resolution).check.as_ref().unwrap().label,
        "Wisdom (Insight)"
    );
    assert!(performance(&resolution).succeeded);
    assert!(fallback_line(&resolution).contains("Nella's claim reads as honest"));
    assert!(lore.development_decisions().is_empty());
    assert!(endpoint.bodies().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn hs67_freeform_action_rejected_failure_description_cannot_leak_recorded_freshness() {
    let (mut situation, _, _) = hs67_bread();
    situation
        .things
        .iter_mut()
        .find(|thing| thing.id == "bread")
        .unwrap()
        .detail = HS67_BREAD_FACT.into();
    let plan = FreeformPlan {
        sense: Some(Sense::Smell),
        ..plan(
            ActKind::Sense,
            Some(target(&situation, "bread")),
            Difficulty::Easy,
        )
    };
    let endpoint = FakeEndpoint::start(vec![Reply::Output(
        json!({"impression":"The bread was baked this morning."}),
    )]);
    let jev = FakeEndpoint::start(vec![answer("o2"), answer("o2")]);
    let (voice, _) = hs67_voice(&endpoint, &jev, "rejected-impression");
    let mut resolution = run(
        &situation,
        &plan,
        "I roll Perception to smell the bread.",
        1,
    );
    let additional = refine_sense(
        &voice,
        None,
        &mut resolution,
        "I roll Perception to smell the bread.",
        1,
        fair(0),
        || panic!("only one requested part"),
    )
    .await;
    assert!(additional.is_empty());
    let line = fallback_line(&resolution);
    assert!(
        !line.contains("morning") && !line.contains("nothing new"),
        "{line}"
    );
    assert!(line.contains("plainly visible"));
    assert!(
        !narration_facts(&resolution, "")
            .to_string()
            .contains("morning")
    );
}

#[test]
fn hs67_freeform_action_both_wisdom_checks_use_the_authoritative_hero_sheet() {
    let current = crate::gameplay::beat_script_tests::prepared_story();
    let state = current.state();
    let hero = state.characters.first().unwrap().entity;
    // Stalwart's Wisdom 12 is +1; the fixture trained Perception (+2), but not Insight.
    assert_eq!(journey::value(state, hero, "wisdom").unwrap(), 12);
    for (skill, modifier) in [("perception", 3), ("insight", 1)] {
        let result = skill_roll(state, hero, skill, 16, 10).unwrap();
        assert_eq!(result.total, 16 + modifier);
        assert_eq!(result.modifier, modifier);
        assert!(result.succeeded);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn hs67_freeform_action_maximum_sensory_slots_preserve_honesty_in_the_published_line() {
    let (situation, plan, mut context) = hs67_bread();
    let mut replies = hs67_backfill_replies();
    replies.push(Reply::Compute(std::sync::Arc::new(|prompt| {
        let slots: Vec<Value> = serde_json::from_str(
            prompt
                .lines()
                .find_map(|line| line.strip_prefix("SLOTS: "))
                .unwrap(),
        )
        .unwrap();
        let mut response = json!({});
        for slot in slots {
            let maximum = slot["max_characters"].as_u64().unwrap() as usize;
            // Synthetic long sentences exercise the full accepted publication budget.
            response[slot["name"].as_str().unwrap()] =
                json!(format!("{}.", "a".repeat(maximum - 1)));
        }
        response
    })));
    let endpoint = FakeEndpoint::start(replies);
    let jev = FakeEndpoint::start(vec![
        answer("o7"),
        answer("o2"),
        answer("o1"),
        answer("o1"),
        answer("o1"),
        answer("o1"),
    ]);
    let (voice, _) = hs67_voice(&endpoint, &jev, "maximum-publication");
    let words = "I roll Insight to judge the bread's freshness and Nella's honesty.";
    let mut resolution = run(&situation, &plan, words, 16);
    let additional = refine_sense(
        &voice,
        Some(&mut context),
        &mut resolution,
        words,
        16,
        fair(3),
        || Ok(12),
    )
    .await;
    let line = narrate(voice.hero_identity_models().0, &resolution, words).await;
    assert!(line.chars().count() <= NARRATION_CHARACTERS);
    assert!(line.ends_with("Nella's claim reads as honest."), "{line}");
    let log = TableLog::new();
    publish_compound_outcome(
        &log,
        0,
        performance(&resolution)
            .check
            .as_ref()
            .map(CheckRecord::row),
        &additional,
        &line,
    );
    let lines = log.lines();
    assert_eq!(lines.last().unwrap().text, line);
    assert!(
        lines
            .last()
            .unwrap()
            .text
            .ends_with("Nella's claim reads as honest.")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn hs67_freeform_action_unavailable_check_judge_preserves_the_visible_thing() {
    let (situation, plan, _) = hs67_bread();
    let mut resolution = run(&situation, &plan, "I smell the bread.", 12);
    let additional = refine_sense(
        &crate::gameplay::npc_reply::NpcVoice::silent(),
        None,
        &mut resolution,
        "I smell the bread.",
        12,
        |_, _, _| panic!("an unavailable judge cannot choose a check"),
        || panic!("an unavailable judge cannot draw a check die"),
    )
    .await;
    assert!(additional.is_empty());
    assert!(performance(&resolution).check.is_none());
    let line = narrate(&Storyteller::disabled(), &resolution, "I smell the bread.").await;
    assert!(
        line.contains("loaf of bread") && line.contains("remains in view"),
        "{line}"
    );
    assert!(line.contains("check ruling is unavailable"), "{line}");
    assert!(!line.contains("nothing new"));
}

fn thing_fact(id: &str, name: &str, detail: &str, portable: bool, held: bool) -> ThingFact {
    ThingFact {
        id: id.to_owned(),
        name: name.to_owned(),
        detail: detail.to_owned(),
        portable,
        held_by_hero: held,
        holder_name: None,
        owner_id: None,
    }
}

/// A shop scene with Marek carrying a greatsword, chain mail, javelins and rations, six gold
/// pieces in the purse, and Corin (passive Perception 12) present.
fn shop() -> Situation {
    Situation {
        hero: HeroSheet {
            name: "Marek".to_owned(),
            strength_modifier: 3,
            proficiency_bonus: 2,
            known_spells: Vec::new(),
            pronouns: None,
            appearance: None,
        },
        things: vec![
            thing_fact(
                "counter",
                "counter",
                "A long pine counter, scarred by use.",
                false,
                false,
            ),
            thing_fact(
                "candle",
                "candle",
                "A tallow candle in a tin holder.",
                true,
                false,
            ),
            thing_fact(
                "pack-greatsword",
                "greatsword",
                "Part of your starting kit: greatsword.",
                true,
                true,
            ),
            thing_fact(
                "pack-javelin",
                "javelin",
                "Part of your starting kit: javelin.",
                true,
                true,
            ),
            thing_fact(
                "pack-chain-mail",
                "chain mail",
                "Part of your starting kit: chain mail.",
                true,
                true,
            ),
            thing_fact(
                "pack-rations",
                "rations",
                "Part of your starting kit: rations.",
                true,
                true,
            ),
            thing_fact(
                "clothing-rack",
                "clothing rack",
                "A rack of wool coats.",
                false,
                false,
            ),
        ],
        people: vec![PersonFact {
            id: "corin".to_owned(),
            name: "Corin".to_owned(),
            role: rpc::SceneRole::Contact,
            passive_perception: 12,
        }],
        other_heroes: vec!["Sigrun".to_owned()],
        place: "The market shop smells of dyed wool and beeswax.".to_owned(),
        coin_gp: 6,
    }
}

fn target(situation: &Situation, id: &str) -> Target {
    situation
        .options()
        .into_iter()
        .find(|option| option.id == id)
        .unwrap_or_else(|| panic!("no listed target {id}"))
}

fn plan(act: ActKind, target: Option<Target>, difficulty: Difficulty) -> FreeformPlan {
    FreeformPlan {
        act,
        sense: None,
        target,
        recipient: None,
        difficulty,
        means: StrikeMeans::Unarmed,
        spell: None,
    }
}

/// A fair resolver: the die plus the modifier, against the DC.
fn fair(modifier: i32) -> impl Fn(&str, u8, u8) -> Option<SkillRoll> {
    move |_skill, die, dc| {
        let total = i32::from(die) + modifier;
        Some(SkillRoll {
            modifier,
            total,
            succeeded: total >= i32::from(dc),
        })
    }
}

fn run(situation: &Situation, plan: &FreeformPlan, _words: &str, die: u8) -> Resolution {
    adjudicate(situation, plan, die, fair(3))
}

#[tokio::test(flavor = "current_thread")]
async fn harmless_counter_rap_is_judged_as_attention_contact() {
    use crate::gameplay::approach_model::tests::{answer, model};

    let endpoint = FakeEndpoint::start(vec![answer("o12"), answer("o10"), answer("o1")]);
    let mut situation = shop();
    situation.people.push(PersonFact {
        id: "marek".to_owned(),
        name: "Marek".to_owned(),
        role: rpc::SceneRole::Contact,
        passive_perception: 10,
    });
    let judged = super::plan(
        &model(&endpoint),
        "I rap my knuckles on the counter to catch Marek's eye.",
        &situation,
    )
    .await
    .unwrap();

    assert_eq!(judged.act, ActKind::ContactFriendly);
    assert_eq!(
        judged.target.as_ref().map(|target| target.name.as_str()),
        Some("Marek")
    );
    let choices = endpoint.bodies();
    let act_options = choices[0]["questions"]["q1"]["criteria"]
        .as_object()
        .unwrap();
    assert!(act_options.values().any(|choice| {
        choice
            .as_str()
            .unwrap()
            .contains("light rap of knuckles on a counter")
    }));
    assert!(
        act_options
            .values()
            .any(|choice| { choice.as_str().unwrap().contains("never an assault") })
    );
}

#[tokio::test(flavor = "current_thread")]
async fn jev_keeps_an_unarmed_assault_out_of_the_present_npc_dialogue_route() {
    use crate::gameplay::approach_model::tests::{answer, model};

    let situation = shop();
    let strike_choice = ACT_KINDS
        .iter()
        .position(|(act, _, _)| *act == ActKind::Strike)
        .expect("strike is a listed act")
        + 1;
    let corin_choice = situation
        .options()
        .iter()
        .position(|target| target.name == "Corin")
        .expect("Corin is present")
        + 1;
    let endpoint = FakeEndpoint::start(vec![
        answer(&format!("o{strike_choice}")),
        answer(&format!("o{corin_choice}")),
        answer("o1"), // No additional difficulty.
        answer("o1"), // Unarmed.
    ]);
    let judged = super::plan(
        &model(&endpoint),
        "That's it. I punch Corin square in the jaw!",
        &situation,
    )
    .await
    .expect("Jev judges the typed line");

    assert_eq!(judged.act, ActKind::Strike);
    assert_eq!(
        judged.target.as_ref().map(|target| target.name.as_str()),
        Some("Corin")
    );
    assert!(assaults_person(&judged));
}

#[tokio::test(flavor = "current_thread")]
async fn jev_routes_non_hostile_contact_to_the_present_npc_dialogue_turn() {
    use crate::gameplay::approach_model::tests::{answer, model};

    let endpoint = FakeEndpoint::start(vec![
        answer("o12"), // Harmless contact.
        answer("o6"),  // Corin.
        answer("o1"),  // No additional difficulty.
    ]);
    let situation = shop();
    let judged = super::plan(
        &model(&endpoint),
        "I laugh and clap Corin on the back. Cheer up, friend.",
        &situation,
    )
    .await
    .expect("Jev judges the typed line");

    assert!(!assaults_person(&judged));
}

fn performance(resolution: &Resolution) -> &Performance {
    match &resolution.verdict {
        Verdict::Performed(performance) => performance,
        other => panic!("expected a performed act, got {other:?}"),
    }
}

#[test]
fn each_act_kind_routes_and_narrates_an_answer_for_the_hero() {
    let situation = shop();
    for (kind, label, _) in ACT_KINDS {
        let target = situation
            .options()
            .into_iter()
            .find(|option| accepts(kind, option.kind));
        let sense = (kind == ActKind::Sense).then_some(Sense::Sight);
        let plan = FreeformPlan {
            sense,
            ..plan(kind, target, Difficulty::None)
        };
        let resolution = run(&situation, &plan, "I do the thing.", 12);
        assert_ne!(resolution.verdict, Verdict::Unjudged, "{label}");
        let line = fallback_line(&resolution);
        assert!(line.ends_with('.'), "{label}: {line}");
        assert!(!line.is_empty(), "{label}");
    }
}

#[test]
fn a_missing_target_says_so_in_the_world() {
    let situation = shop();
    let resolution = run(
        &situation,
        &plan(ActKind::Light, None, Difficulty::None),
        "I light a candle on the table.",
        12,
    );
    assert_eq!(resolution.verdict, Verdict::Refused(Refusal::MissingTarget));
    assert_eq!(
        fallback_line(&resolution),
        "Marek finds nothing like that here to light."
    );
}

#[test]
fn a_fixed_thing_cannot_be_thrown() {
    let situation = shop();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Throw,
            Some(target(&situation, "counter")),
            Difficulty::Easy,
        ),
        "I throw the counter.",
        12,
    );
    assert_eq!(resolution.verdict, Verdict::Refused(Refusal::Fixed));
    assert_eq!(
        fallback_line(&resolution),
        "The counter is fixed in place and can't be thrown."
    );
}

#[test]
fn a_fighter_cannot_cast_light() {
    let situation = shop();
    let resolution = run(
        &situation,
        &FreeformPlan {
            spell: Some("Light".to_owned()),
            ..plan(ActKind::Cast, None, Difficulty::None)
        },
        "I cast Light at the candle.",
        12,
    );
    assert_eq!(
        resolution.verdict,
        Verdict::Refused(Refusal::UnknownSpell {
            spell: "Light".to_owned()
        })
    );
    assert_eq!(
        fallback_line(&resolution),
        "Marek doesn't know the Light spell."
    );
}

#[test]
fn a_known_spell_is_cast_only_when_the_sheet_names_it() {
    let mut situation = shop();
    situation.hero.known_spells = vec!["Prestidigitation".to_owned()];
    let resolution = run(
        &situation,
        &FreeformPlan {
            spell: Some("Prestidigitation".to_owned()),
            ..plan(ActKind::Cast, None, Difficulty::None)
        },
        "I cast Prestidigitation to clean the counter.",
        12,
    );
    assert_eq!(performance(&resolution).succeeded, true);
    assert_eq!(resolution.spell.as_deref(), Some("Prestidigitation"));
}

#[test]
fn a_shove_rolls_the_saving_throw_at_dc_eight_plus_strength_and_proficiency() {
    let situation = shop();
    // DC 8 + Strength modifier 3 + Proficiency Bonus 2 = 13.
    let plan = plan(
        ActKind::ContactRough,
        Some(target(&situation, "corin")),
        Difficulty::None,
    );
    let failed = run(&situation, &plan, "I shove Corin out of my way.", 4);
    let record = performance(&failed)
        .check
        .clone()
        .expect("a save is rolled");
    assert_eq!(record.number, 13);
    assert_eq!(record.against, "DC");
    assert_eq!(record.label, "Strength save");
    assert_eq!(record.total, 4);
    assert!(!record.success, "a 4 does not meet DC 13");
    assert!(
        performance(&failed).succeeded,
        "Corin fails the save and stumbles"
    );

    let resisted = run(&situation, &plan, "I shove Corin out of my way.", 15);
    assert!(!performance(&resisted).succeeded);
    assert!(
        performance(&resisted)
            .check
            .as_ref()
            .is_some_and(|record| record.success)
    );
}

#[test]
fn a_punch_rolls_an_attack_against_armor_class_ten() {
    let situation = shop();
    let plan = plan(
        ActKind::Strike,
        Some(target(&situation, "corin")),
        Difficulty::None,
    );
    // Die 4 + Strength 3 + Proficiency 2 = 9, short of AC 10.
    let miss = run(&situation, &plan, "I punch Corin in the jaw.", 4);
    let record = performance(&miss)
        .check
        .clone()
        .expect("an attack is rolled");
    assert_eq!(record.against, "AC");
    assert_eq!(record.number, 10);
    assert_eq!(record.total, 9);
    assert!(!record.success);
    // Die 5 makes exactly 10: a hit.
    let hit = run(&situation, &plan, "I punch Corin in the jaw.", 5);
    assert!(performance(&hit).succeeded);
    assert_eq!(
        fallback_line(&hit),
        "Marek punches Corin. Corin takes it badly."
    );
}

#[test]
fn a_weapon_strike_starts_no_combat_and_records_hostile_contact() {
    let situation = shop();
    let plan = FreeformPlan {
        means: StrikeMeans::Weapon,
        ..plan(
            ActKind::Strike,
            Some(target(&situation, "corin")),
            Difficulty::None,
        )
    };
    let resolution = run(&situation, &plan, "I swing my greatsword at Corin.", 12);
    assert!(performance(&resolution).check.is_none(), "no attack roll");
    assert_eq!(resolution.consequences.len(), 1);
    assert_eq!(resolution.consequences[0].kind, ConsequenceKind::Offence);
    assert!(fallback_line(&resolution).starts_with("Marek squares up to Corin"));
}

#[test]
fn rough_contact_records_a_grievance_only_for_the_hero_who_made_it() {
    let dossier = crate::gameplay::npc_dossier_tests::adventure(1)
        .dossiers
        .into_iter()
        .find(|dossier| dossier.role == crate::gameplay::adventure::dossier::NpcRole::Patron)
        .expect("a patron dossier");
    let (_, person_id) = dossier.npc_id.split_once(':').expect("role:person id");
    let ledger = crate::gameplay::social::SocialLedger::open(vec![dossier.clone()], "0x01", None)
        .expect("ledger");
    let shover = MemberId::from_bytes(&[11; 16]).unwrap();
    let bystander = MemberId::from_bytes(&[12; 16]).unwrap();
    let consequence = Consequence {
        person_id: person_id.to_owned(),
        person_name: "Corin".to_owned(),
        role: rpc::SceneRole::Patron,
        kind: ConsequenceKind::Offence,
    };

    let recorded = apply_consequences(
        &ledger,
        shover,
        "opening",
        "I shove Corin out of my way.",
        "Corin stumbles back.",
        std::slice::from_ref(&consequence),
    );

    assert!(recorded);
    assert!(
        !ledger.grievance_lines(shover, &dossier).is_empty(),
        "the hero who shoved keeps the grievance"
    );
    assert!(
        ledger.grievance_lines(bystander, &dossier).is_empty(),
        "another hero keeps nothing"
    );
}

#[test]
fn a_drawn_weapon_makes_present_npcs_wary_without_a_lasting_grievance() {
    let situation = shop();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Draw,
            Some(target(&situation, "pack-greatsword")),
            Difficulty::None,
        ),
        "I draw my greatsword.",
        12,
    );
    assert_eq!(resolution.consequences.len(), 1);
    assert_eq!(resolution.consequences[0].kind, ConsequenceKind::Wary);
    assert!(fallback_line(&resolution).contains("grows wary"));
}

#[test]
fn a_smell_answer_stays_within_the_scene_facts() {
    let situation = shop();
    let plan = FreeformPlan {
        sense: Some(Sense::Smell),
        ..plan(
            ActKind::Sense,
            Some(target(&situation, "place")),
            Difficulty::None,
        )
    };
    let resolution = run(&situation, &plan, "What does the shop smell like?", 12);
    assert_eq!(
        performance(&resolution).revealed.as_deref(),
        Some("The market shop smells of dyed wool and beeswax.")
    );
    assert_eq!(
        fallback_line(&resolution),
        "Marek smells the room. The market shop smells of dyed wool and beeswax."
    );
    assert!(resolution.consequences.is_empty());
}

#[test]
fn a_taste_answer_on_a_thing_reveals_only_that_things_scene_detail() {
    let situation = shop();
    let plan = FreeformPlan {
        sense: Some(Sense::Taste),
        ..plan(
            ActKind::Sense,
            Some(target(&situation, "candle")),
            Difficulty::None,
        )
    };
    let resolution = run(&situation, &plan, "I taste the candle.", 12);
    assert_eq!(
        performance(&resolution).revealed.as_deref(),
        Some("A tallow candle in a tin holder.")
    );
    assert!(!fallback_line(&resolution).contains("beeswax"));
}

#[test]
fn checking_my_own_gear_and_counting_coin_use_the_real_inventory() {
    let situation = shop();
    let coin = FreeformPlan {
        sense: Some(Sense::Sight),
        ..plan(
            ActKind::Sense,
            Some(target(&situation, "coin")),
            Difficulty::None,
        )
    };
    let counted = run(&situation, &coin, "I count the coins in my purse.", 12);
    assert_eq!(
        performance(&counted).revealed.as_deref(),
        Some("Marek's purse holds 6 gold pieces.")
    );
    let line = fallback_line(&counted);
    assert_eq!(
        line,
        "Marek looks at Marek's coin purse. Marek's purse holds 6 gold pieces."
    );
    assert!(!line.to_lowercase().contains("your"), "{line}");
    let edge = FreeformPlan {
        sense: Some(Sense::Sight),
        ..plan(
            ActKind::Sense,
            Some(target(&situation, "pack-greatsword")),
            Difficulty::None,
        )
    };
    let checked = run(&situation, &edge, "I check my greatsword's edge.", 12);
    assert!(fallback_line(&checked).contains("greatsword"));
}

#[test]
fn a_consumable_must_be_in_the_heroes_inventory() {
    let mut situation = shop();
    situation.things.push(thing_fact(
        "apple",
        "apple",
        "A red apple on the counter.",
        true,
        false,
    ));
    let loose = run(
        &situation,
        &plan(
            ActKind::Consume,
            Some(target(&situation, "apple")),
            Difficulty::None,
        ),
        "I eat an apple from my pack.",
        12,
    );
    assert_eq!(loose.verdict, Verdict::Refused(Refusal::NoItem));
    assert_eq!(
        fallback_line(&loose),
        "Marek doesn't have the apple to eat or drink."
    );
    let carried = run(
        &situation,
        &plan(
            ActKind::Consume,
            Some(target(&situation, "pack-rations")),
            Difficulty::None,
        ),
        "I eat some rations.",
        12,
    );
    assert_eq!(performance(&carried).succeeded, true);
}

#[test]
fn juggling_a_portable_weapon_takes_an_acrobatics_check() {
    let situation = shop();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Perform,
            Some(target(&situation, "pack-javelin")),
            Difficulty::Easy,
        ),
        "I juggle my javelins.",
        12,
    );
    let record = performance(&resolution).check.clone().expect("a check");
    assert_eq!(record.label, "Dexterity (Acrobatics)");
    assert_eq!(record.number, 10);
}

#[test]
fn showing_an_item_puts_it_in_front_of_the_recipient_without_a_transfer() {
    let situation = shop();
    let plan = FreeformPlan {
        recipient: Some(target(&situation, "corin")),
        ..plan(
            ActKind::Show,
            Some(target(&situation, "pack-chain-mail")),
            Difficulty::None,
        )
    };
    let resolution = run(
        &situation,
        &plan,
        "I show Corin my chain mail and ask about the stitching.",
        12,
    );
    assert_eq!(resolution.notices, vec!["Corin".to_owned()]);
    assert!(resolution.consequences.is_empty());
    assert!(
        fallback_line(&resolution)
            .starts_with("Marek shows the chain mail to Corin, who looks it over.")
    );
}

#[test]
fn hiding_rolls_stealth_against_the_passive_perception_of_who_could_see() {
    let situation = shop();
    // Corin's passive Perception is 12, so Stealth must reach 12. Die 9 + 3 = 12 is hidden.
    let plan = plan(
        ActKind::Hide,
        Some(target(&situation, "clothing-rack")),
        Difficulty::None,
    );
    let hidden = run(&situation, &plan, "I duck behind the clothing rack.", 9);
    let record = performance(&hidden).check.clone().expect("a stealth check");
    assert_eq!(record.label, "Dexterity (Stealth)");
    assert_eq!(record.number, 12);
    assert!(performance(&hidden).succeeded);
    assert_eq!(
        narration_facts(&hidden, "I duck behind the clothing rack.")["hidden"],
        true
    );
    assert_eq!(
        fallback_line(&hidden),
        "Marek ducks behind the clothing rack and keeps still and goes unnoticed."
    );
    let seen = run(&situation, &plan, "I duck behind the clothing rack.", 8);
    assert!(!performance(&seen).succeeded);
}

#[test]
fn climbing_onto_a_counter_takes_an_athletics_check_only_when_uncertain() {
    let situation = shop();
    let plan = plan(
        ActKind::Climb,
        Some(target(&situation, "counter")),
        Difficulty::None,
    );
    let climbed = run(&situation, &plan, "I climb onto the counter.", 2);
    assert!(performance(&climbed).check.is_none());
    assert_eq!(fallback_line(&climbed), "Marek climbs onto the counter.");
    assert_eq!(
        narration_facts(&climbed, "I climb onto the counter.")["position"],
        "counter"
    );
}

#[test]
fn a_note_left_in_the_scene_is_noticed_without_a_ledger_consequence() {
    let situation = shop();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Write,
            Some(target(&situation, "counter")),
            Difficulty::None,
        ),
        "I write a note thanking Marek and leave it on the counter.",
        12,
    );
    assert_eq!(resolution.notices, vec!["Corin".to_owned()]);
    assert!(resolution.consequences.is_empty());
    assert!(fallback_line(&resolution).ends_with("Corin notices it."));
}

#[test]
fn a_hero_line_is_published_before_the_roll_and_the_narration() {
    let log = TableLog::new();
    publish_hero_line(&log, "Marek", 0, "I smash the fitting mirror with my fist.");
    publish_outcome(
        &log,
        0,
        Some("Marek: Strength (unarmed strike) 15 + 5 = 20 vs AC 10 — hit."),
        "Marek smashes the mirror.",
    );
    let lines = log.lines();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].speaker, "Marek");
    assert_eq!(lines[0].role, rpc::TableRole::Hero as i32);
    assert_eq!(lines[0].text, "I smash the fitting mirror with my fist.");
    assert_eq!(lines[1].speaker, "Roll");
    assert_eq!(lines[2].speaker, "Narrator");
    assert!(lines[0].sequence < lines[1].sequence && lines[1].sequence < lines[2].sequence);
}

#[tokio::test(flavor = "current_thread")]
async fn jev_plan_asks_closed_questions_in_order_and_takes_each_answer_as_a_position() {
    // Act sense (o1), sense smell (o3), target the room (o11 of 13), difficulty none (o1).
    let endpoint = FakeEndpoint::start(vec![
        answer("o1"),
        answer("o3"),
        answer("o11"),
        answer("o1"),
    ]);
    let model = ApproachModel::configured(Some(ApproachSettings {
        credential: "jev-fixture-sentinel".to_owned(),
        base_url: format!("http://{}", endpoint.address),
    }));
    let situation = shop();
    let plan = super::plan(&model, "What does the shop smell like?", &situation)
        .await
        .expect("jev plan");
    assert_eq!(plan.act, ActKind::Sense);
    assert_eq!(plan.sense, Some(Sense::Smell));
    assert_eq!(
        plan.target.map(|target| target.id),
        Some("place".to_owned())
    );
    assert_eq!(plan.difficulty, Difficulty::None);
    assert_eq!(endpoint.connections(), 4);
    let bodies = endpoint.bodies();
    assert_eq!(
        bodies[2]["questions"]["q1"]["criteria"]
            .as_object()
            .unwrap()
            .len(),
        13
    );
}

#[tokio::test(flavor = "current_thread")]
async fn jev_names_the_known_spell_last_and_none_names_no_spell() {
    // Cast is the eleventh act. Plan asks the act, then the target, the difficulty and the spell.
    let cast = ACT_KINDS
        .iter()
        .position(|(kind, _, _)| *kind == ActKind::Cast)
        .unwrap()
        + 1;
    let mut situation = shop();
    situation.hero.known_spells = vec!["Mage Hand".to_owned()];
    let named = FakeEndpoint::start(vec![
        answer(&format!("o{cast}")),
        answer("o1"),
        answer("o1"),
        answer("o1"),
    ]);
    let model = ApproachModel::configured(Some(ApproachSettings {
        credential: "jev-fixture-sentinel".to_owned(),
        base_url: format!("http://{}", named.address),
    }));
    let plan = super::plan(&model, "I cast Mage Hand on the candle.", &situation)
        .await
        .expect("jev plan");
    assert_eq!(plan.act, ActKind::Cast);
    assert_eq!(plan.spell.as_deref(), Some("Mage Hand"));
    assert_eq!(named.connections(), 4);
    assert_eq!(
        named.bodies()[3]["questions"]["q1"]["criteria"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    let resolution = run(&situation, &plan, "I cast Mage Hand on the candle.", 12);
    assert_eq!(performance(&resolution).succeeded, true);
    assert_eq!(resolution.spell.as_deref(), Some("Mage Hand"));

    let none = FakeEndpoint::start(vec![
        answer(&format!("o{cast}")),
        answer("o1"),
        answer("o1"),
        answer("o2"),
    ]);
    let model = ApproachModel::configured(Some(ApproachSettings {
        credential: "jev-fixture-sentinel".to_owned(),
        base_url: format!("http://{}", none.address),
    }));
    let plan = super::plan(&model, "I cast something on the candle.", &situation)
        .await
        .expect("jev plan");
    assert_eq!(plan.spell, None);
    let resolution = run(&situation, &plan, "I cast something on the candle.", 12);
    assert_eq!(resolution.verdict, Verdict::Refused(Refusal::NoSpell));
}

fn answer(option: &str) -> Reply {
    Reply::Status(
        200,
        json!({
            "answers": {"q1": {"type": "choice", "choice": option, "confidence": 0.99}},
            "model": "jev-1.13.0",
            "usage": {"input_tokens": 10, "output_tokens": 1}
        }),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn without_jev_a_freeform_act_is_unjudged_and_never_silently_succeeds() {
    let resolution = resolve(
        &ApproachModel::default(),
        &shop(),
        "I sit on the edge of the counter.",
        12,
        fair(3),
    )
    .await;
    assert_eq!(resolution.verdict, Verdict::Unjudged);
    assert_eq!(
        fallback_line(&resolution),
        "Marek tries it, but how it turns out isn't clear yet."
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_storyteller_narration_that_validates_is_used_and_names_the_hero() {
    let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({
        "narration": "Marek sits on the edge of the counter."
    }))]);
    let cache = cache_directory("freeform-narration-valid");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let resolution = run(
        &shop(),
        &plan(
            ActKind::Lean,
            Some(target(&shop(), "counter")),
            Difficulty::None,
        ),
        "I sit on the edge of the counter.",
        12,
    );
    let line = narrate(
        &storyteller,
        &resolution,
        "I sit on the edge of the counter.",
    )
    .await;
    assert_eq!(line, "Marek sits on the edge of the counter.");
}

#[tokio::test(flavor = "current_thread")]
async fn the_narration_asks_for_no_hidden_reasoning_so_the_cap_is_for_the_answer() {
    let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({
        "narration": "Marek sits on the edge of the counter."
    }))]);
    let cache = cache_directory("freeform-narration-profile");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let resolution = run(
        &shop(),
        &plan(
            ActKind::Lean,
            Some(target(&shop(), "counter")),
            Difficulty::None,
        ),
        "I sit on the edge of the counter.",
        12,
    );
    narrate(
        &storyteller,
        &resolution,
        "I sit on the edge of the counter.",
    )
    .await;
    let bodies = endpoint.bodies();
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["reasoning"], json!({"effort": "none"}));
    assert_eq!(bodies[0]["max_output_tokens"], 220);
    assert!(bodies[0]["text"].to_string().find("hero_words").is_none());
    assert!(
        bodies[0]["text"]
            .to_string()
            .find("I sit on the edge of the counter.")
            .is_none()
    );
    std::fs::remove_dir_all(cache).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn a_narration_cut_off_at_its_cap_falls_back_to_the_rules_sentence() {
    let incomplete = |sent: &Value| {
        (
            200,
            json!({"object":"response","id":"resp_fixture","status":"incomplete","model":sent["model"],
                "metadata":sent["metadata"],"text":sent["text"],"error":null,
                "incomplete_details":{"reason":"max_output_tokens"},"output":[],
                "usage":{"input_tokens":100,"output_tokens":120,"total_tokens":220,
                    "input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":100}}}),
        )
    };
    let endpoint = FakeEndpoint::start(vec![Reply::ComputeResponse(std::sync::Arc::new(
        incomplete,
    ))]);
    let cache = cache_directory("freeform-narration-cut-off");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let resolution = run(
        &shop(),
        &plan(
            ActKind::Lean,
            Some(target(&shop(), "counter")),
            Difficulty::None,
        ),
        "I sit on the edge of the counter.",
        12,
    );
    let line = narrate(
        &storyteller,
        &resolution,
        "I sit on the edge of the counter.",
    )
    .await;
    assert_eq!(line, fallback_line(&resolution));
    let _ = std::fs::remove_dir_all(cache);
}

#[tokio::test(flavor = "current_thread")]
async fn with_the_storyteller_off_the_deterministic_sentence_is_used() {
    let resolution = run(
        &shop(),
        &plan(
            ActKind::Lean,
            Some(target(&shop(), "counter")),
            Difficulty::None,
        ),
        "I sit on the edge of the counter.",
        12,
    );
    let line = narrate(
        &crate::gameplay::storyteller::Storyteller::disabled(),
        &resolution,
        "I sit on the edge of the counter.",
    )
    .await;
    assert_eq!(line, "Marek sits or leans on the counter.");
}

/// The live probe's street: Sigrun (she/her) with her own look on the sheet, Marek (who owns the
/// folded label), Corin and Brakka present, and carts and a gull outside the shop door.
fn market_street() -> Situation {
    let mut situation = shop();
    situation.hero = HeroSheet {
        name: "Sigrun".to_owned(),
        strength_modifier: 1,
        proficiency_bonus: 2,
        known_spells: Vec::new(),
        pronouns: Some(Pronouns {
            subject: "she",
            object: "her",
            possessive: "her",
            reflexive: "herself",
        }),
        appearance: Some(
            "An adult dwarf, middle aged adult, broad build, pale skin, braided auburn hair, hazel eyes, chainmail pauldron."
                .to_owned(),
        ),
    };
    situation.things = vec![
        thing_fact(
            "counter",
            "counter",
            "A long pine counter, scarred by use.",
            false,
            false,
        ),
        ThingFact {
            owner_id: Some("marek".to_owned()),
            ..thing_fact(
                "label",
                "label",
                "A folded paper label with Marek's mark.",
                true,
                false,
            )
        },
    ];
    situation.people = ["marek", "corin", "brakka"]
        .into_iter()
        .map(|id| PersonFact {
            id: id.to_owned(),
            name: id[..1].to_uppercase() + &id[1..],
            role: rpc::SceneRole::Contact,
            passive_perception: 12,
        })
        .collect();
    situation.other_heroes = Vec::new();
    situation.place =
        "Carts rattle past on the market street, and a gull calls over the shop door.".to_owned();
    situation
}

/// Marks a listed thing as carried by the hero.
fn hold(situation: &mut Situation, id: &str) {
    let thing = situation
        .things
        .iter_mut()
        .find(|thing| thing.id == id)
        .expect("a listed thing");
    thing.held_by_hero = true;
    thing.holder_name = None;
}

#[test]
fn a_door_opened_or_closed_from_a_freeform_reading_keeps_the_scene_interaction() {
    assert!(keeps_scene_interaction("open"));
    assert!(keeps_scene_interaction("close"));
    for option in ["give", "take", "drop", "examine", "none"] {
        assert!(!keeps_scene_interaction(option), "{option}");
    }
}

#[test]
fn a_give_of_an_unlisted_kit_item_is_a_freeform_hand_over_and_a_listed_give_is_not() {
    // Sigrun hands Brakka her waterskin: the scene model lists no such thing, so no scene give takes it.
    assert!(is_unlisted_hand_over("give", false));
    assert!(!is_unlisted_hand_over("give", true));
    for option in ["gift", "take", "open", "examine", "none"] {
        assert!(!is_unlisted_hand_over(option, false), "{option}");
    }
}

#[test]
fn a_give_is_adjudicated_as_a_hand_over_to_the_judged_recipient() {
    let mut situation = market_street();
    situation.things.push(thing_fact(
        "waterskin",
        "waterskin",
        "Part of the dungeoneer pack.",
        true,
        true,
    ));
    let plan = FreeformPlan {
        recipient: Some(target(&situation, "brakka")),
        ..plan(
            ActKind::Give,
            Some(target(&situation, "waterskin")),
            Difficulty::None,
        )
    };
    let resolution = run(
        &situation,
        &plan,
        "I hand Brakka my waterskin. Drink, you look parched.",
        12,
    );
    assert!(matches!(resolution.verdict, Verdict::Performed(_)));
    assert_eq!(resolution.recipient.as_deref(), Some("Brakka"));
    assert_eq!(
        fallback_line(&resolution),
        "Sigrun hands the waterskin to Brakka."
    );
    let facts = narration_facts(
        &resolution,
        "I hand Brakka my waterskin. Drink, you look parched.",
    );
    assert_eq!(facts["recipient"], "Brakka");
    assert!(facts.get("hero_words").is_none());
}

#[test]
fn a_give_requires_a_held_item_and_a_recipient() {
    let situation = market_street();
    let plan = plan(
        ActKind::Give,
        Some(target(&situation, "label")),
        Difficulty::None,
    );
    let resolution = run(&situation, &plan, "I give Marek's label to Brakka.", 12);
    assert_eq!(
        resolution.verdict,
        Verdict::Refused(Refusal::NotHeld { holder: None })
    );
}

#[test]
fn sitting_on_the_edge_of_the_counter_is_a_position_that_moves_nothing_and_offends_nobody() {
    let situation = market_street();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Lean,
            Some(target(&situation, "counter")),
            Difficulty::None,
        ),
        "I sit on the edge of the counter.",
        12,
    );
    assert_eq!(resolution.act, Some(ActKind::Lean));
    assert!(resolution.consequences.is_empty());
    assert_eq!(
        fallback_line(&resolution),
        "Sigrun sits or leans on the counter."
    );
}

#[test]
fn laughing_at_brakkas_joke_is_an_expressive_reaction_and_never_a_sense_answer() {
    let situation = market_street();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Express,
            Some(target(&situation, "brakka")),
            Difficulty::None,
        ),
        "I laugh at Brakka's joke.",
        12,
    );
    assert_eq!(resolution.act, Some(ActKind::Express));
    assert!(performance(&resolution).check.is_none());
    assert_eq!(
        fallback_line(&resolution),
        "Sigrun reacts to Brakka out loud."
    );
}

#[test]
fn dropping_a_label_sigrun_does_not_hold_is_refused_in_the_world() {
    let situation = market_street();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Drop,
            Some(target(&situation, "label")),
            Difficulty::None,
        ),
        "I drop the folded label on the floor.",
        12,
    );
    assert_eq!(
        resolution.verdict,
        Verdict::Refused(Refusal::NotHeld { holder: None })
    );
    assert_eq!(
        fallback_line(&resolution),
        "Sigrun isn't holding the label."
    );
}

#[test]
fn dropping_a_label_sigrun_holds_sets_it_down() {
    let mut situation = market_street();
    hold(&mut situation, "label");
    let resolution = run(
        &situation,
        &plan(
            ActKind::Drop,
            Some(target(&situation, "label")),
            Difficulty::None,
        ),
        "I drop the label.",
        12,
    );
    assert!(performance(&resolution).succeeded);
    assert!(fallback_line(&resolution).starts_with("Sigrun sets the label down."));
}

#[test]
fn throwing_a_held_label_at_corin_is_a_freeform_throw_not_a_hand_over() {
    let mut situation = market_street();
    hold(&mut situation, "label");
    let resolution = run(
        &situation,
        &plan(
            ActKind::Throw,
            Some(target(&situation, "label")),
            Difficulty::Easy,
        ),
        "I throw the folded label at Corin.",
        12,
    );
    assert_eq!(resolution.act, Some(ActKind::Throw));
    assert!(performance(&resolution).succeeded);
    assert!(fallback_line(&resolution).starts_with("Sigrun throws the label."));
}

#[test]
fn every_refusal_is_an_in_world_sentence_with_no_machine_wording() {
    let situation = market_street();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Throw,
            Some(target(&situation, "counter")),
            Difficulty::None,
        ),
        "I throw the counter.",
        12,
    );
    let refusals = [
        Refusal::MissingTarget,
        Refusal::Fixed,
        Refusal::NotHeld { holder: None },
        Refusal::NotHeld {
            holder: Some("Corin".to_owned()),
        },
        Refusal::NoItem,
        Refusal::NoRecipient,
        Refusal::UnknownSpell {
            spell: "Light".to_owned(),
        },
        Refusal::NoSpell,
    ];
    for refusal in refusals {
        let line = refused_line("Sigrun", &resolution, &refusal);
        assert!(line.ends_with('.'), "{line}");
        for machine in [
            "Refusal",
            "MissingTarget",
            "UnknownSpell",
            "NotHeld",
            "{",
            "refused",
            "target",
            "is refused",
        ] {
            assert!(!line.contains(machine), "{machine} in {line}");
        }
    }
}

#[test]
fn writing_a_note_on_the_counter_just_happens_and_everyone_present_notices_it() {
    let situation = market_street();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Write,
            Some(target(&situation, "counter")),
            Difficulty::None,
        ),
        "I write a note thanking Marek and leave it on the counter.",
        12,
    );
    assert!(matches!(resolution.verdict, Verdict::Performed(_)));
    assert_eq!(
        fallback_line(&resolution),
        "Sigrun writes a note and leaves it on the counter. Marek, Corin and Brakka notice it."
    );
}

#[test]
fn a_note_with_no_named_thing_is_left_in_the_room_without_any_target_needed() {
    let situation = market_street();
    let resolution = run(
        &situation,
        &plan(ActKind::Write, None, Difficulty::None),
        "I write a note thanking Marek.",
        12,
    );
    assert!(matches!(resolution.verdict, Verdict::Performed(_)));
    assert_eq!(
        fallback_line(&resolution),
        "Sigrun writes a note and leaves it in the room. Marek, Corin and Brakka notice it."
    );
}

#[test]
fn a_refused_spell_reaches_the_storyteller_as_the_in_world_sentence() {
    let situation = market_street();
    let resolution = run(
        &situation,
        &FreeformPlan {
            spell: Some("Light".to_owned()),
            ..plan(ActKind::Cast, None, Difficulty::None)
        },
        "I cast Light at the candle.",
        12,
    );
    let facts = narration_facts(&resolution, "I cast Light at the candle.");
    assert_eq!(facts["outcome"], "refused");
    assert_eq!(facts["refusal"], "Sigrun doesn't know the Light spell.");
}

#[test]
fn listening_to_the_street_outside_gives_the_rooms_own_sounds_even_when_the_check_would_fail() {
    let situation = market_street();
    let plan = FreeformPlan {
        sense: Some(Sense::Sound),
        ..plan(
            ActKind::Sense,
            Some(target(&situation, "place")),
            Difficulty::Hard,
        )
    };
    // Die 2 + 3 = 5 misses DC 20, yet the room's own sounds are heard: a check gates only hidden
    // detail about a thing.
    let resolution = run(
        &situation,
        &plan,
        "I stop and listen to the street outside.",
        2,
    );
    assert!(performance(&resolution).check.is_none());
    assert_eq!(
        fallback_line(&resolution),
        "Sigrun listens to the room. Carts rattle past on the market street, and a gull calls over the shop door."
    );
}

#[test]
fn looking_at_herself_in_the_fitting_mirror_describes_her_own_appearance_from_the_sheet() {
    let situation = market_street();
    let appearance = situation
        .hero
        .appearance
        .clone()
        .expect("the sheet records an appearance");
    let plan = FreeformPlan {
        sense: Some(Sense::Sight),
        ..plan(
            ActKind::Sense,
            Some(target(&situation, "Sigrun")),
            Difficulty::Easy,
        )
    };
    // Die 1 would fail any check: a look at oneself needs none.
    let resolution = run(
        &situation,
        &plan,
        "I look at myself in the fitting mirror.",
        1,
    );
    assert!(performance(&resolution).check.is_none());
    assert_eq!(
        fallback_line(&resolution),
        format!("Sigrun looks at herself. {appearance}")
    );
    assert_eq!(
        narration_facts(&resolution, "I look at myself in the fitting mirror.")["revealed"],
        appearance.as_str()
    );
}

#[test]
fn a_look_at_oneself_that_finds_nothing_uses_the_sheets_reflexive_never_themself() {
    let mut situation = market_street();
    situation.hero.appearance = None;
    situation.hero.pronouns = Some(Pronouns {
        subject: "they",
        object: "them",
        possessive: "their",
        reflexive: "themselves",
    });
    let plan = FreeformPlan {
        sense: Some(Sense::Sight),
        ..plan(
            ActKind::Sense,
            Some(target(&situation, "Sigrun")),
            Difficulty::None,
        )
    };
    let resolution = run(&situation, &plan, "I look at myself.", 12);
    assert_eq!(
        fallback_line(&resolution),
        "Sigrun looks at themselves, but nothing more stands out."
    );
}

#[test]
fn a_hero_whose_sheet_records_no_pronouns_is_named_and_never_given_themself() {
    let mut situation = market_street();
    situation.hero.appearance = None;
    situation.hero.pronouns = None;
    let plan = FreeformPlan {
        sense: Some(Sense::Sight),
        ..plan(
            ActKind::Sense,
            Some(target(&situation, "Sigrun")),
            Difficulty::None,
        )
    };
    let resolution = run(&situation, &plan, "I look at myself.", 12);
    let line = fallback_line(&resolution);
    assert_eq!(line, "Sigrun looks at Sigrun, but nothing more stands out.");
    assert!(!line.contains("themself"));
}

#[test]
fn ordinary_travel_takes_no_check_even_when_jev_calls_it_easy() {
    // SRD 5.2.1: walking down a street is not an uncertain outcome, so nothing is rolled.
    let situation = market_street();
    for difficulty in [Difficulty::Easy, Difficulty::Hard] {
        let resolution = run(
            &situation,
            &plan(ActKind::Move, None, difficulty),
            "We walk on down the market street.",
            1,
        );
        match resolution.verdict {
            Verdict::Performed(performance) => {
                assert!(performance.check.is_none(), "{difficulty:?}");
                assert!(performance.succeeded);
            }
            other => panic!("a walk is performed, not {other:?}"),
        }
    }
}

#[test]
fn walking_up_to_the_counter_says_where_sigrun_is_now_and_what_is_there() {
    let situation = market_street();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Move,
            Some(target(&situation, "counter")),
            Difficulty::None,
        ),
        "I walk up to the counter.",
        12,
    );
    assert_eq!(
        fallback_line(&resolution),
        "Sigrun walks up to the counter."
    );
    let facts = narration_facts(&resolution, "I walk up to the counter.");
    assert_eq!(facts["destination"]["name"], "counter");
    assert_eq!(
        facts["destination"]["scene_fact"],
        "A long pine counter, scarred by use."
    );
}

#[test]
fn the_fact_bundle_carries_the_heroes_own_pronouns_when_the_hero_is_the_target() {
    let situation = market_street();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Express,
            Some(target(&situation, "Sigrun")),
            Difficulty::None,
        ),
        "I laugh at myself.",
        12,
    );
    let facts = narration_facts(&resolution, "I laugh at myself.");
    assert_eq!(facts["hero_pronouns"]["subject"], "she");
    assert_eq!(facts["hero_pronouns"]["reflexive"], "herself");
    assert_eq!(fallback_line(&resolution), "Sigrun reacts out loud.");
}

#[test]
fn a_present_merchants_stock_is_listed_as_portable_things_and_an_absent_merchants_is_not() {
    let model: SceneModel = serde_json::from_slice(include_bytes!(
        "../../../../content/merchant-shop/scene-model.json"
    ))
    .expect("the authored merchant shop model deserializes");
    let marek = PersonFact {
        id: "merchant".to_owned(),
        name: "Marek".to_owned(),
        role: rpc::SceneRole::Contact,
        passive_perception: 11,
    };
    let mut situation = shop();
    situation.people = vec![marek];
    situation
        .things
        .extend(present_stock(&model, &situation.people));
    let options = situation.options();
    let cloak = options
        .iter()
        .find(|target| target.id == "item-warm-cloak-wool")
        .expect("the present merchant's warm wool cloak is a listed target");
    assert_eq!(cloak.kind, TargetKind::Thing);
    assert_eq!(cloak.name, "warm wool cloak");
    let thing = situation
        .thing("item-warm-cloak-wool")
        .expect("the stock is a thing in the situation");
    assert!(thing.portable);
    assert!(!thing.held_by_hero);
    assert!(thing.holder_name.is_none());
    assert_eq!(thing.owner_id.as_deref(), Some("merchant"));

    // The shop's other people are present but Marek is not, so none of his stock is listed.
    let without_merchant = shop().people;
    assert!(present_stock(&model, &without_merchant).is_empty());
}

#[test]
fn hs56_situation_uses_canonical_stacks_without_virtual_pack_children_or_private_peer_gear() {
    let current = crate::gameplay::beat_script_tests::prepared_story();
    let state = current.state();
    let hero = state.characters[0].entity;
    let scene = beat_script_runtime::scene(state).unwrap();
    let situation = Situation::read(state, hero, &scene, &[]).unwrap();
    let custody = journey::living_scene::source_custody(state).unwrap();
    let own = journey::name(state, hero).unwrap();
    for thing in custody
        .iter()
        .filter(|thing| thing.holder_id.as_deref() == Some(own.as_str()))
    {
        let projected = situation
            .things
            .iter()
            .find(|target| target.id == thing.id)
            .unwrap();
        assert!(projected.held_by_hero);
        assert_eq!(projected.detail, thing.detail);
    }
    assert!(
        situation
            .things
            .iter()
            .all(|thing| !thing.id.starts_with("pack-") && !thing.id.starts_with("kit-"))
    );
    for thing in custody
        .iter()
        .filter(|thing| thing.holder_id.as_deref() != Some(own.as_str()) && !thing.publicly_visible)
    {
        assert!(!situation.things.iter().any(|target| target.id == thing.id));
    }
}

#[test]
fn wearing_a_cloak_the_owner_permitted_this_hero_to_try_on_is_no_offence() {
    let mut situation = market_street();
    situation.things.push(ThingFact {
        owner_id: Some("marek".to_owned()),
        ..thing_fact(
            "item-warm-cloak-wool",
            "warm wool cloak",
            "A warm wool cloak for sale from Marek.",
            true,
            false,
        )
    });
    let wear = plan(
        ActKind::Wear,
        Some(target(&situation, "item-warm-cloak-wool")),
        Difficulty::None,
    );
    // Without consent the owner takes offence, as before.
    let unconsented = run(
        &situation,
        &wear,
        "I step behind the curtain and try on the warm wool cloak.",
        12,
    );
    assert_eq!(unconsented.consequences.len(), 1);
    assert_eq!(unconsented.consequences[0].kind, ConsequenceKind::Offence);
    // With this hero's consent for this exact item, wearing it is permitted.
    let consented = adjudicate_with_try_on_consents(
        &situation,
        &wear,
        12,
        fair(3),
        &["item-warm-cloak-wool".to_owned()],
    );
    assert!(matches!(consented.verdict, Verdict::Performed(_)));
    assert!(consented.consequences.is_empty());
    assert_eq!(
        fallback_line(&consented),
        "Sigrun puts on the warm wool cloak."
    );
    // Consent for a different item does not cover the cloak.
    let other_item = adjudicate_with_try_on_consents(
        &situation,
        &wear,
        12,
        fair(3),
        &["item-gloves".to_owned()],
    );
    assert_eq!(other_item.consequences.len(), 1);
    assert_eq!(other_item.consequences[0].kind, ConsequenceKind::Offence);
}

#[test]
fn a_greeting_with_no_person_chosen_says_no_one_is_here_to_greet() {
    let situation = shop();
    let resolution = run(
        &situation,
        &plan(ActKind::ContactFriendly, None, Difficulty::None),
        "I wave to the nearest stallholder.",
        12,
    );
    assert_eq!(resolution.verdict, Verdict::Refused(Refusal::MissingTarget));
    assert_eq!(
        fallback_line(&resolution),
        "Marek finds no one here to greet."
    );
}

#[tokio::test(flavor = "current_thread")]
async fn jev_target_choices_describe_people_by_role_and_the_hero_by_reflection() {
    use crate::gameplay::approach_model::tests::{answer, model};

    let situation = market_street();
    let self_choice = situation
        .options()
        .iter()
        .position(|option| option.id == "Sigrun")
        .expect("the hero themself is a listed target")
        + 1;
    let endpoint = FakeEndpoint::start(vec![
        answer("o1"), // Sense.
        answer("o1"), // Sight.
        answer(&format!("o{self_choice}")),
        answer("o1"), // No difficulty.
    ]);
    let judged = super::plan(
        &model(&endpoint),
        "I study my reflection in the fitting mirror.",
        &situation,
    )
    .await
    .expect("Jev judges the typed line");
    assert_eq!(judged.act, ActKind::Sense);
    assert_eq!(
        judged.target.as_ref().map(|target| target.id.as_str()),
        Some("Sigrun")
    );
    let bodies = endpoint.bodies();
    let target_criteria = bodies[2]["questions"]["q1"]["criteria"].to_string();
    assert!(target_criteria.contains("reflection"), "{target_criteria}");
    assert!(
        target_criteria.contains("a contact in this place"),
        "{target_criteria}"
    );
    assert!(bodies[0].to_string().contains("hero_appearance"));
}

#[tokio::test(flavor = "current_thread")]
async fn hs41_freeform_action_six_saved_build_inventories_resolve_my_sword_by_model_choice() {
    use crate::gameplay::beat_script_tests::{build, input};
    let mut tuning: Value = serde_json::from_slice(include_bytes!(
        "../../../../content/harbor-journey/tuning.json"
    ))
    .unwrap();
    tuning["tactical"] = json!({"enemies":[{"srd_id":"bandit","count":3,"ammunition":20}]});
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        &serde_json::to_vec(&tuning).unwrap(),
    )
    .unwrap();
    let names = ["Thorin", "Kazrak", "Orla", "Pip", "Vesper", "Sylvara"];
    let packages = [
        "melee-stalwart",
        "melee-stalwart",
        "melee-stalwart",
        "melee-vanguard",
        "melee-vanguard",
        "archer",
    ];
    let situations = pack::with_test_pack(loaded, || {
        let mut current = journey::initial().unwrap();
        for op in 1..=6 {
            current = journey::stage_join(
                &current,
                &input(
                    &current,
                    journey::bootstrap_member().unwrap(),
                    op,
                    "join-room",
                    vec![],
                ),
            )
            .unwrap();
        }
        let members = current
            .state()
            .members
            .iter()
            .filter(|member| member.member != journey::bootstrap_member().unwrap())
            .map(|member| member.member)
            .collect::<Vec<_>>();
        for (index, (name, package)) in names.iter().zip(packages).enumerate() {
            let mut choices = build(name);
            choices.retain(|(key, _)| {
                !["array", "equipment", "style-masteries"].contains(&key.as_str())
            });
            choices.push((
                super::super::model::label("build-package").unwrap(),
                super::super::model::label(package).unwrap(),
            ));
            current = journey::stage_with_supplier(
                &current,
                &input(
                    &current,
                    members[index],
                    10 + index as u8,
                    "create-character",
                    choices,
                ),
                &mut |_| panic!("creation is certain"),
            )
            .unwrap();
        }
        current = journey::stage_with_supplier(
            &current,
            &input(&current, members[0], 30, "begin-story", vec![]),
            &mut |_| panic!("opening is certain"),
        )
        .unwrap();
        let scene = beat_script_runtime::scene(current.state()).unwrap();
        members
            .iter()
            .map(|member| {
                let hero = journey::player_entity(*member, &current).unwrap();
                Situation::read(
                    current.state(),
                    hero,
                    &scene,
                    &super::super::wire::scene_cast(&scene),
                )
                .unwrap()
            })
            .collect::<Vec<_>>()
    });
    let draw = ACT_KINDS
        .iter()
        .position(|(act, _, _)| *act == ActKind::Draw)
        .unwrap()
        + 1;
    for (index, mut situation) in situations.into_iter().enumerate() {
        let weapon = if packages[index] == "archer" {
            "shortsword"
        } else {
            "greatsword"
        };
        let choices = situation
            .options()
            .into_iter()
            .filter(|target| {
                situation
                    .thing(&target.id)
                    .is_some_and(|thing| thing.held_by_hero && weapon_name(&thing.name))
            })
            .collect::<Vec<_>>();
        let selected = choices
            .iter()
            .position(|target| target.name == weapon)
            .unwrap()
            + 1;
        situation.hero.pronouns = market_street().hero.pronouns;
        let endpoint = FakeEndpoint::start(vec![
            answer(&format!("o{draw}")),
            answer(&format!("o{selected}")),
            answer("o1"),
            answer("o1"),
        ]);
        let model = crate::gameplay::approach_model::tests::model(&endpoint);
        let judged = super::plan(&model, "I draw my sword.", &situation)
            .await
            .unwrap();
        assert_eq!(
            judged.target.as_ref().unwrap().name,
            weapon,
            "{} actual inventory",
            names[index]
        );
        let resolved = adjudicate(&situation, &judged, 1, |_, _, _| {
            panic!("drawing requires no check")
        });
        assert!(matches!(
            resolved.verdict,
            Verdict::Performed(Performance {
                succeeded: true,
                check: None,
                ..
            })
        ));
        assert!(fallback_line(&resolved).contains(&format!("draws her {weapon}")));
        assert_eq!(narration_facts(&resolved, "")["main_weapon_instead"], false);
    }
}

#[test]
fn hs41_freeform_action_a_refused_weapon_names_actual_carried_weapons() {
    let mut situation = shop();
    situation.things.retain(|thing| thing.name != "greatsword");
    let resolution = run(
        &situation,
        &plan(ActKind::Draw, None, Difficulty::None),
        "I draw my sword.",
        1,
    );
    assert_eq!(resolution.verdict, Verdict::Refused(Refusal::MissingTarget));
    assert_eq!(
        fallback_line(&resolution),
        "Marek has no such weapon; the carried weapons are javelin."
    );
}

#[test]
fn hs41_freeform_action_large_current_inventory_stacks_do_not_hide_carried_weapons() {
    let current = crate::gameplay::beat_script_tests::prepared_story();
    let member = MemberId::from_bytes(&[0x61; 16]).unwrap();
    let hero = journey::player_entity(member, &current).unwrap();
    let mut saved = current.state().clone();
    let gold = saved
        .inventory
        .iter_mut()
        .find(|item| item.owner == hero && item.origin.entry.as_str() == "gold-piece")
        .unwrap();
    gold.quantity = 300;
    let javelins = saved
        .inventory
        .iter_mut()
        .find(|item| item.owner == hero && item.origin.entry.as_str() == "javelin")
        .unwrap();
    javelins.quantity = 500;
    let scene = beat_script_runtime::scene(&saved).unwrap();
    let situation = Situation::read(
        &saved,
        hero,
        &scene,
        &super::super::wire::scene_cast(&scene),
    )
    .unwrap();
    assert_eq!(situation.coin_gp, 300);
    assert!(
        situation
            .things
            .iter()
            .any(|thing| thing.held_by_hero && thing.name == "greatsword")
    );
    assert!(
        situation
            .things
            .iter()
            .any(|thing| thing.held_by_hero && thing.name == "javelin")
    );
}

#[test]
fn hs41_freeform_action_source_weapon_display_case_does_not_prevent_drawing() {
    let mut situation = shop();
    situation
        .things
        .iter_mut()
        .find(|thing| thing.id == "pack-greatsword")
        .unwrap()
        .name = "Greatsword".into();
    let resolution = run(
        &situation,
        &plan(
            ActKind::Draw,
            Some(target(&situation, "pack-greatsword")),
            Difficulty::Hard,
        ),
        "I draw my sword.",
        1,
    );
    assert!(matches!(
        resolution.verdict,
        Verdict::Performed(Performance {
            succeeded: true,
            check: None,
            ..
        })
    ));
    assert!(fallback_line(&resolution).contains("draws Marek's Greatsword"));
}

#[tokio::test(flavor = "current_thread")]
async fn hs41_freeform_action_model_can_ready_a_main_weapon_only_when_fairly_judged() {
    let mut situation = shop();
    situation.hero.name = "Sylvara".into();
    situation.hero.pronouns = market_street().hero.pronouns;
    situation.things = vec![thing_fact(
        "pack-longbow",
        "longbow",
        "Her only carried weapon is a longbow.",
        true,
        true,
    )];
    let draw = ACT_KINDS
        .iter()
        .position(|(act, _, _)| *act == ActKind::Draw)
        .unwrap()
        + 1;
    let endpoint = FakeEndpoint::start(vec![
        answer(&format!("o{draw}")),
        answer("o1"),
        answer("o2"),
        answer("o1"),
    ]);
    let model = crate::gameplay::approach_model::tests::model(&endpoint);
    let judged = super::plan(&model, "I draw my sword.", &situation)
        .await
        .unwrap();
    assert_eq!(
        judged.target.as_ref().unwrap().kind,
        TargetKind::MainWeaponInstead
    );
    let resolution = adjudicate(&situation, &judged, 1, |_, _, _| {
        panic!("readying never rolls")
    });
    assert!(
        fallback_line(&resolution)
            .contains("Sylvara has no such weapon; Sylvara readies her longbow instead.")
    );
    let bodies = endpoint.bodies();
    assert!(bodies[2].to_string().contains("carried_weapons"));
    assert_eq!(
        narration_facts(&resolution, "")["main_weapon_instead"],
        true
    );
}
