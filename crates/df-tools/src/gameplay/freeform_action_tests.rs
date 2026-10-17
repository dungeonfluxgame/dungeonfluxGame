use super::*;
use crate::gameplay::startup_configuration::ApproachSettings;
use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply, cache_directory, teller};
use std::time::Duration;

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
        "There's nothing like that here to light."
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
        Some("Your purse holds 6 gold pieces.")
    );
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
