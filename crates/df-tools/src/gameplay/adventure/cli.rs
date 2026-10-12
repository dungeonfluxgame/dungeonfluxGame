//! Designer inspection uses the same generator and includes DM-only content by explicit command.
use super::super::{model, startup_configuration};
use super::{ForcedEntries, GenerateOptions, Tables, generate};
use std::{ffi::OsString, path::Path};
pub(crate) fn run(arguments: &[OsString]) -> Result<(), Box<dyn std::error::Error>> {
    if !(1..=2).contains(&arguments.len()) {
        return Err("usage: df-transport-fixture --roll-adventure <tables_dir> [seed]".into());
    }
    let directory = arguments.first().ok_or("tables directory required")?;
    let seed = match arguments.get(1) {
        Some(value) => {
            startup_configuration::adventure_seed(value.to_str().ok_or("seed must be Unicode")?)?
        }
        None => u64::from_le_bytes(model::random::<8>().map_err(|_| "OS seed unavailable")?),
    };
    let tables = Tables::load(Path::new(directory))?;
    let generated = generate(
        &tables,
        seed,
        GenerateOptions {
            rules_mode: startup_configuration::adventure_creature_mode(std::env::var(
                "DF_GAMEPLAY_ADVENTURE_CREATURES",
            ))?,
            rating: startup_configuration::content_rating_setting(std::env::var(
                "DF_GAMEPLAY_CONTENT_RATING",
            ))?,
        },
        &ForcedEntries::new(),
    )?;
    println!(
        "ADVENTURE seed=0x{seed:016x} tables={} generator=splitmix64-patterns-1",
        generated.tables_revision
    );
    println!(
        "Facts: location={} hook={} patron={} stake={} antagonist={} complication={} secret={} destination={} contact={}",
        generated.facts["loc.id"],
        generated.facts["hook.id"],
        generated.facts["patron.full"],
        generated.facts["stake.id"],
        generated.facts["ant.full"],
        generated.facts["complication.id"],
        generated.facts["secret.id"],
        generated.facts["dest.id"],
        generated.facts["contact.full"]
    );
    println!("DM's rolls ({}):", generated.rolls.len());
    for roll in &generated.rolls {
        println!(
            "{:02} {} d{}:{} {} [{:?}]",
            roll.n, roll.table, roll.die, roll.natural, roll.entry_id, roll.status
        );
    }
    let labels = &generated.pack.offer_labels;
    println!(
        "Offers: begin=\"{}\"; ask=\"{}\"; help=\"{}\"; stand=\"{}\"; continue=\"{}\"; destination=\"{}\"",
        labels.begin_story,
        labels.ask_courier,
        labels.escort_courier,
        labels.defend_courier,
        labels.choose_harbor_scene,
        generated
            .pack
            .destination_label
            .as_deref()
            .ok_or("generated destination missing")?
    );
    println!("Private clue: {}", generated.pack.courier_clue);
    let enemy = &generated.tuning.bandit;
    println!(
        "Antagonist: {} ({}) HP {} AC {} attack {:+}; {} {}d{}{:+}; abilities {:?}",
        generated.pack.bandit.name,
        generated
            .pack
            .bandit
            .srd_id
            .as_deref()
            .ok_or("generated SRD id missing")?,
        enemy.hit_points,
        enemy.armor_class,
        enemy.attack_bonus,
        enemy.weapon.item,
        enemy.weapon.damage_dice,
        enemy.weapon.damage_sides,
        enemy.damage_modifier,
        enemy.abilities
    );
    for scene in &generated.pack.scenes {
        println!("\n{:?}: {}\n{}", scene.key, scene.title, scene.narration);
        if let Some(setting) = &scene.setting {
            println!(
                "Setting: {} | short={} | kind={:?} | exterior={} | art={}",
                setting.name, setting.short, setting.kind, setting.exterior, setting.art_key
            );
        }
        if !scene.dialogue.is_empty() {
            println!("{}: \"{}\"", scene.speaker, scene.dialogue);
        }
        if let Some(rested) = &scene.narration_rested {
            println!("After rest: {rested}");
        }
    }
    Ok(())
}

pub(crate) fn campaign(
    arguments: &[OsString],
    simulate: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use super::{
        campaign::{
            Approach, CampaignError, CampaignRecord, ChapterOutcome, ChapterResult, digest,
            generate_frame, next_chapter,
        },
        campaign_render::generate_chapter,
        campaign_tables::CampaignTables,
    };
    if !(1..=2).contains(&arguments.len()) || simulate && arguments.len() != 2 {
        return Err(
            "usage: --roll-campaign <tables_dir> [seed] | --simulate-campaign <tables_dir> <seed>"
                .into(),
        );
    }
    let directory = arguments.first().ok_or("tables directory required")?;
    let seed = match arguments.get(1) {
        Some(s) => {
            startup_configuration::adventure_seed(s.to_str().ok_or("seed must be Unicode")?)?
        }
        None => u64::from_le_bytes(model::random::<8>().map_err(|_| "OS seed unavailable")?),
    };
    let tables = CampaignTables::load(Path::new(directory))?;
    startup_configuration::install_content_rating(startup_configuration::content_rating_setting(
        std::env::var("DF_GAMEPLAY_CONTENT_RATING"),
    )?)?;
    let frame = generate_frame(
        &tables,
        seed,
        GenerateOptions {
            rules_mode: startup_configuration::adventure_creature_mode(std::env::var(
                "DF_GAMEPLAY_ADVENTURE_CREATURES",
            ))?,
            rating: startup_configuration::content_rating_setting(std::env::var(
                "DF_GAMEPLAY_CONTENT_RATING",
            ))?,
        },
    )?;
    print_frame(&frame);
    let mut record = CampaignRecord::new(frame.clone())?;
    loop {
        let mut plan = match next_chapter(&tables, &frame, &record) {
            Ok(plan) => plan,
            Err(CampaignError::Complete) => break,
            Err(error) => return Err(error.into()),
        };
        let generated = generate_chapter(&tables, &frame, &mut plan)?;
        println!(
            "\nCHAPTER {} act={} type={} title=\"{}\" site=\"{}\" seed=0x{:016x} clock={}/7{}",
            plan.number,
            plan.act,
            plan.chapter_type,
            plan.title,
            plan.site,
            plan.seed,
            record.clock_position,
            if plan.desperate { " desperate" } else { "" }
        );
        println!("Hook continuity: {}", plan.hook_context);
        for lead in &plan.leads {
            println!(
                "Lead {} [{}] {}: {}",
                lead.node, lead.vehicle, lead.clue_id, lead.text
            );
        }
        for lead in &plan.rescued {
            println!("Rescued {} [{}]: {}", lead.node, lead.vehicle, lead.text);
        }
        if let Some(twist) = &plan.twist {
            println!("Act reveal: {twist}");
        }
        if let Some(milestone) = &plan.milestone {
            println!("Milestone (story only): {milestone}");
        }
        if !simulate {
            println!("Forced entries: {}", serde_json::to_string(&plan.forced)?);
            println!(
                "Server story so far (private/DM fields included): {}",
                serde_json::to_string(&plan.recap)?
            );
            println!("Private clue: {}", generated.pack.courier_clue);
            let l = &generated.pack.offer_labels;
            println!(
                "Offers: begin=\"{}\"; ask=\"{}\"; help=\"{}\"; stand=\"{}\"; continue=\"{}\"",
                l.begin_story,
                l.ask_courier,
                l.escort_courier,
                l.defend_courier,
                l.choose_harbor_scene
            );
            let enemy = &generated.tuning.bandit;
            println!(
                "Antagonist: {} ({}) HP {} AC {} attack {:+}; {} {}d{}{:+}; abilities {:?}",
                generated.pack.bandit.name,
                generated
                    .pack
                    .bandit
                    .srd_id
                    .as_deref()
                    .ok_or("SRD identity missing")?,
                enemy.hit_points,
                enemy.armor_class,
                enemy.attack_bonus,
                enemy.weapon.item,
                enemy.weapon.damage_dice,
                enemy.weapon.damage_sides,
                enemy.damage_modifier,
                enemy.abilities
            );
            for scene in &generated.pack.scenes {
                println!("\n{:?}: {}\n{}", scene.key, scene.title, scene.narration);
                if let Some(setting) = &scene.setting {
                    println!(
                        "Setting: {} | short={} | kind={:?} | exterior={} | art={}",
                        setting.name,
                        setting.short,
                        setting.kind,
                        setting.exterior,
                        setting.art_key
                    );
                }
                if !scene.dialogue.is_empty() {
                    println!("{}: \"{}\"", scene.speaker, scene.dialogue);
                }
                if let Some(rested) = &scene.narration_rested {
                    println!("After rest: {rested}");
                }
            }
            println!("\nChapter decisions:");
            for roll in plan.rolls.iter().chain(&generated.rolls) {
                println!(
                    "{} d{}:{} {} [{:?}]",
                    roll.table, roll.die, roll.natural, roll.entry_id, roll.status
                );
            }
            break;
        }
        let n = plan.number;
        record.begin(
            plan,
            format!("df_gameplay_demo_sim_ch{n:02}"),
            digest(&generated.documents(true)?.record),
        )?;
        let result = if seed.wrapping_add(u64::from(n)).is_multiple_of(3) {
            ChapterResult::Defeat
        } else {
            ChapterResult::Victory
        };
        let outcome = ChapterOutcome {
            destination_reached: result == ChapterResult::Victory,
            result: result.clone(),
            approach: if n.is_multiple_of(2) {
                Approach::Asked
            } else {
                Approach::Escorted
            },
            member: if n.is_multiple_of(2) {
                "hero-two"
            } else {
                "hero-one"
            }
            .into(),
            rested: n.is_multiple_of(2),
            antagonist_spared: result == ChapterResult::Victory,
            campaign_time: u64::from(n) * 3600,
        };
        record.complete(n, outcome)?;
        let chapter = record.chapters.last().ok_or("completed chapter missing")?;
        println!(
            "Outcome: {:?}; lieutenant fate={:?}; clock={}/7; record={}",
            chapter.outcome,
            chapter.lieutenant_fate,
            record.clock_position,
            record.digest()?
        );
    }
    if simulate {
        println!(
            "CAMPAIGN COMPLETE: {} playable chapters; clock={}/7; {} clue deliveries; record={}",
            record.chapters.len(),
            record.clock_position,
            record.clues.iter().filter(|c| c.delivered).count(),
            record.digest()?
        );
    }
    Ok(())
}
pub(crate) fn print_frame(frame: &super::campaign::CampaignFrame) {
    println!(
        "CAMPAIGN seed=0x{:016x} title=\"{}\" tables={} generator={}",
        frame.seed, frame.title, frame.tables_revision, frame.generator_version
    );
    println!(
        "Premise: {} [{} / {}]\n{}",
        frame.premise.archetype, frame.premise.id, frame.premise.family, frame.premise.pitch
    );
    let v = &frame.villain;
    println!(
        "Villain: {}, the {} — {}\nMotive: {}\nScheme: {}\nMethod: {}\nResources: {}\nWeakness: {}\nWeakness clues: {} | {}\nSignature: {}",
        v.name,
        v.title,
        v.kind.label,
        v.motive,
        v.scheme,
        v.method,
        v.resources,
        v.weakness.text,
        v.weakness.clues[0],
        v.weakness.clues[1],
        v.signature
    );
    for l in &v.lieutenants {
        println!("Lieutenant: {} [{} / {}]", l.title, l.id, l.antagonist);
    }
    println!(
        "Relic: {} — {}; {}; hunger: {}",
        frame.artifact.name, frame.artifact.form, frame.artifact.power, frame.artifact.hunger
    );
    println!("Region: {}", frame.region);
    for site in &frame.sites {
        println!("Site {}: {} [{}]", site.role, site.name, site.location);
    }
    for f in &frame.factions {
        println!(
            "Faction: {} — {}; party={} ({}), villain={} ({})",
            f.faction.name, f.faction.goal, f.party.id, f.party.text, f.villain.id, f.villain.text
        );
    }
    println!(
        "Recurring allies: {} {} ({}) / {} {} ({})",
        frame.patron.first,
        frame.patron.surname,
        frame.patron.archetype,
        frame.contact.first,
        frame.contact.surname,
        frame.contact.archetype
    );
    println!(
        "Themes: {} / {}; tone: {}",
        frame.themes[0], frame.themes[1], frame.tone
    );
    println!("Clock (starts at one):");
    for (n, text) in frame.clock.iter().enumerate() {
        println!("{}. {text}", n + 1);
    }
    for (n, twist) in frame.twists.iter().enumerate() {
        println!(
            "Act {} twist {}: {}; hints: {} | {}",
            n + 1,
            twist.id,
            twist.reveal,
            twist.hints[0],
            twist.hints[1]
        );
    }
    println!("Outline ({} slots):", frame.outline.len());
    for slot in &frame.outline {
        println!(
            "{} act={} role={} type={} pillars={:?} nodes={:?}{}",
            slot.number,
            slot.act,
            slot.role,
            slot.chapter_type,
            slot.pillars,
            slot.nodes,
            if slot.skipped {
                format!(" SKIPPED: needs {}", slot.needs.join(", "))
            } else {
                String::new()
            }
        );
    }
    println!("DM frame rolls ({}):", frame.rolls.len());
    for roll in &frame.rolls {
        println!(
            "{:02} {} d{}:{} {} [{:?}]",
            roll.n, roll.table, roll.die, roll.natural, roll.entry_id, roll.status
        );
    }
}

pub(crate) fn campaign_run(
    arguments: &[OsString],
    status: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use super::{
        campaign_startup::{current_database, frame},
        campaign_store::CampaignStore,
    };
    let root = std::env::current_dir()?;
    if (status && arguments.len() != 1) || (!status && !(2..=3).contains(&arguments.len())) {
        return Err(
            "usage: --campaign-status <id> | --campaign-run <id> <tables_dir> [seed]".into(),
        );
    }
    let id = arguments[0].to_str().ok_or("campaign id must be Unicode")?;
    let record = if status {
        CampaignStore::new(&root, id)?
            .load()?
            .ok_or("campaign does not exist")?
            .record
    } else {
        let seed = match arguments.get(2) {
            Some(value) => Some(startup_configuration::adventure_seed(
                value.to_str().ok_or("seed must be Unicode")?,
            )?),
            None => None,
        };
        frame(
            &root,
            id,
            Path::new(&arguments[1]),
            seed,
            GenerateOptions::default(),
        )?
    };
    if status {
        print_frame(&record.frame);
        println!(
            "Current clock: {}/7 — {}",
            record.clock_position,
            record.frame.clock[usize::from(record.clock_position - 1)]
        );
        for chapter in &record.chapters {
            println!(
                "Chapter {} act={} {} database={} seed=0x{:016x} outcome={:?} lieutenant={:?}",
                chapter.plan.number,
                chapter.plan.act,
                chapter.plan.title,
                chapter.run_database,
                chapter.plan.seed,
                chapter.outcome,
                chapter.lieutenant_fate
            );
        }
    } else {
        let database = current_database(id, &record)?;
        println!("DATABASE={database}");
        println!("Campaign: {}", record.frame.title);
        println!("Premise: {}", record.frame.premise.pitch);
        if let Some(chapter) = record.chapters.last().filter(|c| c.outcome.is_none()) {
            println!(
                "Act {}, chapter {}/{}: {} ({}) — resume",
                chapter.plan.act,
                chapter.plan.number,
                record.frame.outline.len(),
                chapter.plan.title,
                chapter.plan.chapter_type
            );
        } else {
            let tables = super::campaign_tables::CampaignTables::load(Path::new(&arguments[1]))?;
            let plan = super::campaign::next_chapter(&tables, &record.frame, &record)?;
            println!(
                "Act {}, chapter {}/{}: {} ({})",
                plan.act,
                plan.number,
                record.frame.outline.len(),
                plan.title,
                plan.chapter_type
            );
        }
    }
    Ok(())
}
