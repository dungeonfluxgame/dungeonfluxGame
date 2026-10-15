//! Pattern campaign overlays and audience-safe continuity projections.
use super::super::{
    pack::{self, SceneKey},
    startup_configuration::content_rating,
};
use super::campaign::{
    Audience, CampaignError, CampaignFrame, CampaignRecord, ChapterPlan, StorySoFar,
};
use super::campaign_tables::CampaignTables;
use super::{GenerateOptions, GeneratedAdventure, generate};
use std::collections::BTreeMap;

pub(super) fn placeholder_context() -> BTreeMap<String, String> {
    [
        "artifact.name",
        "artifact.short",
        "campaign.title",
        "chapter.label",
        "faction.a",
        "faction.b",
        "home.name",
        "n",
        "region.name",
        "region.short",
        "site.clue",
        "site.stronghold",
        "v.scheme",
        "v.sig",
        "v.title",
        "v.weak",
        "v.weak_clue_0",
        "v.weak_clue_1",
        "villain.name",
    ]
    .into_iter()
    .map(|s| (s.into(), "validated".into()))
    .collect()
}
fn context(frame: &CampaignFrame, public: bool) -> Result<BTreeMap<String, String>, CampaignError> {
    let site = |role: &str| {
        frame
            .sites
            .iter()
            .find(|s| s.role == role)
            .map(|s| s.name.clone())
            .ok_or_else(|| CampaignError::Missing(role.into()))
    };
    let villain = if public {
        format!("the {}", frame.villain.title)
    } else {
        frame.villain.name.clone()
    };
    let faction = |n: usize| {
        frame
            .factions
            .get(n)
            .map(|f| f.faction.name.clone())
            .ok_or_else(|| CampaignError::Missing("faction".into()))
    };
    Ok(BTreeMap::from([
        ("artifact.name".into(), frame.artifact.name.clone()),
        ("artifact.short".into(), frame.artifact.short.clone()),
        ("campaign.title".into(), frame.title.clone()),
        ("region.name".into(), frame.region.clone()),
        ("region.short".into(), frame.region_short.clone()),
        ("home.name".into(), site("home-base")?),
        ("site.clue".into(), site("first-clue")?),
        ("site.stronghold".into(), site("stronghold")?),
        ("villain.name".into(), villain),
        ("v.title".into(), frame.villain.title.clone()),
        ("v.scheme".into(), frame.villain.scheme.clone()),
        ("v.sig".into(), frame.villain.signature.clone()),
        ("v.weak".into(), frame.villain.weakness.text.clone()),
        (
            "v.weak_clue_0".into(),
            frame.villain.weakness.clues[0].clone(),
        ),
        (
            "v.weak_clue_1".into(),
            frame.villain.weakness.clues[1].clone(),
        ),
        ("faction.a".into(), faction(0)?),
        ("faction.b".into(), faction(1)?),
    ]))
}
pub(super) fn render(frame: &CampaignFrame, text: &str, dm: bool) -> Result<String, CampaignError> {
    Ok(super::render::render(text, &context(frame, !dm)?)?)
}
pub(super) fn public_text(frame: &CampaignFrame, text: &str) -> String {
    text.replace(&frame.villain.name, &format!("the {}", frame.villain.title))
}
pub(super) fn resolve_frame(frame: &mut CampaignFrame) -> Result<(), CampaignError> {
    frame.villain.scheme = render(frame, &frame.villain.scheme, true)?;
    frame.title = render(frame, &frame.title, true)?;
    frame.premise.titles = frame
        .premise
        .titles
        .iter()
        .map(|t| render(frame, t, true))
        .collect::<Result<_, _>>()?;
    frame.clock = frame
        .clock
        .iter()
        .map(|text| render(frame, text, true))
        .collect::<Result<_, _>>()?;
    frame.twists = frame
        .twists
        .clone()
        .map(|mut twist| {
            twist.reveal = render(frame, &twist.reveal, true)?;
            twist.hints = twist
                .hints
                .iter()
                .map(|t| render(frame, t, true))
                .collect::<Result<Vec<_>, _>>()?
                .try_into()
                .map_err(|_| CampaignError::Invalid("twist hints".into()))?;
            Ok(twist)
        })
        .into_iter()
        .collect::<Result<Vec<_>, CampaignError>>()?
        .try_into()
        .map_err(|_| CampaignError::Invalid("twists".into()))?;
    frame.milestones = frame
        .milestones
        .iter()
        .map(|t| render(frame, t, true))
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| CampaignError::Invalid("milestones".into()))?;
    Ok(())
}
pub(super) fn story_so_far(record: &CampaignRecord) -> StorySoFar {
    let f = &record.frame;
    let mut private_facts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for fact in &record.facts {
        if let Audience::Private(hero) = &fact.audience {
            private_facts
                .entry(hero.clone())
                .or_default()
                .push(fact.text.clone());
        }
    }
    let public_facts = record
        .facts
        .iter()
        .rev()
        .filter(|fact| fact.audience == Audience::Public && fact.revealed)
        .take(6)
        .map(|fact| public_text(f, &fact.text))
        .collect();
    let chapters = record
        .chapters
        .iter()
        .rev()
        .filter_map(|c| {
            c.outcome
                .as_ref()
                .map(|o| format!("{}: {}", c.plan.title, o.result.phrase()))
        })
        .take(3)
        .collect();
    let latest_sign = f
        .clock
        .get(usize::from(record.clock_position.saturating_sub(1)))
        .map(|t| public_text(f, t))
        .unwrap_or_else(|| "The recorded clock has no sign.".into());
    StorySoFar {
        campaign: f.title.clone(),
        region: f.region.clone(),
        tone: f.tone.clone(),
        teaser: f
            .premise
            .pitch
            .split_inclusive('.')
            .next()
            .filter(|t| t.chars().count() <= 160)
            .unwrap_or(&f.premise.archetype)
            .to_owned(),
        chapters,
        public_facts,
        private_facts,
        open_leads: Vec::new(),
        latest_sign,
        npcs: record.npcs.clone(),
        hint_only: std::iter::once(f.villain.name.clone())
            .chain(
                f.twists
                    .iter()
                    .filter(|t| {
                        !record
                            .facts
                            .iter()
                            .any(|fact| fact.text == public_text(f, &t.reveal))
                    })
                    .map(|t| t.reveal.clone()),
            )
            .collect(),
    }
}
fn check(text: &str, bound: usize, slot: &str) -> Result<(), CampaignError> {
    if text.chars().count() > bound || text.contains(['{', '}']) {
        return Err(CampaignError::Invalid(format!(
            "{slot} exceeds {bound} characters or contains a placeholder"
        )));
    }
    Ok(())
}
fn append(text: &mut String, addition: &str, bound: usize) -> bool {
    let extra = usize::from(!text.is_empty()) + addition.chars().count();
    if text.chars().count() + extra > bound {
        return false;
    }
    if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(addition);
    true
}
/// Generate through the chapter generator, then apply truthful bounded campaign prose.
pub(crate) fn generate_chapter(
    tables: &CampaignTables,
    frame: &CampaignFrame,
    plan: &mut ChapterPlan,
) -> Result<GeneratedAdventure, CampaignError> {
    plan.forced.insert(
        super::beat_tables::CHAPTER_TYPE.into(),
        plan.chapter_type.clone(),
    );
    let mut generated = generate(
        &tables.chapter,
        plan.seed,
        GenerateOptions {
            rules_mode: frame.rules_mode,
            rating: content_rating(),
        },
        &plan.forced,
    )?;
    if plan.chapter_type == "lieutenant" || plan.returning_lieutenant.is_some() {
        let id = plan.returning_lieutenant.as_deref();
        let ordinal = usize::from(
            frame
                .outline
                .iter()
                .any(|s| s.number == plan.number && s.role == "lieutenant-2"),
        );
        let lieutenant = if let Some(id) = id {
            frame.villain.lieutenants.iter().find(|l| l.id == id)
        } else {
            frame.villain.lieutenants.get(ordinal)
        }
        .ok_or_else(|| CampaignError::Missing("lieutenant".into()))?;
        let old = generated.pack.bandit.name.clone();
        let persistent = super::render::capitalize(
            lieutenant
                .title
                .strip_prefix("the ")
                .unwrap_or(&lieutenant.title),
        );
        generated.pack.bandit.name = persistent.clone();
        for scene in &mut generated.pack.scenes {
            scene.narration = scene.narration.replace(&old, &persistent);
            scene.dialogue = scene.dialogue.replace(&old, &persistent);
            scene.speaker = scene.speaker.replace(&old, &persistent);
            if let Some(rested) = &mut scene.narration_rested {
                *rested = rested.replace(&old, &persistent);
            }
        }
        for sheet in &mut generated.fact_sheet {
            sheet.baseline = sheet.baseline.replace(&old, &persistent);
        }
        generated
            .facts
            .insert("ant.full".into(), persistent.clone());
        generated
            .facts
            .insert("ant.short".into(), persistent.clone());
        generated.rolls.push(super::Roll {
            n: generated.rolls.len() + 1,
            table: "campaign.antagonist.name".into(),
            die: 4,
            natural: 0,
            entry_id: persistent,
            status: super::RollStatus::Forced,
        });
    }
    let kind = tables
        .types
        .tables
        .chapter_types
        .entries
        .iter()
        .find(|t| t.id == plan.chapter_type)
        .ok_or_else(|| CampaignError::Missing(plan.chapter_type.clone()))?;
    // Labels are two-variant authored patterns; use a recorded die, not hidden randomness.
    let mut dice = super::rng::Dice::new(plan.seed ^ 0xc01a);
    let natural = dice.roll(4);
    let index = usize::from((natural - 1) / 2);
    plan.rolls.push(super::Roll {
        n: plan.rolls.len() + 1,
        table: "campaign.approach_labels".into(),
        die: 4,
        natural,
        entry_id: format!("variant {}", index + 1),
        status: super::RollStatus::Ok,
    });
    generated.pack.offer_labels.ask_courier = kind
        .asks
        .get(index)
        .ok_or_else(|| CampaignError::Missing("ask label".into()))?
        .clone();
    generated.pack.offer_labels.escort_courier = kind
        .helps
        .get(index)
        .ok_or_else(|| CampaignError::Missing("help label".into()))?
        .clone();
    generated.pack.offer_labels.defend_courier = kind
        .stands
        .get(index)
        .ok_or_else(|| CampaignError::Missing("stand label".into()))?
        .clone();
    let private = plan
        .leads
        .iter()
        .find(|c| c.vehicle == "private")
        .ok_or_else(|| CampaignError::Invalid("private lead".into()))?;
    generated.pack.courier_clue = whispered_clue(
        generated
            .facts
            .get("patron.first")
            .ok_or_else(|| CampaignError::Missing("patron.first".into()))?,
        generated
            .facts
            .get("contact.full")
            .ok_or_else(|| CampaignError::Missing("contact.full".into()))?,
        generated
            .facts
            .get("dest.name")
            .ok_or_else(|| CampaignError::Missing("dest.name".into()))?,
        &private.text,
    );
    check(&generated.pack.courier_clue, 280, "private clue")?;
    let mut room_narration = if let Some(previous) = plan.recap.chapters.first() {
        format!("Previously, {previous}. Chapter {}.", plan.number)
    } else {
        format!("Chapter {}.", plan.number)
    };
    let mut room_dialogue = if let Some(previous) = plan.recap.chapters.first() {
        format!("Previously: {previous}.")
    } else {
        plan.recap.teaser.clone()
    };
    check(&room_dialogue, 160, "Previously")?;
    let mut opening = format!(
        "You meet {} in {}.",
        plan.patron.first, generated.settings.meeting.name
    );
    let mut opening_dialogue = String::new();
    let mut followup = String::new();
    let mut followup_dialogue = String::new();
    let hook_context = if plan.desperate {
        format!("Time is running out. {}", plan.hook_context)
    } else {
        plan.hook_context.clone()
    };
    if !append(&mut opening, &hook_context, 220)
        && !append(&mut opening_dialogue, &hook_context, 160)
        && !append(&mut followup, &hook_context, 220)
        && !append(&mut followup_dialogue, &hook_context, 160)
    {
        return Err(CampaignError::Invalid(
            "hook continuity exceeds visible public fields".into(),
        ));
    }
    let fate = format!("The fate of {} hangs in the balance.", frame.artifact.name);
    if !append(&mut followup, &fate, 220)
        && !append(&mut opening_dialogue, &fate, 160)
        && !append(&mut followup_dialogue, &fate, 160)
        && !append(&mut opening, &fate, 220)
    {
        return Err(CampaignError::Invalid(
            "artifact continuity exceeds visible public fields".into(),
        ));
    }
    // Lobby hides its dialogue and all but the first narration sentence. Required
    // continuity is emitted in Opening or BOTH roleplay branches, before the fight.
    for rescued in &mut plan.rescued {
        if rescued.vehicle == "rescue-dialogue" && append(&mut opening_dialogue, &rescued.text, 160)
        {
            rescued.vehicle = "rescue-dialogue".into();
        } else if append(&mut opening, &rescued.text, 220) {
            rescued.vehicle = "rescue-opening".into();
        } else if append(&mut opening_dialogue, &rescued.text, 160) {
            rescued.vehicle = "rescue-dialogue".into();
        } else if append(&mut followup, &rescued.text, 220) {
            rescued.vehicle = "rescue-roleplay".into();
        } else if append(&mut followup_dialogue, &rescued.text, 160) {
            rescued.vehicle = "rescue-roleplay-dialogue".into();
        } else {
            return Err(CampaignError::Invalid(
                "deadline rescue exceeds visible public fields".into(),
            ));
        }
    }
    for text in plan
        .twist
        .iter()
        .chain(plan.milestone.iter())
        .chain(plan.hints.iter())
    {
        if !append(&mut opening, text, 220)
            && !append(&mut opening_dialogue, text, 160)
            && !append(&mut followup, text, 220)
            && !append(&mut followup_dialogue, text, 160)
        {
            return Err(CampaignError::Invalid(
                "twist/hint exceeds visible public fields".into(),
            ));
        }
    }
    if plan.number > 1 {
        if !append(&mut room_dialogue, &plan.recap.latest_sign, 160) {
            append(&mut room_narration, &plan.recap.latest_sign, 220);
        }
        if let Some(lead) = plan.recap.open_leads.first()
            && !append(&mut room_dialogue, lead, 160)
        {
            append(&mut room_narration, lead, 220);
        }
    }
    let victory = plan
        .leads
        .iter()
        .find(|c| c.vehicle == "victory")
        .ok_or_else(|| CampaignError::Invalid("victory lead".into()))?;
    let contact = plan
        .leads
        .iter()
        .find(|c| c.vehicle == "contact")
        .ok_or_else(|| CampaignError::Invalid("contact lead".into()))?;
    let mut mechanical = generated.facts.clone();
    mechanical.insert(
        "story".into(),
        format!(
            "The path to {} remains open. {}",
            generated.settings.destination.short, victory.text
        ),
    );
    let defeat_ending = (plan.chapter_type == "finale")
        .then(|| epilogue(frame, plan, false, super::campaign::Approach::Escorted));
    for scene in &mut generated.pack.scenes {
        match scene.key {
            SceneKey::Room => {
                scene.title = frame.title.clone();
                scene.narration = room_narration.clone();
                scene.speaker = if plan.number == 1 {
                    "The tale begins"
                } else {
                    "Previously"
                }
                .into();
                scene.dialogue = room_dialogue.clone();
            }
            SceneKey::Opening => {
                scene.title = plan.title.clone();
                if !opening.is_empty() {
                    scene.narration = opening.clone();
                }
                if !opening_dialogue.is_empty() {
                    scene.dialogue = opening_dialogue.clone();
                }
            }
            SceneKey::Victory => {
                for (key, rested) in [("victory_mech", false), ("victory_rested", true)] {
                    let template = tables
                        .chapter
                        .prose
                        .get(key)
                        .and_then(|p| p.variants.first())
                        .ok_or_else(|| CampaignError::Missing(key.into()))?;
                    let text = super::render::render(&template.text, &mechanical)?;
                    if rested {
                        scene.narration_rested = Some(format!(
                            "The path to {} remains open. {} {text}",
                            generated.settings.destination.short, victory.text
                        ));
                    } else {
                        // The rules sentences are the scene's own `rules_note`; display joins them.
                        scene.narration = format!(
                            "The path to {} remains open. {}",
                            generated.settings.destination.short, victory.text
                        );
                    }
                }
            }
            SceneKey::Defeat if defeat_ending.is_some() => {
                let ending = defeat_ending
                    .as_ref()
                    .ok_or_else(|| CampaignError::Missing("finale ending".into()))?;
                check(ending, 220, "defeat story")?;
                let mut context = mechanical.clone();
                context.insert("story".into(), ending.clone());
                for (key, rested) in [("defeat_mech", false), ("defeat_rested", true)] {
                    let template = tables
                        .chapter
                        .prose
                        .get(key)
                        .and_then(|p| p.variants.first())
                        .ok_or_else(|| CampaignError::Missing(key.into()))?;
                    let text = super::render::render(&template.text, &context)?;
                    if rested {
                        scene.narration_rested = Some(format!("{ending} {text}"));
                    } else {
                        scene.narration = ending.clone();
                    }
                }
            }
            SceneKey::DialogueSeal | SceneKey::DialogueEscort => {
                scene.narration = followup.clone();
                if !followup_dialogue.is_empty() {
                    scene.dialogue = followup_dialogue.clone();
                }
            }
            SceneKey::Inn => {
                if plan.chapter_type == "finale" {
                    let (speaker, dialogue) = epilogue_speech(
                        &plan.contact.first,
                        &frame.artifact.name,
                        &frame.region_short,
                    );
                    scene.speaker = speaker;
                    scene.dialogue = dialogue;
                } else {
                    scene.dialogue = contact.text.clone();
                }
            }
            _ => {}
        }
    }
    script_continuity(&mut generated)?;
    remove_repeated_ending_sentences(&mut generated, None);
    for scene in &generated.pack.scenes {
        check(&scene.title, 32, "scene title")?;
        check(&scene.dialogue, 160, "spoken line")?;
    }
    if generated
        .pack
        .scenes
        .iter()
        .flat_map(|s| [&s.title, &s.narration, &s.dialogue])
        .any(|s| s.contains(&frame.villain.name))
    {
        return Err(CampaignError::Invalid(
            "private villain name in public campaign prose".into(),
        ));
    }

    // The campaign overlays are canonical pattern expression. Replace stale chapter
    // secret/complication authority before the adventure digest is frozen.
    generated
        .facts
        .retain(|key, _| !key.starts_with("secret.") && !key.starts_with("complication."));
    generated
        .facts
        .insert("secret.fact".into(), private.text.clone());
    generated
        .facts
        .insert("campaign.title".into(), frame.title.clone());
    generated
        .facts
        .insert("campaign.artifact".into(), frame.artifact.name.clone());
    let labels = [
        &generated.pack.offer_labels.begin_story,
        &generated.pack.offer_labels.ask_courier,
        &generated.pack.offer_labels.escort_courier,
        &generated.pack.offer_labels.defend_courier,
        &generated.pack.offer_labels.choose_harbor_scene,
        &generated.pack.courier_clue,
    ];
    for (sheet, baseline) in generated.fact_sheet.iter_mut().take(6).zip(labels) {
        sheet.baseline = baseline.clone();
    }
    for (index, scene) in generated.pack.scenes.iter().enumerate() {
        let start = 6 + index * 3;
        let narration = if scene.key == SceneKey::Victory {
            victory.text.clone()
        } else if scene.key == SceneKey::Defeat && defeat_ending.is_some() {
            defeat_ending
                .clone()
                .ok_or_else(|| CampaignError::Missing("finale ending".into()))?
        } else if matches!(scene.key, SceneKey::Combat | SceneKey::Defeat) {
            generated
                .fact_sheet
                .get(start + 1)
                .ok_or_else(|| CampaignError::Invalid("defeat fact sheet".into()))?
                .baseline
                .clone()
        } else {
            scene.narration.clone()
        };
        for (offset, baseline) in [scene.title.clone(), narration, scene.dialogue.clone()]
            .into_iter()
            .enumerate()
        {
            let sheet = generated
                .fact_sheet
                .get_mut(start + offset)
                .ok_or_else(|| CampaignError::Invalid("scene fact sheet".into()))?;
            sheet.baseline = baseline;
            sheet.speaker = scene.speaker.clone();
        }
    }
    for sheet in &mut generated.fact_sheet {
        let private_slot = sheet.slot == "clue";
        sheet.facts = generated
            .facts
            .iter()
            .filter(|(key, _)| private_slot || !key.starts_with("secret."))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        sheet.must_mention = sheet
            .facts
            .values()
            .filter(|value| !value.is_empty() && sheet.baseline.contains(value.as_str()))
            .cloned()
            .collect();
        sheet.must_mention.sort();
        sheet.must_mention.dedup();
        sheet.forbidden = if private_slot {
            vec![frame.villain.name.clone()]
        } else {
            vec![private.text.clone(), frame.villain.name.clone()]
        };
        sheet.tone = frame.tone.clone();
        check(&sheet.baseline, sheet.bounds, "campaign expression")?;
    }
    if plan.chapter_type == "finale" {
        generated.pack.campaign_epilogue = Some(pack::FinaleProse {
            asked: epilogue(frame, plan, true, super::campaign::Approach::Asked),
            escorted: epilogue(frame, plan, true, super::campaign::Approach::Escorted),
            defeat: epilogue(frame, plan, false, super::campaign::Approach::Escorted),
        });
        if let Some(epilogue) = &generated.pack.campaign_epilogue {
            for (slot, text) in [
                ("epilogue_asked", &epilogue.asked),
                ("epilogue_escorted", &epilogue.escorted),
                ("epilogue_defeat", &epilogue.defeat),
            ] {
                check(text, 220, "epilogue")?;
                generated.fact_sheet.push(super::FactSheet {
                    scene: None,
                    beat: "resolution".into(),
                    slot: slot.into(),
                    speaker: String::new(),
                    voice: String::new(),
                    delivery: String::new(),
                    facts: generated
                        .facts
                        .iter()
                        .filter(|(key, _)| !key.starts_with("secret."))
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                    must_mention: vec![frame.artifact.name.clone()],
                    forbidden: vec![frame.villain.name.clone(), private.text.clone()],
                    bounds: 220,
                    baseline: text.clone(),
                    tone: frame.tone.clone(),
                });
            }
        }
    }
    // The chapter generator rolled dossiers before this overlay renamed the antagonist and
    // replaced the secret; roll them again from the facts the party will actually play, naming
    // only what the campaign already makes public.
    generated.dossiers = super::dossier_cast::roll_for(
        &tables.chapter,
        &generated,
        &super::dossier_cast::PublicStory {
            villain: Some(format!("the {}", frame.villain.title)),
            faction: frame.factions.first().map(|f| f.faction.name.clone()),
            extra_secret_terms: vec![private.text.clone(), frame.villain.name.clone()],
        },
    )?;
    let pack = serde_json::to_vec(&generated.pack).map_err(|_| CampaignError::Serialization)?;
    let tuning = serde_json::to_vec(&generated.tuning).map_err(|_| CampaignError::Serialization)?;
    pack::load_from_bytes(&pack, &tuning)
        .map_err(|e| CampaignError::Generate(super::GenerateError::Pack(e)))?;
    Ok(generated)
}

fn whispered_clue(patron: &str, contact: &str, destination: &str, detail: &str) -> String {
    format!("{patron} whispers: “{contact} waits at {destination}. {detail}” Keep it between us.")
}

fn epilogue_speech(speaker: &str, artifact: &str, region: &str) -> (String, String) {
    (
        speaker.to_owned(),
        format!("I'll tell the people of {region} what you risked for {artifact}."),
    )
}

pub(super) fn epilogue(
    frame: &CampaignFrame,
    plan: &ChapterPlan,
    victory: bool,
    approach: super::campaign::Approach,
) -> String {
    if !victory {
        return format!(
            "{} is dragged away as the {} claims {}.",
            plan.patron.first, frame.villain.title, frame.artifact.name
        );
    }
    let strategy = match approach {
        super::campaign::Approach::Asked => format!(
            "With {}, you offer the {} mercy",
            frame.artifact.name, frame.villain.title
        ),
        super::campaign::Approach::Third
        | super::campaign::Approach::Investigated
        | super::campaign::Approach::Continued
        | super::campaign::Approach::Escorted => format!(
            "Your allies secure {} and end the {}'s reign",
            frame.artifact.name, frame.villain.title
        ),
    };
    format!(
        "{strategy}. {}",
        if plan.desperate {
            "The region survives, bearing the cost of its long struggle."
        } else {
            "The region can begin to heal, and your deeds will be remembered."
        }
    )
}

/// Carry current campaign public sources through peaceful variants. Optional looks
/// retain their authored discoveries; required handlers deliver continuity on either path.
pub(super) fn script_continuity(generated: &mut GeneratedAdventure) -> Result<(), CampaignError> {
    let base_scenes = &generated.pack.scenes;
    if let Some(script) = &mut generated.pack.beat_script
        && !script.uses_encounter()
    {
        let roles = super::beat_tables::scene_roles(script)
            .map_err(|error| CampaignError::Invalid(error.to_string()))?;
        for (id, scene) in &mut script.scenes {
            let base = base_scenes
                .iter()
                .find(|base| base.key == scene.key)
                .ok_or_else(|| CampaignError::Missing(format!("script scene {id}")))?;
            use super::beat_tables::SceneRole;
            let role = roles
                .get(id)
                .ok_or_else(|| CampaignError::Missing(format!("script role {id}")))?;
            let source_key = match role {
                SceneRole::Room => SceneKey::Room,
                SceneRole::Opening => SceneKey::Opening,
                SceneRole::Followup => scene.key,
                SceneRole::Observation => scene.key,
                SceneRole::Contact => SceneKey::Inn,
            };
            let carrier = base_scenes
                .iter()
                .find(|base| base.key == source_key)
                .ok_or_else(|| CampaignError::Missing(format!("script continuity {id}")))?;
            match role {
                SceneRole::Room | SceneRole::Opening | SceneRole::Followup => {
                    scene.narration = carrier.narration.clone();
                    scene.dialogue = carrier.dialogue.clone();
                }
                SceneRole::Contact => scene.dialogue = carrier.dialogue.clone(),
                SceneRole::Observation => {}
            }
            scene.speaker = base.speaker.clone();
            scene.asset = base.asset.clone();
            scene.speaker_portrait = base.speaker_portrait.clone();
            scene.cast = base.cast.clone();
            scene.setting = base.setting.clone();
            scene.rules_note.clear();
            scene.narration_rested = None;
        }
        let start = 6 + base_scenes.len() * 3 + script.labels.len();
        for (index, scene) in script.scenes.values().enumerate() {
            for (offset, text) in [
                scene.title.clone(),
                scene.narration.clone(),
                scene.dialogue.clone(),
            ]
            .into_iter()
            .enumerate()
            {
                let sheet = generated
                    .fact_sheet
                    .get_mut(start + index * 3 + offset)
                    .ok_or_else(|| CampaignError::Invalid("script fact sheet".into()))?;
                sheet.baseline = text;
                if sheet.slot == "line" {
                    sheet.speaker = scene.speaker.clone();
                }
            }
        }
    }
    Ok(())
}

/// Removes exact sentences reused by an ending in an earlier scene at a different place.
pub(super) fn remove_repeated_ending_sentences(
    generated: &mut GeneratedAdventure,
    table_text: Option<&GeneratedAdventure>,
) {
    let settings = &generated.settings;
    let Some(script) = &mut generated.pack.beat_script else {
        return;
    };
    let mut earlier: Vec<(SceneKey, String, String)> = Vec::new();
    for beat in &script.beats {
        if beat.kind == super::super::beat_script::BeatKind::Ending {
            if let Some(variant) = &beat.scene_variant
                && let Some(ending) = script.scenes.get_mut(variant)
            {
                let ending_place = settings.scene(ending.key).place_id.clone();
                let prior_text: Vec<&str> = earlier
                    .iter()
                    .filter(|(scene, _, _)| settings.scene(*scene).place_id != ending_place)
                    .flat_map(|(_, narration, dialogue)| [narration.as_str(), dialogue.as_str()])
                    .collect();
                let narration =
                    super::expression::remove_repeated_sentences(&ending.narration, &prior_text);
                if narration.is_empty() {
                    ending.narration = table_text
                        .and_then(|table| {
                            table
                                .pack
                                .beat_script
                                .as_ref()?
                                .scenes
                                .get(variant)
                                .map(|scene| scene.narration.clone())
                        })
                        .unwrap_or_default();
                } else {
                    ending.narration = narration;
                }
                let dialogue =
                    super::expression::remove_repeated_sentences(&ending.dialogue, &prior_text);
                if dialogue.is_empty() {
                    ending.dialogue = table_text
                        .and_then(|table| {
                            table
                                .pack
                                .beat_script
                                .as_ref()?
                                .scenes
                                .get(variant)
                                .map(|scene| scene.dialogue.clone())
                        })
                        .unwrap_or_default();
                } else {
                    ending.dialogue = dialogue;
                }
            }
            continue;
        }
        if let Some(variant) = &beat.scene_variant
            && let Some(scene) = script.scenes.get(variant)
        {
            earlier.push((scene.key, scene.narration.clone(), scene.dialogue.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{epilogue_speech, whispered_clue};

    #[test]
    fn private_clue_quotes_the_whispered_speech() {
        assert_eq!(
            whispered_clue(
                "Jorun",
                "Tomas Greaves",
                "Tin Street Tea House",
                "Ask him plainly for help. There is old kindness between them."
            ),
            "Jorun whispers: “Tomas Greaves waits at Tin Street Tea House. Ask him plainly for help. There is old kindness between them.” Keep it between us."
        );
    }

    #[test]
    fn epilogue_speech_uses_its_speaker_and_campaign_details() {
        let calla = epilogue_speech("Calla", "the Ledger", "Shepherds' Bothy");
        let mara = epilogue_speech("Mara", "the Moon Glass", "Greyhaven");

        assert_eq!(calla.0, "Calla");
        assert_eq!(mara.0, "Mara");
        assert!(calla.1.contains("the Ledger"));
        assert!(calla.1.contains("Shepherds' Bothy"));
        assert_ne!(calla, mara);
    }
}
